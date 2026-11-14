//! The voice mode of Noise Reduction (T12): `RNNoise` at 48 kHz and, through the node's own
//! resampling, at other rates; its delay, the mix, the settling, blocks, resets, bad input, and
//! the memory of the thread of the mix.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::float_cmp,
    reason = "the numbers of tests"
)]

mod denoise_signals;

use columbina_audio::constants::{RNNOISE_RESAMPLE_BAND, RNNOISE_SPARE_STATES};
use columbina_audio::fx::denoise::{Mode, NoiseReduction, Param, Settings};
use columbina_audio::fx::rnnoise::{applies, warm_thread, History, Resampler, Voice};
use columbina_audio::testing::{
    assert_latency, assert_settle, chunk_invariance, noise, realtime, render, rt_violations, sine,
    CountingAllocator, Run,
};
use columbina_audio::{Automated, Automation, Block, Layout, Node, Planar, Spec};
use denoise_signals::{kept, left, silences, speeches, sum, voice, white};

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn voiced() -> Settings {
    Settings {
        mode: Mode::Voice,
        ..Settings::default()
    }
}

fn with(edit: impl FnOnce(&mut Settings)) -> Settings {
    let mut settings = voiced();
    edit(&mut settings);
    settings
}

fn run(settings: &Settings, spec: &Spec, input: Vec<Vec<f32>>) -> Vec<Vec<f32>> {
    let mut node = Voice::new(settings);
    let spec = spec.with_max_block(input[0].len());
    node.prepare(&spec).unwrap();
    render(&mut node, &spec, &Run::new(input), |left| left)
}

#[test]
fn its_delay_is_two_frames_at_48_khz_and_whole_at_other_rates() {
    assert_eq!(Voice::latency_for(48_000), Some(960));
    // 44.1 kHz: 17 taps of the way up, and the way back waits 160 samples of 48 kHz, its 19
    // taps rounded up so that 960 + 160 = 1120 is a multiple of 160 (147 samples of 44.1 kHz
    // for each 160).
    assert_eq!(Resampler::radius(44_100, 48_000), 17);
    assert_eq!(Resampler::radius(48_000, 44_100), 19);
    assert_eq!(Voice::latency_for(44_100), Some(17 + 1120 * 147 / 160));
    for rate in [
        8_000, 11_025, 16_000, 22_050, 32_000, 44_100, 88_200, 96_000, 192_000, 384_000,
    ] {
        let latency = Voice::latency_for(rate).unwrap();
        println!("{rate}: {latency}");
        assert!(latency > 0);
    }
    for rate in [7_999, 44_056, 47_999, 384_001] {
        assert_eq!(Voice::latency_for(rate), None, "{rate}");
        let mut node = Voice::new(&voiced());
        assert!(
            node.prepare(&Spec::new(rate, 64, Layout::Mono)).is_err(),
            "{rate}"
        );
    }
    for rate in [48_000, 44_100, 96_000] {
        let spec = Spec::new(rate, 512, Layout::Stereo);
        let measured = assert_latency(|| Voice::new(&voiced()), &spec);
        assert_eq!(
            Some(measured),
            Voice::latency_for(rate).map(|l| l as usize),
            "{rate}"
        );
    }
}

