//! What the thread of the mix does takes no memory: the nodes processing, set and reset, the
//! smoothing, the transforms, the rings and the seqlock, counted by an allocator of this test
//! binary's own.

use columbina_audio::fft::RealFft;
use columbina_audio::rt::{messages, samples, seqlock, Domain, Smoothed};
use columbina_audio::testing::{noise, realtime, CountingAllocator, Delay, Reference};
use columbina_audio::{Automated, Automation, Block, Layout, Node, ParamId, Planar, Spec};

mod session;

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

#[test]
fn the_count_sees_what_a_section_allocates_and_frees() {
    let (boxed, counted) = realtime(|| Box::new([0_u8; 64]));
    assert_eq!(counted.allocations, 1);
    assert_eq!(counted.frees, 0);
    let ((), counted) = realtime(|| drop(boxed));
    assert_eq!(counted.frees, 1);
    let (grown, counted) = realtime(|| {
        let mut grown = Vec::with_capacity(1);
        grown.extend_from_slice(&[1, 2, 3]);
        grown
    });
    assert!(counted.allocations >= 2, "{counted:?}");
    drop(grown);
    let ((), counted) = realtime(|| {});
    assert!(counted.none());
}

#[test]
#[allow(clippy::cast_precision_loss, reason = "the counts of a test")]
fn ten_seconds_of_the_work_of_the_thread_of_the_mix_take_no_memory() {
    const RATE: u32 = 48_000;
    const BLOCK: usize = 480;
    let spec = Spec::new(RATE, 512, Layout::Stereo);

    // Built off the thread of the mix.
    let mut reference = Reference::new(0.9, 64);
    reference.prepare(&spec).unwrap();
    let mut delay = Delay::new(100);
    delay.prepare(&spec).unwrap();
    let mut input = Planar::new(2, 512);
    input.channel_mut(0).copy_from_slice(&noise(1, 0.5, 512));
    input.channel_mut(1).copy_from_slice(&noise(2, 0.5, 512));
    let mut between = Planar::new(2, 512);
    let mut output = Planar::new(2, 512);
    let lane = noise(3, 1.0, 512);
    let mut fader = Smoothed::new(1.0, Domain::Linear);
    fader.prepare(RATE);
    let mut gains = vec![0.0; 512];
    let mut fft = RealFft::new(2_048);
    let mut signal = noise(4, 1.0, 2_048);
    let mut spectrum = fft.spectrum();
    let (mut to_card, mut card) = samples(4_096, 2);
    let mut played = vec![0.0; 2 * BLOCK];
    let (mut changes, mut changes_in) = messages::<(ParamId, f32)>(64);
    let (mut meter, meter_out) = seqlock([0.0_f32; 4]);

    let mut processed = 0_u64;

    let ((), counted) = realtime(|| {
        for n in 0..(10 * RATE as usize / BLOCK) {
            // Parameters sent from elsewhere, taken at the edge of the block.
            if n % 50 == 0 {
                let _ = changes.push((Reference::GAIN, (n % 7) as f32 / 7.0));
            }
            while let Some((param, value)) = changes_in.pop() {
                reference.set(param, value);
                fader.set(value);
            }
            if n == 500 {
                reference.reset();
                delay.reset();
                processed = 0;
            }
            let lanes = [Automated {
                param: Reference::GAIN,
                values: &lane[..BLOCK],
            }];
            let automation = if n % 3 == 0 {
                Automation::new(&lanes)
            } else {
                Automation::NONE
            };
            let block = Block::new(processed, RATE, BLOCK).with_automation(automation);
            processed += BLOCK as u64;
            input.read(BLOCK, |input| {
                between.write(BLOCK, |between| reference.process(&block, input, between));
            });
            between.read(BLOCK, |between| {
                output.write(BLOCK, |output| delay.process(&block, between, output));
            });
            fader.fill(&mut gains[..BLOCK]);
            for (k, gain) in gains[..BLOCK].iter().enumerate() {
                played[2 * k] = output.channel(0)[k] * gain;
                played[2 * k + 1] = output.channel(1)[k] * gain;
            }
            to_card.push(&played);
            card.pop(&mut played);
            if n % 10 == 0 {
                fft.forward(&mut signal, &mut spectrum);
                fft.inverse(&mut spectrum, &mut signal);
            }
            meter.write([output.channel(0)[0], output.channel(1)[0], gains[0], 0.0]);
            let _ = meter_out.try_read();
        }
    });
    assert!(counted.none(), "{counted:?}");
}

