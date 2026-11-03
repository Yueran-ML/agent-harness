//! The interface of a node and the checks that every node of the graph goes through: the output
//! does not depend on the blocks, the latency reported is the delay of the output, the settling
//! time reported is enough.

#![allow(clippy::float_cmp, reason = "the outputs are exact")]

use columbina_audio::testing::{
    assert_latency, assert_settle, chunk_invariance, impulse, measured_latency, noise, render,
    sine, Delay, Reference, Run,
};
use columbina_audio::{Automated, Automation, Block, Node, ParamId, Planar, Spec, Unsupported};
use proptest::prelude::*;

const RATE: u32 = 48_000;

/// A gain that ramps from 0 to 1 over each block it is given: its output depends on the blocks.
struct RampsEachBlock;

impl Node for RampsEachBlock {
    fn prepare(&mut self, _: &Spec) -> Result<(), Unsupported> {
        Ok(())
    }
    #[allow(clippy::cast_precision_loss, reason = "a test")]
    fn process(&mut self, ctx: &Block, input: &[&[f32]], output: &mut [&mut [f32]]) {
        for (output, input) in output.iter_mut().zip(input) {
            for (n, (out, x)) in output.iter_mut().zip(*input).enumerate() {
                *out = x * n as f32 / ctx.frames as f32;
            }
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

#[test]
#[should_panic(expected = "in blocks from seed")]
fn the_check_of_the_blocks_finds_a_node_whose_output_depends_on_them() {
    let run = Run::new(vec![noise(1, 0.5, 10_000)]);
    chunk_invariance(|| RampsEachBlock, &Spec::new(RATE, 4_096, 1), &run, &[1]);
}

/// A delay whose reset forgets nothing.
struct RemembersThroughAReset(Delay);

impl Node for RemembersThroughAReset {
    fn prepare(&mut self, spec: &Spec) -> Result<(), Unsupported> {
        self.0.prepare(spec)
    }
    fn process(&mut self, ctx: &Block, input: &[&[f32]], output: &mut [&mut [f32]]) {
        self.0.process(ctx, input, output);
    }
    fn latency(&self) -> u32 {
        self.0.latency()
    }
    fn settle(&self) -> u32 {
        self.0.settle()
    }
    fn reset(&mut self) {}
    fn set(&mut self, _: ParamId, _: f32) {}
}

#[test]
#[should_panic(expected = "after a reset")]
fn the_check_of_the_blocks_finds_a_reset_that_keeps_the_sound() {
    let run = Run::new(vec![noise(1, 0.5, 5_000)]);
    chunk_invariance(
        || RemembersThroughAReset(Delay::new(100)),
        &Spec::new(RATE, 4_096, 1),
        &run,
        &[1],
    );
}

#[test]
fn the_reference_node_is_the_same_in_any_blocks_with_changes_and_automation() {
    let frames = 30_000;
    let run = Run::new(vec![noise(1, 0.8, frames), sine(RATE, 997.0, 0.5, frames)])
        .starting_at(-1_000)
        .with_change(0, Reference::GAIN, 0.25)
        .with_change(4_321, Reference::GAIN, 1.0)
        .with_change(4_500, Reference::GAIN, 0.0)
        .with_change(20_000, Reference::GAIN, 0.7);
    let output = chunk_invariance(
        || Reference::new(0.9, 37),
        &Spec::new(RATE, 4_096, 2),
        &run,
        &[1, 2, 3, 4, 5],
    );
    assert_eq!(output.len(), 2);
    assert!(output[0].iter().any(|&x| x != 0.0));

    let automated = run.with_automation(Reference::GAIN, noise(9, 1.0, frames));
    chunk_invariance(
        || Reference::new(0.5, 0),
        &Spec::new(RATE, 4_096, 2),
        &automated,
        &[6, 7],
    );
}

#[test]
fn an_automated_parameter_takes_the_lane_sample_by_sample() {
    let frames = 1_000;
    let lane: Vec<f32> = (0..frames)
        .map(|n| if n < 500 { 0.0 } else { 1.0 })
        .collect();
    let run = Run::new(vec![vec![1.0; frames]]).with_automation(Reference::GAIN, lane);
    let mut node = Reference::new(0.0, 0);
    let spec = Spec::new(RATE, 256, 1);
    node.prepare(&spec).unwrap();
    let output = render(&mut node, &spec, &run, |left| left);
    assert!(output[0][..500].iter().all(|&x| x == 0.0));
    assert!(output[0][500..].iter().all(|&x| x == 1.0));
}

#[test]
fn a_parameter_set_ramps_over_ten_milliseconds() {
    let frames = 2_000;
    let run = Run::new(vec![vec![1.0; frames]]).with_change(100, Reference::GAIN, 0.0);
    let spec = Spec::new(RATE, 64, 1);
    let mut node = Reference::new(0.0, 0);
    node.prepare(&spec).unwrap();
    let output = render(&mut node, &spec, &run, |_| 64);
    assert!(output[0][..100].iter().all(|&x| x == 1.0));
    assert!((output[0][100 + 239] - 0.5).abs() < 1e-6);
    assert_eq!(output[0][100 + 479], 0.0);
}

#[test]
fn a_delay_reports_the_delay_of_its_output() {
    let spec = Spec::new(RATE, 512, 2);
    assert_eq!(assert_latency(|| Delay::new(64), &spec), 64);
    assert_eq!(assert_latency(|| Delay::new(0), &spec), 0);
    let mut node = Delay::new(64);
    node.prepare(&spec).unwrap();
    let run = Run::new(vec![impulse(2_000, 1_000, 1.0), impulse(2_000, 3, 0.5)]);
    let output = render(&mut node, &spec, &run, |_| 100);
    assert_eq!(output[0][1_064], 1.0);
    assert_eq!(output[1][67], 0.5);
    assert_eq!(output[0].iter().filter(|&&x| x != 0.0).count(), 1);
}

#[test]
fn a_node_that_lies_about_its_latency_is_found() {
    struct Lies(Delay);
    impl Node for Lies {
        fn prepare(&mut self, spec: &Spec) -> Result<(), Unsupported> {
            self.0.prepare(spec)
        }
        fn process(&mut self, ctx: &Block, input: &[&[f32]], output: &mut [&mut [f32]]) {
            self.0.process(ctx, input, output);
        }
        fn latency(&self) -> u32 {
            self.0.latency() - 1
        }
        fn settle(&self) -> u32 {
            self.0.settle()
        }
        fn reset(&mut self) {
            self.0.reset();
        }
        fn set(&mut self, _: ParamId, _: f32) {}
    }
    let found = std::panic::catch_unwind(|| {
        assert_latency(|| Lies(Delay::new(64)), &Spec::new(RATE, 512, 1));
    });
    assert!(found.is_err());
}

#[test]
fn a_node_that_settles_later_than_it_says_is_found() {
    struct Hurries(Reference);
    impl Node for Hurries {
        fn prepare(&mut self, spec: &Spec) -> Result<(), Unsupported> {
            self.0.prepare(spec)
        }
        fn process(&mut self, ctx: &Block, input: &[&[f32]], output: &mut [&mut [f32]]) {
            self.0.process(ctx, input, output);
        }
        fn latency(&self) -> u32 {
            self.0.latency()
        }
        fn settle(&self) -> u32 {
            self.0.settle() / 4
        }
        fn reset(&mut self) {
            self.0.reset();
        }
        fn set(&mut self, param: ParamId, value: f32) {
            self.0.set(param, value);
        }
    }
    let run = Run::new(vec![noise(3, 1.0, 20_000)]);
    let spec = Spec::new(RATE, 1_024, 1);
    assert_settle(|| Reference::new(0.99, 10), &spec, &run, 7_000);
    let found = std::panic::catch_unwind(|| {
        assert_settle(|| Hurries(Reference::new(0.99, 10)), &spec, &run, 7_000);
    });
    assert!(found.is_err());
}

#[test]
fn a_block_finds_its_automated_parameters() {
    let values = [0.5_f32; 4];
    let lanes = [Automated {
        param: ParamId(3),
        values: &values,
    }];
    let block = Block::new(10, RATE, 4).with_automation(Automation::new(&lanes));
    assert_eq!(block.automation.get(ParamId(3)), Some(&values[..]));
    assert_eq!(block.automation.get(ParamId(4)), None);
    assert!(Block::new(0, RATE, 4).automation.is_empty());
}

#[test]
fn a_planar_buffer_gives_the_slices_of_its_channels() {
    let mut buffer = Planar::new(3, 8);
    buffer.channel_mut(1).fill(2.0);
    buffer.write(5, |channels| {
        assert_eq!(channels.len(), 3);
        assert!(channels.iter().all(|channel| channel.len() == 5));
        channels[2][4] = 7.0;
    });
    buffer.read(8, |channels| {
        assert_eq!(channels[1], &[2.0; 8]);
        assert_eq!(channels[2][4], 7.0);
        assert_eq!(channels[0], &[0.0; 8]);
    });
    buffer.silence();
    assert!(buffer.channel(2).iter().all(|&x| x == 0.0));
    let empty = Planar::new(0, 16);
    empty.read(16, |channels| assert_eq!(channels.len(), 0));
}

#[test]
fn a_delay_does_not_take_more_channels_out_than_in() {
    let mut node = Delay::new(4);
    let spec = Spec::new(RATE, 64, 1).with_outputs(2);
    assert_eq!(
        node.prepare(&spec),
        Err(Unsupported {
            inputs: 1,
            outputs: 2,
            sidechain: 0
        })
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// A stateful node (a ramped gain, a low-pass in `f64`, a delay) gives the same bits however
    /// the sound is cut, wherever the parameters change, with and without automation.
    #[test]
    fn the_reference_node_does_not_depend_on_the_blocks(
        pole in 0.0_f64..0.999,
        delay in 0_usize..300,
        channels in 1_usize..4,
        max_block in 1_usize..4_097,
        changes in prop::collection::vec((0_usize..6_000, 0.0_f32..2.0), 0..6),
        automated in any::<bool>(),
        seed in any::<u64>(),
        start in -10_000_i64..10_000,
    ) {
        let frames = 6_000;
        let input = (0..channels).map(|c| noise(seed ^ c as u64, 0.9, frames)).collect();
        let mut run = Run::new(input).starting_at(start);
        for (at, value) in changes {
            run = run.with_change(at, Reference::GAIN, value);
        }
        if automated {
            run = run.with_automation(Reference::GAIN, noise(seed.wrapping_add(7), 1.0, frames));
        }
        chunk_invariance(
            || Reference::new(pole, delay),
            &Spec::new(RATE, max_block, channels),
            &run,
            &[seed, seed.wrapping_add(1)],
        );
    }

    /// The latency a delay reports is where its output is.
    #[test]
    fn a_delay_reports_where_its_output_is(
        delay in 0_usize..2_000,
        channels in 1_usize..3,
        max_block in 1_usize..2_048,
        pole in 0.0_f64..0.5,
    ) {
        let spec = Spec::new(RATE, max_block, channels);
        prop_assert_eq!(assert_latency(|| Delay::new(delay), &spec), delay);
        prop_assert_eq!(assert_latency(|| Reference::new(pole, delay), &spec), delay);
    }

    /// The reference node settles within the time it reports, from wherever it starts.
    #[test]
    fn the_reference_node_settles_as_it_says(
        pole in 0.0_f64..0.995,
        delay in 0_usize..500,
        from in 1_usize..10_000,
        seed in any::<u64>(),
        change in prop::option::of((0_usize..12_000, 0.0_f32..1.0)),
    ) {
        let mut run = Run::new(vec![noise(seed, 1.0, 12_000)]);
        if let Some((at, value)) = change {
            run = run.with_change(at, Reference::GAIN, value);
        }
        let spec = Spec::new(RATE, 512, 1);
        assert_settle(|| Reference::new(pole, delay), &spec, &run, from);
    }
}

#[test]
fn the_latency_measured_is_the_peak_of_the_correlation() {
    let input = noise(5, 1.0, 4_000);
    let mut output = vec![0.0; 4_000];
    output[123..].copy_from_slice(&input[..4_000 - 123]);
    assert_eq!(measured_latency(&input, &output, 500), 123);
}