/// The resampling checked on its own: a tone taken from 44.1 kHz to 48 kHz lands where a tone
/// made at 48 kHz, delayed as declared, is; and back.
#[test]
fn the_resampling_puts_a_tone_where_the_delay_says() {
    for (from, to, hz) in [
        (44_100_u32, 48_000_u32, 1_000.0),
        (48_000, 44_100, 3_000.0),
        (96_000, 48_000, 5_000.0),
        (16_000, 48_000, 440.0),
    ] {
        let radius = Resampler::radius(from, to) as u64;
        let resampler = Resampler::new(from, to, radius + 3);
        let frames = from as usize;
        let tone = sine(from, hz, 0.5, frames);
        let mut history = History::new(4 * radius as usize + 8);
        let mut out = Vec::new();
        for (at, &x) in tone.iter().enumerate() {
            history.put(at as u64, x);
            for n in resampler.outputs_at(at as u64) {
                out.push((n, resampler.output(n, &history)));
            }
        }
        let mut largest = 0.0_f64;
        for &(n, y) in &out {
            // The place in the input, in its seconds.
            let place = n as f64 * f64::from(from) / f64::from(to) - (radius + 3) as f64;
            if place < 2.0 * radius as f64 || place > (frames - 1) as f64 {
                continue;
            }
            let expected = 0.5 * (std::f64::consts::TAU * hz * place / f64::from(from)).sin();
            largest = largest.max((f64::from(y) - expected).abs());
        }
        println!("{from} -> {to}: {largest:e}");
        assert!(largest < 1e-4, "{from} -> {to}: {largest}");
        // As many outputs as the rates say.
        let count = out.len() as f64;
        assert!((count - frames as f64 * f64::from(to) / f64::from(from)).abs() <= 1.0);
    }
    // The band: a tone just above the lower Nyquist frequency is stopped.
    let resampler = Resampler::new(48_000, 44_100, 40);
    let hz = 23_000.0;
    let tone = sine(48_000, hz, 0.5, 48_000);
    let mut history = History::new(256);
    let mut largest = 0.0_f32;
    for (at, &x) in tone.iter().enumerate() {
        history.put(at as u64, x);
        for n in resampler.outputs_at(at as u64) {
            if n > 1_000 {
                largest = largest.max(resampler.output(n, &history).abs());
            }
        }
    }
    println!("23 kHz into 44.1 kHz: {largest:e} (band {RNNOISE_RESAMPLE_BAND})");
    assert!(largest < 0.5 * 0.01, "{largest}: less than 40 dB down");
}

/// The plan's sound: white noise at −40 dBFS under a voice at −20 dBFS that comes and goes.
#[test]
fn the_voice_takes_the_noise_down_between_the_words_at_48_and_at_44_1_khz() {
    for rate in [48_000_u32, 44_100] {
        let frames = 8 * rate as usize;
        let clean = voice(rate, 0.1, frames);
        let input = sum(&clean, &white(201, 0.01, frames));
        let latency = Voice::latency_for(rate).unwrap() as usize;
        let output = run(
            &voiced(),
            &Spec::new(rate, 1024, Layout::Mono),
            vec![input.clone()],
        )
        .remove(0);
        let floor = left(
            &input,
            &output,
            latency,
            &silences(rate, frames - latency, 1.0, 0.1),
        );
        let kept = kept(
            &clean,
            &output,
            latency,
            &speeches(rate, frames - latency, 1.0, 0.05),
        );
        println!("{rate}: the noise between the words {floor:.2} dB, the voice {kept:.3} dB");
        assert!(floor <= -10.0, "{rate}: {floor} dB");
        assert!(kept.abs() <= 1.0, "{rate}: {kept} dB");
    }
}

#[test]
fn all_dry_and_bypassed_give_the_input_late_to_the_bit() {
    for rate in [48_000_u32, 44_100] {
        let frames = rate as usize;
        let input = vec![
            sum(&voice(rate, 0.1, frames), &white(211, 0.01, frames)),
            noise(212, 0.3, frames),
        ];
        let latency = Voice::latency_for(rate).unwrap() as usize;
        for settings in [with(|s| s.dry_wet = 0.0), with(|s| s.on = false)] {
            let output = run(
                &settings,
                &Spec::new(rate, 1024, Layout::Stereo),
                input.clone(),
            );
            for (channel, output) in output.iter().enumerate() {
                for n in 0..frames {
                    let expected = if n >= latency {
                        input[channel][n - latency]
                    } else {
                        0.0
                    };
                    assert_eq!(
                        output[n].to_bits(),
                        expected.to_bits(),
                        "{rate}, {channel} at {n}"
                    );
                }
            }
        }
    }
    // Listen lets out what is taken away.
    let frames = 48_000;
    let input = sum(&voice(48_000, 0.1, frames), &white(213, 0.01, frames));
    let spec = Spec::new(48_000, 1024, Layout::Mono);
    let wet = run(&voiced(), &spec, vec![input.clone()]).remove(0);
    let noise_only = run(&with(|s| s.listen = true), &spec, vec![input.clone()]).remove(0);
    for n in 960..frames {
        assert!(
            (wet[n] + noise_only[n] - input[n - 960]).abs() <= 1e-7,
            "at {n}"
        );
    }
    // The numbers of the gate change nothing.
    let plain = run(&voiced(), &spec, vec![input.clone()]);
    let other = run(
        &with(|s| {
            s.threshold_db = 40.0;
            s.attack_ms = 50.0;
            s.time_smoothing = false;
        }),
        &spec,
        vec![input],
    );
    assert_eq!(plain, other);
    assert!(applies(Param::DryWet) && applies(Param::On) && !applies(Param::ThresholdDb));
}