#[test]
fn the_graph_of_a_timeline_plays_ten_seconds_with_no_memory() {
    use columbina_audio::feed::FeedError;
    use columbina_audio::graph::{Graph, Live, Options, Owner, Purpose};
    let project = session::sixteen_tracks();
    let mut graph = Graph::build(&project, 0, Options::new(512, Purpose::Preview)).unwrap();
    let fader = graph
        .control(&Owner::Track("A5".into()), "fader_db")
        .unwrap();
    let threshold = graph
        .control(
            &Owner::Track("A3".into()),
            "dynamics.compressor.threshold_db",
        )
        .unwrap();
    let vca = graph
        .control(&Owner::Vca("VCA1".into()), "fader_db")
        .unwrap();
    let pan = graph.control(&Owner::Track("A2".into()), "pan.x").unwrap();
    let duck = graph
        .control(&Owner::Track("A7".into()), "track_fx.hold_ms")
        .unwrap();
    // Values held live in place of lanes, and a glide, as automation is recorded (T16).
    let held = graph
        .lane_key(&Owner::Track("A4".into()), "fader_db")
        .unwrap();
    let panned = graph.lane_key(&Owner::Track("A6".into()), "pan.x").unwrap();
    let mut feed = |_: usize, range: std::ops::Range<i64>, out: &mut [&mut [f32]]| {
        for out in out.iter_mut() {
            for (n, sample) in out.iter_mut().enumerate() {
                #[allow(
                    clippy::cast_precision_loss,
                    clippy::cast_possible_wrap,
                    reason = "a test signal"
                )]
                let value = ((range.start + n as i64) % 101) as f32 / 101.0 - 0.5;
                *sample = value;
            }
        }
        Ok::<(), FeedError>(())
    };
    let monitor = graph.monitor().unwrap();
    let mut device = vec![0.0_f32; 512 * 2];
    let mut heard = false;
    // The meters hear it, the loudness of the monitor too (T9b).
    let taps = graph.taps();
    let ((), counted) = realtime(|| {
        graph.set_metering(true);
        graph.start(4800, &mut feed).unwrap();
        for block in 0..1000 {
            if block == 500 {
                graph.set_metering(false);
                graph.set_metering(true);
            }
            if block % 7 == 0 {
                #[allow(clippy::cast_precision_loss, reason = "a test value")]
                let value = -((block % 20) as f32);
                graph.set(fader, value);
                graph.set(threshold, value);
                graph.set(vca, value / 2.0);
                graph.set(pan, value / 20.0);
                graph.set(duck, -value * 10.0);
            }
            #[allow(clippy::cast_precision_loss, reason = "a test value")]
            let value = -((block % 13) as f32);
            match block % 11 {
                2 => graph.live(held, Live::Hold(value)),
                4 => graph.live(panned, Live::Hold(value / 13.0)),
                6 => graph.live(held, Live::Offset(value / 2.0)),
                8 => {
                    graph.live(held, Live::Release(2_400));
                    graph.live(panned, Live::Release(0));
                    graph.glide(vca, value, 2_400);
                }
                _ => {}
            }
            graph.process(&mut feed, 480).unwrap();
            graph.write_device(monitor, 480, &mut device);
            heard |= device.iter().any(|&sample| sample != 0.0);
        }
    });
    assert!(counted.none(), "{counted:?}");
    assert!(heard);
    // 10 s of cells of 10 ms, published on the thread of the mix.
    assert_eq!(taps.loudness.as_ref().unwrap().waiting(), 1000);
    assert!(taps.meters.iter().all(|meter| meter.post.waiting() == 1000));
}

