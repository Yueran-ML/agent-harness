//! Real DSP readings and their reset state, independent of the server/UI.
use columbina_audio::fx::{denoise, width};
use columbina_audio::testing::{Run, render};
use columbina_audio::{Layout, Node, Spec};

#[test]
fn width_scope_is_actual_left_and_right_output() {
    let mut node = width::StereoWidth::new(&width::Settings {
        on: true,
        width: 1.0,
        diffusion: 0.0,
        sparkle: 0.0,
    });
    let read = node.scope();
    let spec = Spec::new(48000, 4096, Layout::Stereo);
    node.prepare(&spec).unwrap();
    let run = Run::new(vec![vec![0.25; 4096], vec![-0.125; 4096]]);
    render(&mut node, &spec, &run, |left| left);
    let values = read.read();
    assert!((values[126] - 0.25).abs() < 1e-6);
    assert!((values[127] + 0.125).abs() < 1e-6);
    assert!((values[128] - 20.0 * 0.25_f32.log10()).abs() < 1e-5);
    node.reset();
    assert!(read.read()[..128].iter().all(|v| *v == 0.0));
}

#[test]
fn nr_reads_signal_power_and_real_noise_estimate_and_clears_on_reset() {
    let mut node = denoise::Spectral::new(&denoise::Settings::default());
    let read = node.spectrum();
    let spec = Spec::new(48000, 4096, Layout::Stereo);
    node.prepare(&spec).unwrap();
    let tone: Vec<f32> = (0..48000)
        .map(|i| (std::f32::consts::TAU * 1000.0 * i as f32 / 48000.0).sin() * 0.2)
        .collect();
    render(
        &mut node,
        &spec,
        &Run::new(vec![tone.clone(), tone]),
        |left| left.min(4096),
    );
    let values = read.read();
    assert!(values.iter().all(|v| v.is_finite()));
    let strongest = values[..128]
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .unwrap()
        .0;
    assert!((4..=6).contains(&strongest), "{strongest}");
    assert!(values[128..256].iter().any(|v| *v > -150.0));
    assert!((values[256] - 20.0 * 0.2_f32.log10()).abs() < 0.05);
    node.reset();
    assert_eq!(read.read(), [-200.0; 258]);
}