fn busy_run(rate: u32, frames: usize, channels: usize) -> Run {
    let input: Vec<Vec<f32>> = (0..channels)
        .map(|c| {
            sum(
                &voice(rate, 0.1, frames),
                &white(220 + c as u64, 0.01, frames),
            )
        })
        .collect();
    let ramp: Vec<f32> = (0..frames)
        .map(|n| 100.0 - 70.0 * n as f32 / frames as f32)
        .collect();
    let square: Vec<f32> = (0..frames).map(|n| ((n / 9_001) % 2) as f32).collect();
    Run::new(input)
        .with_change(3_000, Param::DryWet.id(), 50.0)
        .with_change(7_000, Param::LevelDb.id(), 9.0)
        .with_change(11_000, Param::Listen.id(), 1.0)
        .with_change(15_000, Param::Listen.id(), 0.0)
        .with_change(19_000, Param::On.id(), 0.0)
        .with_change(19_200, Param::On.id(), 1.0)
        .with_change(23_000, Param::ThresholdDb.id(), 30.0)
        .with_automation(Param::DryWet.id(), ramp)
        .with_automation(Param::On.id(), square)
}

#[test]
fn its_output_does_not_depend_on_the_blocks() {
    chunk_invariance(
        || Voice::new(&voiced()),
        &Spec::new(48_000, 1024, Layout::Stereo),
        &busy_run(48_000, 30_000, 2),
        &[1, 2],
    );
    chunk_invariance(
        || Voice::new(&voiced()),
        &Spec::new(44_100, 700, Layout::Mono),
        &busy_run(44_100, 30_000, 1),
        &[3],
    );
    chunk_invariance(
        || NoiseReduction::new(&voiced()),
        &Spec::new(96_000, 512, Layout::Mono),
        &busy_run(96_000, 20_000, 1),
        &[4],
    );
}

/// Each reset makes the node as new, as long as fresh states are left; when they are spent, a
/// reset keeps a used state, and the node still comes within the settling.
#[test]
fn a_reset_takes_a_fresh_state_while_there_are_spares() {
    let frames = 20_000;
    let input = sum(&voice(48_000, 0.1, frames), &white(231, 0.01, frames));
    let spec = Spec::new(48_000, frames, Layout::Mono);
    let new = run(&voiced(), &spec, vec![input.clone()]);
    let mut node = Voice::new(&voiced());
    node.prepare(&spec).unwrap();
    let run_once = Run::new(vec![input]);
    assert_eq!(node.spares(), RNNOISE_SPARE_STATES);
    for reset in 0..RNNOISE_SPARE_STATES {
        let output = render(&mut node, &spec, &run_once, |left| left);
        assert_eq!(output, new, "after {reset} resets");
        node.reset();
        assert_eq!(node.spares(), RNNOISE_SPARE_STATES - reset - 1);
    }
    // A reset of a node that has taken no frame keeps its fresh state.
    node.reset();
    assert_eq!(node.spares(), 0);
    let output = render(&mut node, &spec, &run_once, |left| left);
    assert_eq!(output, new);
    // Spent: the next reset keeps the used state.
    node.reset();
    let output = render(&mut node, &spec, &run_once, |left| left);
    assert_ne!(output, new);
    // A prepare makes them again.
    node.prepare(&spec).unwrap();
    assert_eq!(node.spares(), RNNOISE_SPARE_STATES);
    assert_eq!(render(&mut node, &spec, &run_once, |left| left), new);
}

#[test]
fn it_settles_as_it_reports() {
    let rate = 48_000;
    let settle = Voice::settle_for(&voiced(), rate) as usize;
    println!(
        "the voice settles in {:.2} s",
        settle as f64 / f64::from(rate)
    );
    let frames = settle + 6 * rate as usize;
    let input = sum(&voice(rate, 0.1, frames), &white(241, 0.01, frames));
    let run = Run::new(vec![input]);
    for from in [rate as usize / 2 + 17, 2 * rate as usize + 300] {
        let largest = assert_settle(
            || Voice::new(&voiced()),
            &Spec::new(rate, 1024, Layout::Mono),
            &run,
            from,
        );
        println!("from {from}: {largest:e}");
    }
}