#[test]
fn ten_seconds_of_a_session_of_sixteen_tracks_take_no_memory_on_the_threads_of_the_sound() {
    use columbina_audio::graph::{Live, Owner};
    use session::{Sim, Tone};
    let project = session::sixteen_tracks();
    // Another graph to swap in: an equalizer of A3 off, which takes its node out.
    let mut other = project.clone();
    other.timelines[0].tracks[3].audio.strip.eq.on = false;
    let tone = Tone {
        hz: 440.0,
        amplitude: 0.3,
    };
    let mut sim = Sim::of(&project, 4800, tone);
    let (a5, a3, a7, vca) = (
        Owner::Track("A5".into()),
        Owner::Track("A3".into()),
        Owner::Track("A7".into()),
        Owner::Vca("VCA1".into()),
    );
    let mut counted = Vec::new();
    for tick in 0..1000 {
        // Off the threads of the sound: reading ahead, and the graphs and the session to come
        // made.
        sim.pump();
        if tick == 400 {
            sim.swap(&other, 4800);
        }
        let prepared = (tick == 600).then(|| sim.prepare(&project, 96_000));
        // Two graphs sent while another waits (the second takes the place of the first after
        // it), and one too late.
        let swaps = (tick == 700).then(|| {
            let mixed = sim.counters.mixed();
            [
                (sim.run_of(&other, mixed + 9_600), mixed + 9_600),
                (sim.run_of(&project, mixed + 12_000), mixed + 12_000),
                (sim.run_of(&project, mixed + 12_100), mixed + 12_100),
                (sim.run_of(&other, mixed - 480), mixed - 480),
            ]
        });
        // On them: the changes of the settings sent, a new session as a seek starts one, the
        // graphs sent, the mix topped up, the device's run.
        let ((), count) = realtime(|| {
            if let Some(prepared) = prepared {
                sim.send(prepared);
            }
            if let Some(swaps) = swaps {
                for (run, at) in swaps {
                    sim.send_swap(run, at);
                }
            }
            if tick % 2 == 0 {
                #[allow(clippy::cast_precision_loss, reason = "a test value")]
                let value = -((tick % 20) as f32);
                sim.set(&a5, "fader_db", value);
                sim.set(&a3, "dynamics.compressor.threshold_db", value);
                sim.set(&vca, "fader_db", value / 2.0);
                sim.set(&a7, "track_fx.hold_ms", -value * 10.0);
            }
            // A control touched, trimmed and let go while automation is recorded (T16).
            #[allow(clippy::cast_precision_loss, reason = "a test value")]
            let value = -((tick % 13) as f32);
            match tick % 9 {
                1 => sim.live(&a5, "pan.x", Live::Hold(value / 13.0)),
                3 => sim.live(&a3, "fader_db", Live::Offset(value / 2.0)),
                5 => sim.live(&a3, "fader_db", Live::Release(2_400)),
                7 => {
                    sim.live(&a5, "pan.x", Live::Release(0));
                    sim.glide(&vca, "fader_db", value, 2_400);
                }
                _ => {}
            }
            sim.device_run();
        });
        counted.push(count);
    }
    let all = counted
        .iter()
        .fold((0, 0), |(a, f), c| (a + c.allocations, f + c.frees));
    assert_eq!(
        all,
        (0, 0),
        "allocations and frees on the threads of the sound"
    );
    // The swap at 4 s, and after the seek two of the four sent: the first, and the third, which
    // took the place of the second behind it; the last came too late.
    assert_eq!(sim.counters.swaps(), 3);
    assert_eq!(sim.counters.late(), 1);
    sim.pump();
    assert_eq!(sim.late, 1);
    assert_eq!(sim.counters.underruns(), 0);
    assert_eq!(sim.control.shared.short(), 0);
    assert!(sim.played.iter().any(|&sample| sample != 0.0));
    // The meters of the graphs that played heard them on the thread of the mix (T9b).
    assert!(sim.taps[0].loudness.as_ref().unwrap().waiting() > 300);
    assert!(sim.taps[0]
        .meters
        .iter()
        .all(|meter| meter.post.waiting() > 300));
}

#[test]
fn a_session_at_speeds_and_one_of_grains_of_scrubbing_take_no_memory_on_the_threads_of_the_sound() {
    use columbina_audio::graph::Owner;
    use session::{Sim, Tone};
    // Sixteen tracks played backward at 3, changed to 8 forward while they play (T18); then a
    // session of grains of one track, each next swapped in while the one before sounds.
    let project = session::sixteen_tracks();
    let tone = Tone {
        hz: 440.0,
        amplitude: 0.3,
    };
    let a5 = Owner::Track("A5".into());
    let mut counted = Vec::new();
    let mut sim = Sim::at_speed(&project, 240_000, tone, -3);
    for tick in 0..300 {
        sim.pump();
        let change = (tick == 100).then(|| {
            let at = sim.counters.mixed() + 4_800;
            sim.warp = sim.warp.with_speed(8, at);
            (sim.run_of(&project, at), at)
        });
        let ((), count) = realtime(|| {
            if let Some((run, at)) = change {
                sim.send_swap(run, at);
            }
            if tick % 4 == 0 {
                #[allow(clippy::cast_precision_loss, reason = "a test value")]
                sim.set(&a5, "fader_db", -((tick % 12) as f32));
            }
            sim.device_run();
        });
        counted.push(count);
    }
    assert_eq!(sim.counters.swaps(), 1);
    // Grains of one track (a graph is made for each of a hundred of them).
    let project = session::one_track();
    let mut grains = Sim::scrubbing(&project, 96_000, tone);
    for tick in 0..300 {
        grains.pump();
        // A grain every 30 ms: off the threads of the sound, its graph is made.
        let grain = (tick % 3 == 0).then(|| {
            let at = grains.counters.mixed() + 960;
            let (warp, place) = (grains.warp, 96_000 + 997 * i64::from(tick));
            grains.warp = columbina_audio::graph::Warp::new(1, at, place);
            let run = grains.run_of(&project, at);
            grains.warp = warp;
            (Box::new(run.with_grain(Sim::grain_at(at))), at)
        });
        let ((), count) = realtime(|| {
            if let Some((run, at)) = grain {
                grains.send_swap(run, at);
            }
            grains.device_run();
        });
        counted.push(count);
    }
    let all = counted
        .iter()
        .fold((0, 0), |(a, f), c| (a + c.allocations, f + c.frees));
    assert_eq!(
        all,
        (0, 0),
        "allocations and frees on the threads of the sound"
    );
    assert!(grains.counters.swaps() > 90, "{}", grains.counters.swaps());
    for sim in [&sim, &grains] {
        assert_eq!(sim.counters.underruns(), 0);
        assert_eq!(sim.control.shared.short(), 0);
        assert!(sim.played.iter().any(|&sample| sample != 0.0));
    }
}
