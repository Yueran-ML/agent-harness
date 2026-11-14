//! The mix made into the sound of the device as a stream (plan T5, `src/stream.rs`): the same
//! frames however the mix is handed over and the frames taken, and the very frames that the
//! window of phases 1 and 2 (T4b's `OutputSource`) made of the same mix.

use columbina_audio::graph::{radius, resampled};
use columbina_audio::stream::Resampler;
use columbina_audio::testing::Rng;

/// The frames that `OutputSource` made: the whole mix at hand from sample `base`, each frame
/// `played` at the position `start · device + played · rate`, resampled from the window, laid on
/// the channels of the device and clipped.
#[allow(clippy::cast_precision_loss, reason = "a fraction in [0, 1)")]
fn window_of_phase_2(
    mix: &[Vec<f32>; 2],
    base: i64,
    (start, frames): (i64, usize),
    (rate, device, channels): (u32, u32, usize),
) -> Vec<f32> {
    let cutoff = (f64::from(device) / f64::from(rate)).min(1.0);
    let window = [mix[0].as_slice(), mix[1].as_slice()];
    let mut output = Vec::new();
    for played in 0..frames {
        let position = i128::from(start) * i128::from(device)
            + i128::try_from(played).unwrap() * i128::from(rate);
        let sample = position.div_euclid(i128::from(device));
        let remainder = position.rem_euclid(i128::from(device));
        let index = usize::try_from(sample - i128::from(base)).unwrap();
        let mut stereo = [0.0; 2];
        resampled(
            &window,
            index,
            remainder as f64 / f64::from(device),
            cutoff,
            &mut stereo,
        );
        if channels == 1 {
            output
                .push(((stereo[0] + stereo[1]) * std::f32::consts::FRAC_1_SQRT_2).clamp(-1.0, 1.0));
        } else {
            output.extend(stereo.map(|side| side.clamp(-1.0, 1.0)));
            output.extend(std::iter::repeat_n(0.0, channels - 2));
        }
    }
    output
}

/// A mix of `frames` samples a side, now and then above 1, so that the clipping shows.
fn mix(seed: u64, frames: usize) -> [Vec<f32>; 2] {
    let mut rng = Rng::new(seed);
    let mut side = || -> Vec<f32> { (0..frames).map(|_| rng.signed() * 1.3).collect() };
    [side(), side()]
}

/// The frames of `resampler` from sample `start`, the mix handed over in pieces of `give()` and
/// the frames taken in pieces of `take()`, until `frames` are out.
fn streamed(
    resampler: &mut Resampler,
    mix: &[Vec<f32>; 2],
    base: i64,
    (start, frames): (i64, usize),
    give: &mut dyn FnMut() -> usize,
    take: &mut dyn FnMut() -> usize,
) -> Vec<f32> {
    resampler.start(start);
    let channels = resampler.channels();
    let mut handed = usize::try_from(resampler.first_input() - base).unwrap();
    let mut out = Vec::new();
    let mut run = vec![0.0; 4096 * channels];
    while out.len() < frames * channels {
        let count = give().min(resampler.room()).min(mix[0].len() - handed);
        let taken = resampler.push(
            &mix[0][handed..handed + count],
            &mix[1][handed..handed + count],
        );
        assert_eq!(taken, count);
        handed += taken;
        loop {
            let wanted = take().min(frames - out.len() / channels);
            let got = resampler.pull(&mut run[..wanted * channels]);
            out.extend_from_slice(&run[..got * channels]);
            if got < wanted || out.len() == frames * channels {
                break;
            }
        }
    }
    out
}

#[test]
fn the_stream_gives_the_frames_of_the_window_of_phase_2_however_it_is_cut() {
    for (rate, device) in [
        (48_000, 44_100),
        (44_100, 48_000),
        (48_000, 48_000),
        (96_000, 48_000),
        (48_000, 96_000),
    ] {
        for channels in [1, 2, 6] {
            let frames = 6_000;
            let reach = radius((f64::from(device) / f64::from(rate)).min(1.0));
            let mix = mix(u64::from(rate) + u64::from(device), 20_000);
            // The mix from sample 1000 - reach - 1 of the timeline; the run from 1000.
            let base = 1_000 - reach - 1;
            let start = 1_000;
            let wanted = window_of_phase_2(&mix, base, (start, frames), (rate, device, channels));
            for seed in 0..3 {
                let mut rng = Rng::new(seed);
                let mut cuts = Rng::new(seed + 100);
                let mut resampler = Resampler::new(rate, device, channels, 1024);
                let got = streamed(
                    &mut resampler,
                    &mix,
                    base,
                    (start, frames),
                    &mut || rng.range(1, 1024),
                    &mut || cuts.range(1, 700),
                );
                assert!(
                    got.iter()
                        .zip(&wanted)
                        .all(|(got, wanted)| got.to_bits() == wanted.to_bits()),
                    "{rate} to {device}, {channels} channels, cut by {seed}"
                );
                assert_eq!(got.len(), wanted.len());
            }
        }
    }
}

#[test]
#[allow(clippy::float_cmp, reason = "the samples pass as they are")]
fn at_the_same_rate_the_mix_passes_as_it_is_clipped_and_without_delay() {
    let mix = mix(7, 1_000);
    let mut resampler = Resampler::new(48_000, 48_000, 2, 256);
    assert_eq!(resampler.reach(), 0);
    resampler.start(500);
    assert_eq!(resampler.first_input(), 500);
    assert_eq!(resampler.push(&mix[0][..100], &mix[1][..100]), 100);
    let mut out = vec![0.0; 2 * 200];
    // Each sample handed over is a frame out at once.
    assert_eq!(resampler.pull(&mut out), 100);
    for n in 0..100 {
        assert_eq!(out[2 * n], mix[0][n].clamp(-1.0, 1.0));
        assert_eq!(out[2 * n + 1], mix[1][n].clamp(-1.0, 1.0));
    }
    assert!(out[..200]
        .iter()
        .any(|&sample| sample == 1.0 || sample == -1.0));
    assert_eq!(resampler.next_input(), 600);
    assert_eq!(resampler.next_output(), 600);
}

#[test]
fn it_takes_no_more_than_it_has_room_for_and_frees_what_is_read() {
    let mix = mix(9, 10_000);
    let mut resampler = Resampler::new(48_000, 44_100, 2, 512);
    resampler.start(0);
    let room = resampler.room();
    assert!(room >= 512);
    assert_eq!(
        resampler.push(&mix[0][..room + 50], &mix[1][..room + 50]),
        room
    );
    // Full but for the first sample, which the first frame does not reach.
    assert!(resampler.room() <= 1, "{}", resampler.room());
    let mut out = vec![0.0; 2 * 300];
    let got = resampler.pull(&mut out);
    assert!(got > 0);
    // What the next frame no longer reaches is room again.
    assert!(
        resampler.room() >= got * 48_000 / 44_100 - 1,
        "{}",
        resampler.room()
    );
    assert!(resampler.most_out(512) >= 512 * 44_100 / 48_000);
}