#[test]
fn bad_input_and_bad_lanes_do_not_stick() {
    let rate = 48_000;
    let settle = Voice::settle_for(&voiced(), rate) as usize;
    let frames = settle + 3 * rate as usize;
    let clean = sum(&voice(rate, 0.1, frames), &white(251, 0.01, frames));
    let mut bad = clean.clone();
    bad[10_000] = f32::INFINITY;
    bad[10_001] = f32::NAN;
    bad[20_000] = 1e30;
    let spec = Spec::new(rate, 1024, Layout::Mono);
    let good = run(&voiced(), &spec, vec![clean.clone()]).remove(0);
    let spoiled = run(&voiced(), &spec, vec![bad]).remove(0);
    assert!(spoiled.iter().all(|x| x.is_finite()));
    let mut largest = 0.0_f32;
    for n in 20_000 + settle..frames {
        largest = largest.max((good[n] - spoiled[n]).abs());
    }
    println!("after an infinity: {largest:e}");
    assert!(largest < 3.2e-5);
    // Lanes of NaN keep the values set.
    let short = clean[..rate as usize].to_vec();
    let plain = run(&voiced(), &spec, vec![short.clone()]);
    for param in [Param::DryWet, Param::LevelDb, Param::On] {
        let mut node = Voice::new(&voiced());
        let spec = spec.with_max_block(short.len());
        node.prepare(&spec).unwrap();
        let lane = vec![f32::NAN; short.len()];
        let run = Run::new(vec![short.clone()]).with_automation(param.id(), lane);
        assert_eq!(
            render(&mut node, &spec, &run, |left| left),
            plain,
            "{param:?}"
        );
    }
}

#[test]
fn it_takes_no_memory_on_the_thread_of_the_mix() {
    // As the thread of the mix does before it runs (prepare does not).
    warm_thread();
    for rate in [48_000_u32, 44_100] {
        let spec = Spec::new(rate, 480, Layout::Stereo);
        let mut node = Voice::new(&voiced());
        node.prepare(&spec).unwrap();
        node.align(777);
        let mut input = Planar::new(2, 480);
        let mut output = Planar::new(2, 480);
        let lane = vec![70.0_f32; 480];
        let signal = sum(&voice(rate, 0.1, 480 * 600), &white(261, 0.01, 480 * 600));
        let ((), allocations) = realtime(|| {
            for block in 0..600 {
                for channel in 0..2 {
                    input
                        .channel_mut(channel)
                        .copy_from_slice(&signal[block * 480..(block + 1) * 480]);
                }
                if block % 7 == 0 {
                    node.set(Param::DryWet.id(), 50.0 + (block % 50) as f32);
                    node.set(Param::Listen.id(), (block % 2) as f32);
                }
                if block % 100 == 50 {
                    node.reset();
                }
                let lanes = [Automated {
                    param: Param::DryWet.id(),
                    values: &lane,
                }];
                let automation = if block % 3 == 0 {
                    Automation::new(&lanes)
                } else {
                    Automation::NONE
                };
                let start = ((block
                    - if block >= 50 {
                        (block - 50) / 100 * 100 + 50
                    } else {
                        0
                    })
                    * 480) as u64;
                let ctx = Block::new(start, rate, 480).with_automation(automation);
                input.read(480, |input| {
                    output.write(480, |output| node.process(&ctx, input, output));
                });
                let _ = node.settle();
            }
        });
        assert!(allocations.none(), "{rate}: {allocations:?}");
    }
}

/// `nnnoiseless` plans its transforms on each thread on first use: a thread that has not been
/// warmed takes memory on its first frame, and none once [`warm_thread`] has run on it.
#[test]
fn the_thread_of_the_mix_is_warmed_before_it_runs() {
    let spec = Spec::new(48_000, 480, Layout::Mono);
    let mut node = Voice::new(&voiced());
    node.prepare(&spec).unwrap();
    let signal = sum(&voice(48_000, 0.1, 4_800), &white(271, 0.01, 4_800));
    let (cold, warm) = std::thread::spawn(move || {
        let mut output = vec![0.0; 480];
        let mut process = |node: &mut Voice, block: usize| {
            let ctx = Block::new((block * 480) as u64, 48_000, 480);
            node.process(
                &ctx,
                &[&signal[block * 480..(block + 1) * 480]],
                &mut [&mut output],
            );
        };
        let ((), cold) = realtime(|| process(&mut node, 0));
        node.reset();
        let mut fresh = Voice::new(&voiced());
        fresh.prepare(&spec).unwrap();
        let ((), again) = realtime(|| process(&mut fresh, 0));
        warm_thread();
        let ((), warm) = realtime(|| {
            for block in 1..10 {
                process(&mut fresh, block);
            }
        });
        assert!(again.none(), "a second node on the thread: {again:?}");
        (cold, warm)
    })
    .join()
    .unwrap();
    assert!(
        !cold.none(),
        "the first frame on a cold thread takes memory: {cold:?}"
    );
    assert!(warm.none(), "{warm:?}");
}

