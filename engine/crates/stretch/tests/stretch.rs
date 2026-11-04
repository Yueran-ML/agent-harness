//! The stretcher of Signalsmith Stretch through its shim: its configuration, its errors, the same
//! bits after a reset, a pitch moved by an octave, a speed changed with the pitch kept, and no
//! memory taken where the sound is made.

use columbina_stretch::{allocations, Config, Error, Stretch, MAX_CHANNELS};

const RATE: u32 = 48_000;

/// A sine of `frequency` Hz at `RATE`, of amplitude 0.5.
fn sine(frequency: f64, frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|n| {
            #[allow(clippy::cast_precision_loss, reason = "a test signal")]
            let t = n as f64 / f64::from(RATE);
            #[allow(clippy::cast_possible_truncation, reason = "a sample")]
            let s = (0.5 * (std::f64::consts::TAU * frequency * t).sin()) as f32;
            s
        })
        .collect()
}

/// Noise from a seed, the same on every machine (`SplitMix64`), of amplitude 0.25.
fn noise(seed: u64, frames: usize) -> Vec<f32> {
    let mut state = seed;
    (0..frames)
        .map(|_| {
            state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^= z >> 31;
            #[allow(clippy::cast_precision_loss, reason = "a test signal")]
            let unit = (z >> 11) as f64 / (1_u64 << 53) as f64;
            #[allow(clippy::cast_possible_truncation, reason = "a sample")]
            let s = ((unit * 2.0 - 1.0) * 0.25) as f32;
            s
        })
        .collect()
}

/// The frequency of a steady tone, from the times its rising zero crossings fall on (linearly
/// between the samples), first to last.
fn frequency(signal: &[f32]) -> f64 {
    let mut crossings = Vec::new();
    for (n, pair) in signal.windows(2).enumerate() {
        if pair[0] <= 0.0 && pair[1] > 0.0 {
            let fraction = f64::from(pair[0]) / f64::from(pair[0] - pair[1]);
            #[allow(clippy::cast_precision_loss, reason = "a sample's index")]
            crossings.push(n as f64 + fraction);
        }
    }
    assert!(crossings.len() > 10, "{} crossings", crossings.len());
    let (first, last) = (crossings[0], crossings[crossings.len() - 1]);
    #[allow(clippy::cast_precision_loss, reason = "a count of crossings")]
    let periods = (crossings.len() - 1) as f64;
    periods * f64::from(RATE) / (last - first)
}

/// The cents from `reference` to `measured`.
fn cents(measured: f64, reference: f64) -> f64 {
    1200.0 * (measured / reference).log2()
}

/// Stretches `input` (one channel) through `stretch` in calls of `out` output samples, each fed
/// the input samples that bring it to `rate` of them for each output sample.
fn stretched(
    stretch: &mut Stretch,
    input: &[f32],
    rate: f64,
    out: usize,
    calls: usize,
) -> Vec<f32> {
    let mut output = Vec::new();
    let mut fed = 0usize;
    let mut chunk = vec![0.0_f32; out];
    for call in 1..=calls {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss,
            reason = "a count of samples of a test"
        )]
        let until = ((call * out) as f64 * rate).floor() as usize;
        let until = until.min(input.len());
        stretch.process(&[&input[fed..until]], &mut [&mut chunk[..]]);
        fed = until;
        output.extend_from_slice(&chunk);
    }
    output
}

#[test]
fn the_preset_takes_blocks_of_120_ms_every_30_ms_and_says_its_latencies() {
    let config = Config::of(RATE).unwrap();
    assert_eq!(config.block, 5_760);
    assert_eq!(config.interval, 1_440);
    assert_eq!(config.seek_length, config.block + config.interval);
    assert!(
        (1..=config.block).contains(&config.input_latency),
        "{config:?}"
    );
    assert!(
        (1..=config.block).contains(&config.output_latency),
        "{config:?}"
    );
    let stereo = Stretch::new(2, RATE).unwrap();
    assert_eq!(
        stereo.config(),
        config,
        "the same for any count of channels"
    );
    assert_eq!(stereo.channels(), 2);
}

#[test]
fn a_stretcher_takes_one_to_twenty_four_channels_at_a_rate_above_nought() {
    assert_eq!(Stretch::new(0, RATE).unwrap_err(), Error::Channels(0));
    assert_eq!(
        Stretch::new(MAX_CHANNELS + 1, RATE).unwrap_err(),
        Error::Channels(MAX_CHANNELS + 1)
    );
    assert_eq!(Stretch::new(1, 0).unwrap_err(), Error::Rate);
    assert!(Stretch::new(MAX_CHANNELS, 44_100).is_ok());
}

#[test]
fn the_same_input_after_a_reset_stretches_to_the_same_bits_even_where_phases_are_random() {
    // Slower than half speed: the library draws random phases (time factor above 2).
    let input = noise(7, RATE as usize * 2);
    let rate = 0.3;
    let mut stretch = Stretch::new(1, RATE).unwrap();
    let first = stretched(&mut stretch, &input, rate, 64, 4_000);
    stretch.reset();
    let again = stretched(&mut stretch, &input, rate, 64, 4_000);
    let mut fresh = Stretch::new(1, RATE).unwrap();
    let new = stretched(&mut fresh, &input, rate, 64, 4_000);
    assert!(first.iter().any(|s| s.abs() > 0.01), "it sounds");
    let differs = |a: &[f32], b: &[f32]| {
        a.iter()
            .zip(b)
            .position(|(a, b)| a.to_bits() != b.to_bits())
    };
    assert_eq!(differs(&first, &again), None, "after a reset");
    assert_eq!(differs(&first, &new), None, "a new stretcher");
}

