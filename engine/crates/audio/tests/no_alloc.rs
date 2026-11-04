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
    use columbina_audio::graph::{Graph, Options, Owner, Purpose};
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
    let ((), counted) = realtime(|| {
        graph.start(4800, &mut feed).unwrap();
        for block in 0..1000 {
            if block % 7 == 0 {
                #[allow(clippy::cast_precision_loss, reason = "a test value")]
                let value = -((block % 20) as f32);
                graph.set(fader, value);
                graph.set(threshold, value);
                graph.set(vca, value / 2.0);
                graph.set(pan, value / 20.0);
                graph.set(duck, -value * 10.0);
            }
            graph.process(&mut feed, 480).unwrap();
            graph.write_device(monitor, 480, &mut device);
            heard |= device.iter().any(|&sample| sample != 0.0);
        }
    });
    assert!(counted.none(), "{counted:?}");
    assert!(heard);
}

#[test]
fn ten_seconds_of_a_session_of_sixteen_tracks_take_no_memory_on_the_threads_of_the_sound() {
    use columbina_audio::graph::Owner;
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
        // Off the threads of the sound: reading ahead, and the graph to swap in built.
        sim.pump();
        if tick == 400 {
            sim.swap(&other, 4800);
        }
        // On them: the changes of the settings sent, the mix topped up, the device's run.
        let ((), count) = realtime(|| {
            if tick % 2 == 0 {
                #[allow(clippy::cast_precision_loss, reason = "a test value")]
                let value = -((tick % 20) as f32);
                sim.set(&a5, "fader_db", value);
                sim.set(&a3, "dynamics.compressor.threshold_db", value);
                sim.set(&vca, "fader_db", value / 2.0);
                sim.set(&a7, "track_fx.hold_ms", -value * 10.0);
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
    assert_eq!(sim.counters.swaps(), 1);
    assert_eq!(sim.counters.underruns(), 0);
    assert_eq!(sim.control.shared.short(), 0);
    assert!(sim.played.iter().any(|&sample| sample != 0.0));
}