#[test]
fn the_voice_holds_no_word_of_locks_allocations_or_logs() {
    assert_eq!(rt_violations(include_str!("../src/fx/rnnoise.rs")), []);
}

/// The time of the voice: mono, 60 s at 48 and at 44.1 kHz (run in release:
/// `cargo test --release -p columbina-audio --test rnnoise -- --ignored --nocapture`).
#[test]
#[ignore = "a measure of time, in release"]
fn a_minute_of_mono_in_the_voice() {
    for rate in [48_000_u32, 44_100] {
        let frames = 60 * rate as usize;
        let input = sum(&voice(rate, 0.1, frames), &white(281, 0.01, frames));
        let mut node = Voice::new(&voiced());
        node.prepare(&Spec::new(rate, 1024, Layout::Mono)).unwrap();
        let mut output = vec![0.0; 1024];
        let start = std::time::Instant::now();
        for (block, chunk) in input.chunks(1024).enumerate() {
            let ctx = Block::new((block * 1024) as u64, rate, chunk.len());
            node.process(&ctx, &[chunk], &mut [&mut output[..chunk.len()]]);
        }
        let elapsed = start.elapsed();
        println!("{rate}: 60 s of mono in {elapsed:?}");
        assert!(elapsed.as_secs_f64() < 6.0);
    }
}

/// A node aligned to `origin` whenever it is prepared or reset, as the graph aligns it to where
/// a run starts.
struct Aligned<N> {
    node: N,
    origin: u64,
}

impl<N: Node> Node for Aligned<N> {
    fn prepare(&mut self, spec: &Spec) -> Result<(), columbina_audio::Unsupported> {
        self.node.prepare(spec)?;
        self.node.align(self.origin);
        Ok(())
    }
    fn process(&mut self, ctx: &Block, input: &[&[f32]], output: &mut [&mut [f32]]) {
        self.node.process(ctx, input, output);
    }
    fn latency(&self) -> u32 {
        self.node.latency()
    }
    fn settle(&self) -> u32 {
        self.node.settle()
    }
    fn reset(&mut self) {
        self.node.reset();
        self.node.align(self.origin);
    }
    fn set(&mut self, param: columbina_audio::ParamId, value: f32) {
        self.node.set(param, value);
    }
}

/// Laid on a grid that does not start at 0, the output still does not depend on the blocks.
#[test]
fn aligned_off_the_start_its_output_does_not_depend_on_the_blocks() {
    for origin in [12_345_u64, 1_000_000_000_007] {
        chunk_invariance(
            || Aligned {
                node: Voice::new(&voiced()),
                origin,
            },
            &Spec::new(44_100, 600, Layout::Stereo),
            &busy_run(44_100, 30_000, 2),
            &[7],
        );
    }
}

/// `RNNoise` keeps its state through digital silence; after a second of it the voice takes a
/// fresh state, so that an export from the start and a playback started inside a long gap give
/// the same bits from the second clip on (a gap of 80 s, longer than the settling).
#[test]
fn after_a_second_of_silence_a_start_inside_the_gap_hears_what_an_export_hears() {
    warm_thread();
    let rate = 48_000_usize;
    let clip = |seed: u64| sum(&voice(48_000, 0.1, 3 * rate), &white(seed, 0.01, 3 * rate));
    let mut input = clip(291);
    input.extend(std::iter::repeat_n(0.0, 80 * rate));
    let second = input.len();
    input.extend(clip(292));
    let spec = Spec::new(48_000, input.len(), Layout::Mono);
    let mut early = Voice::new(&voiced());
    early.prepare(&spec).unwrap();
    let whole = render(&mut early, &spec, &Run::new(vec![input.clone()]), |left| {
        left
    })
    .remove(0);
    for from in [40 * rate + 77, second - rate / 3] {
        let mut late = Voice::new(&voiced());
        late.prepare(&spec).unwrap();
        late.align(from as u64);
        let tail = render(
            &mut late,
            &spec,
            &Run::new(vec![input[from..].to_vec()]),
            |left| left,
        )
        .remove(0);
        for t in second..input.len() {
            assert_eq!(
                whole[t].to_bits(),
                tail[t - from].to_bits(),
                "from {from}, at {t}"
            );
        }
    }
}
