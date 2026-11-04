//! The transform of Fourier of real signals: against the plain sum of its definition, and there
//! and back.

use columbina_audio::fft::{Complex32, RealFft};
use columbina_audio::testing::noise;
use proptest::prelude::*;

/// The spectrum of `signal` by the sum of the definition, in `f64`.
#[allow(clippy::cast_precision_loss, reason = "the indices of a transform")]
fn naive(signal: &[f32]) -> Vec<(f64, f64)> {
    let n = signal.len();
    (0..=n / 2)
        .map(|k| {
            signal
                .iter()
                .enumerate()
                .fold((0.0, 0.0), |(re, im), (t, &x)| {
                    let angle = -std::f64::consts::TAU * (k * t % n) as f64 / n as f64;
                    (
                        re + f64::from(x) * angle.cos(),
                        im + f64::from(x) * angle.sin(),
                    )
                })
        })
        .collect()
}

#[test]
fn the_transform_of_2048_points_is_the_sum_of_its_definition() {
    let signal = noise(42, 1.0, 2_048);
    let expected = naive(&signal);
    let mut fft = RealFft::new(2_048);
    assert_eq!(fft.size(), 2_048);
    assert_eq!(fft.bins(), 1_025);
    let mut input = signal.clone();
    let mut spectrum = fft.spectrum();
    fft.forward(&mut input, &mut spectrum);
    let error = spectrum
        .iter()
        .zip(&expected)
        .map(|(got, (re, im))| (f64::from(got.re) - re).hypot(f64::from(got.im) - im))
        .fold(0.0, f64::max);
    assert!(error <= 1e-4, "{error}");
}

#[test]
fn there_and_back_is_the_signal_again() {
    let signal = noise(7, 1.0, 2_048);
    let mut fft = RealFft::new(2_048);
    let mut input = signal.clone();
    let mut spectrum = fft.spectrum();
    fft.forward(&mut input, &mut spectrum);
    let mut back = fft.signal();
    fft.inverse(&mut spectrum, &mut back);
    let error = signal
        .iter()
        .zip(&back)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0, f32::max);
    assert!(error <= 1e-6, "{error}");
}

#[test]
fn a_cosine_on_a_bin_is_that_bin_alone() {
    let size = 1_024;
    let mut fft = RealFft::new(size);
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        reason = "a test"
    )]
    let mut signal: Vec<f32> = (0..size)
        .map(|n| (std::f64::consts::TAU * 10.0 * n as f64 / size as f64).cos() as f32)
        .collect();
    let mut spectrum = fft.spectrum();
    fft.forward(&mut signal, &mut spectrum);
    assert!((spectrum[10].re - 512.0).abs() < 1e-3);
    let others = spectrum.iter().enumerate().filter(|&(k, _)| k != 10);
    assert!(others.map(|(_, bin)| bin.norm()).fold(0.0, f32::max) < 1e-3);
}

#[test]
fn the_inverse_takes_the_edges_of_the_spectrum_as_real() {
    let mut fft = RealFft::new(8);
    let mut spectrum = vec![Complex32::new(8.0, 5.0)];
    spectrum.extend(std::iter::repeat_n(Complex32::default(), 3));
    spectrum.push(Complex32::new(0.0, -3.0));
    let mut signal = fft.signal();
    fft.inverse(&mut spectrum, &mut signal);
    assert!(signal.iter().all(|&x| (x - 1.0).abs() < 1e-6), "{signal:?}");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    /// There and back at any length, even or odd.
    #[test]
    fn there_and_back_at_any_length(size in 1_usize..3_000, seed in any::<u64>()) {
        let signal = noise(seed, 1.0, size);
        let mut fft = RealFft::new(size);
        let mut input = signal.clone();
        let mut spectrum = fft.spectrum();
        fft.forward(&mut input, &mut spectrum);
        let mut back = fft.signal();
        fft.inverse(&mut spectrum, &mut back);
        let error = signal.iter().zip(&back).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
        prop_assert!(error <= 1e-5, "{}", error);
    }
}