#[test]
fn a_transpose_of_two_moves_a_tone_up_an_octave() {
    let input = sine(440.0, RATE as usize * 2);
    let mut stretch = Stretch::new(1, RATE).unwrap();
    stretch.set_transpose(2.0);
    let output = stretched(&mut stretch, &input, 1.0, 128, 700);
    let steady = &output[RATE as usize / 2..RATE as usize * 3 / 2];
    let measured = frequency(steady);
    assert!(cents(measured, 880.0).abs() < 5.0, "{measured} Hz");
}

#[test]
fn a_change_of_speed_keeps_the_pitch() {
    let input = sine(440.0, RATE as usize * 3);
    let mut stretch = Stretch::new(1, RATE).unwrap();
    // Played at 0.8: 0.8 input samples for each output sample.
    let output = stretched(&mut stretch, &input, 0.8, 128, 900);
    let steady = &output[RATE as usize / 2..RATE as usize * 2];
    let measured = frequency(steady);
    assert!(cents(measured, 440.0).abs() < 5.0, "{measured} Hz");
}

#[test]
fn a_seek_then_process_and_a_reset_take_no_memory_of_cpp() {
    // The count works: making a stretcher takes memory.
    let was = allocations::arm(true);
    let before = allocations::counted();
    let mut stretch = Stretch::new(2, RATE).unwrap();
    let made = allocations::counted();
    assert!(made.0 > before.0, "the count sees what making takes");
    let _ = allocations::arm(was);

    let left = noise(1, RATE as usize);
    let right = noise(2, RATE as usize);
    let mut out = [vec![0.0_f32; 64], vec![0.0_f32; 64]];
    let was = allocations::arm(true);
    let before = allocations::counted();
    for round in 0..3 {
        stretch.reset();
        stretch.set_transpose(if round == 1 { 1.5 } else { 1.0 });
        let seek = 7_200;
        stretch.seek(&[&left[..seek], &right[..seek]], 0.4);
        let mut fed = seek;
        for call in 0..300 {
            let take = if call % 3 == 0 { 30 } else { 19 };
            let [a, b] = &mut out;
            stretch.process(
                &[&left[fed..fed + take], &right[fed..fed + take]],
                &mut [&mut a[..], &mut b[..]],
            );
            fed += take;
        }
    }
    let after = allocations::counted();
    let _ = allocations::arm(was);
    assert_eq!(
        after, before,
        "allocations and frees of C++ while processing"
    );
}

/// Runs `stretch` from a reset as a segment does: `seek` with silence, then calls of 64 outputs
/// at `rate` input samples each over a tone of 440 Hz at `sample_rate` that starts after
/// `silent` input samples (never, for `usize::MAX`).
fn segment(
    stretch: &mut Stretch,
    sample_rate: u32,
    silent: usize,
    outputs: usize,
    rate: f64,
) -> Vec<f32> {
    let seek = stretch.config().seek_length;
    stretch.reset();
    let zeros = vec![0.0_f32; seek];
    stretch.seek(&[&zeros], rate);
    let signal = |x: usize| -> f32 {
        if x < silent {
            0.0
        } else {
            #[allow(
                clippy::cast_precision_loss,
                clippy::cast_possible_truncation,
                reason = "a test tone"
            )]
            let s = (0.5
                * (std::f64::consts::TAU * 440.0 * x as f64 / f64::from(sample_rate)).sin())
                as f32;
            s
        }
    };
    let mut out = Vec::with_capacity(outputs);
    let mut chunk = [0.0_f32; 64];
    let mut call = 0usize;
    while out.len() < outputs {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss,
            reason = "a count of samples of a test"
        )]
        let (x0, x1) = (
            ((call * 64) as f64 * rate).floor() as usize,
            (((call + 1) * 64) as f64 * rate).floor() as usize,
        );
        let input: Vec<f32> = (x0..x1).map(signal).collect();
        stretch.process(&[&input], &mut [&mut chunk[..]]);
        out.extend_from_slice(&chunk);
        call += 1;
    }
    out
}

#[test]
fn a_stretcher_whose_last_sound_was_silence_starts_again_as_a_new_one() {
    for (sample_rate, rate) in [(RATE, 0.7), (RATE, 1.3), (44_100, 1.0)] {
        let mut fresh = Stretch::new(1, sample_rate).unwrap();
        let mut used = Stretch::new(1, sample_rate).unwrap();
        // The last segment of `used` ended in long digital silence (the library's silence mode).
        let _ = segment(&mut used, sample_rate, usize::MAX, 48_000, rate);
        let a = segment(&mut fresh, sample_rate, 20_000, 96_000, rate);
        let b = segment(&mut used, sample_rate, 20_000, 96_000, rate);
        assert!(a.iter().any(|s| s.abs() > 0.1), "it sounds");
        let differs = a
            .iter()
            .zip(&b)
            .position(|(a, b)| a.to_bits() != b.to_bits());
        assert_eq!(differs, None, "{sample_rate} Hz at {rate}");
    }
}

#[test]
fn a_stretcher_takes_a_rate_of_8_khz_or_more() {
    assert_eq!(Stretch::new(1, 7_999).unwrap_err(), Error::Rate);
    assert_eq!(Stretch::new(1, 1).unwrap_err(), Error::Rate);
    assert!(Stretch::new(1, 8_000).is_ok());
}
