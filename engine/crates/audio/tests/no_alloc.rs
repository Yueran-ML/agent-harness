//! What the thread of the mix does takes no memory: the nodes processing, set and reset, the
//! smoothing, the transforms, the rings and the seqlock, counted by an allocator of this test
//! binary's own.

use columbina_audio::fft::RealFft;
use columbina_audio::rt::{messages, samples, seqlock, Domain, Smoothed};
use columbina_audio::testing::{noise, realtime, CountingAllocator, Delay, Reference};
use columbina_audio::{Automated, Automation, Block, Node, ParamId, Planar, Spec};

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
#[allow(clippy::cast_precision_loss, clippy::cast_possible_wrap, reason = "the counts of a test")]
fn ten_seconds_of_the_work_of_the_thread_of_the_mix_take_no_memory() {
    const RATE: u32 = 48_000;
    const BLOCK: usize = 480;
    let spec = Spec::new(RATE, 512, 2);

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
    let (mut to_card, mut card) = samples(8_192);
    let mut played = vec![0.0; 2 * BLOCK];
    let (mut changes, mut changes_in) = messages::<(ParamId, f32)>(64);
    let (mut meter, meter_out) = seqlock([0.0_f32; 4]);

    let ((), counted) = realtime(|| {
        for n in 0..(10 * RATE as usize / BLOCK) {
            let start = (n * BLOCK) as i64;
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
            let block = Block::new(start, RATE, BLOCK).with_automation(automation);
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
