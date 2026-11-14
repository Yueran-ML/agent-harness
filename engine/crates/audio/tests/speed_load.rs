//! The load of playing at 8 times (T18's review): sixteen tracks of every kind of strip (Dynamics,
//! equalizers, Duckers, sends to a bus of 5.1, a VCA, lanes, clips at speeds; `session::
//! sixteen_tracks`) played at 8 times on the thread of the mix and a device of its own thread,
//! the clips read ahead by readers of their own: nothing short, nothing read late, and the share
//! of a core that the thread of the mix takes, told. Ten seconds; `COLUMBINA_STRESS_SECONDS`
//! sets another length.

mod session;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use columbina_audio::feed::ClipFeed;
use columbina_audio::graph::{Graph, Options, Purpose, Warp};
use columbina_audio::play::device::{self, Feeding, Order};
use columbina_audio::play::engine::{Command, Engine, Run, Session, MOST_BLOCK};
use columbina_audio::play::fetcher::{ring_feed, Fetcher};
use columbina_audio::play::thread::MixThread;
use columbina_audio::play::Counters;
use columbina_audio::rt::{messages, samples as ring};
use columbina_audio::stream::Resampler;

use session::{Tone, PERIOD, RATE};

const TONE: Tone = Tone {
    hz: 440.0,
    amplitude: 0.3,
};

#[test]
#[allow(clippy::cast_precision_loss, reason = "a share of the time")]
fn sixteen_tracks_of_effects_play_at_eight_times_on_threads_with_nothing_short() {
    let seconds = std::env::var("COLUMBINA_STRESS_SECONDS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(10);
    // Another speed for a comparison (`COLUMBINA_STRESS_SPEED`).
    let speed = std::env::var("COLUMBINA_STRESS_SPEED")
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(8);
    let mut project = session::sixteen_tracks();
    // Long enough for the seconds at 8 times.
    for track in &mut project.timelines[0].tracks {
        track.clips[0].length = 25 * 8 * i64::try_from(seconds + 2).unwrap();
    }
    project.media[0].facts.duration = columbina_model::Time {
        ticks: 48_000 * 8 * 40 * i64::try_from(seconds + 2).unwrap(),
        scale: 48_000,
    };
    let counters = Arc::new(Counters::default());
    let (mut commands, taken) = messages(64);
    let (gone, mut retired) = messages(64);
    let engine = Engine::new(taken, gone, Arc::clone(&counters));
    let (mut player, mut control) = device::player((2, RATE), false, false, None);
    let mut graph = Graph::build(&project, 0, Options::new(MOST_BLOCK, Purpose::Preview)).unwrap();
    graph.set_warp(Warp::new(speed, 0, 0));
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
    std::thread::sleep(Duration::from_millis(300));
    commands.push(Command::Start(Box::new(session))).unwrap();
    let mix = MixThread::spawn(engine).unwrap();
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
    let cpu = Duration::from_nanos(mix.cpu());
    let busy = Duration::from_nanos(mix.busy());
    drop(mix);
    while retired.pop().is_some() {}
    let share = if cpu > Duration::ZERO { cpu } else { busy }.as_secs_f64() / elapsed.as_secs_f64();
    println!(
        "{seconds} s of 16 tracks at {speed} times: {runs} runs, {} short, {} reads short, the mix on \
         the processor {:.1} % of the time",
        control.shared.short(),
        counters.underruns(),
        share * 100.0
    );
    // A runner of CI is a part of a machine that others use too: what it takes to play this
    // is measured and told there, and not judged.
    if std::env::var_os("CI").is_some() {
        println!("on a runner of CI: measured, and not held to its bounds");
        return;
    }
    assert_eq!(control.shared.short(), 0);
    assert_eq!(counters.underruns(), 0);
    let most = if cfg!(debug_assertions) { 0.75 } else { 0.5 };
    assert!(share < most, "{share}");
}
