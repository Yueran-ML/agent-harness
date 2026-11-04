//! The sound of the projects of before through the graph (plan T4b, "旧工程声音不变"): the mixer
//! of phases 1 and 2 (`server/src/play/mix.rs`, `block`, before it moved into the graph) is kept
//! here, as it was, as the oracle; 200 random projects of schema 3 (T1's
//! `samples::schema3_project`), read as schema 4, are mixed by both, in windows over their whole
//! timelines and about every cut and crossfade, from a sound of no more than ±1.
//!
//! Every sample is the same to the bit but where the old mixer summed the pieces of sound in
//! another order: it added every piece of every track to one sum, the graph adds the pieces of a
//! track into the track's sum first, and that into the bus. Where a track after the first sounds
//! two pieces at once (a crossfade), `((a + b) + c) + d` becomes `(a + b) + (c + d)`, and the
//! two differ by the rounding of an `f32` sum: at most (n − 1)·ε of the sum of the sizes of the
//! n pieces summed.

use std::ops::Range;

use columbina_audio::feed::FeedError;
use columbina_audio::graph::{Graph, Options, Purpose};
use columbina_model::samples::schema3_project;
use columbina_model::{resolve, Clip, Media, Project, Timeline};

type Stereo = [f32; 2];

// --- The oracle: `mix.rs` of phase 2, as it was --------------------------------------------

/// The old `block`, which also gives, at each sample, the sum of the sizes of what it summed and
/// how many pieces it summed.
fn block(
    timeline: &Timeline,
    project: &Project,
    from: i64,
    frames: usize,
    mut read: impl FnMut(&Clip, &Media, Range<i64>) -> Vec<Stereo>,
) -> (Vec<Stereo>, Vec<Stereo>, Vec<f32>) {
    let to = from + i64::try_from(frames).unwrap();
    let mut mixed = vec![[0.0; 2]; frames];
    let mut sizes = vec![[0.0; 2]; frames];
    let mut counts = vec![0.0; frames];
    for piece in resolve::audio_between(timeline, project, from, to) {
        let clip = timeline
            .tracks
            .iter()
            .find(|track| track.id == piece.track)
            .and_then(|track| track.clips.iter().find(|clip| clip.id == piece.clip))
            .unwrap();
        let media = project
            .media
            .iter()
            .find(|media| media.id == piece.media)
            .unwrap();
        let mapping = Mapping::new(clip, timeline);
        let (first, _) = mapping.at(piece.from);
        let (last, _) = mapping.at(piece.to - 1);
        let cutoff = (f64::from(clip.speed.den) / f64::from(clip.speed.num)).min(1.0);
        let radius = radius(cutoff);
        let range = first.min(last) - radius..first.max(last) + radius + 1;
        let pcm = read(clip, media, range.clone());
        let pan = pan(piece.pan);
        for sample in piece.from..piece.to {
            let (index, phase) = mapping.at(sample);
            let index = usize::try_from(index - range.start).unwrap();
            let value = resampled(&pcm, index, phase, cutoff);
            let gain = piece.gain.at(sample);
            let at = usize::try_from(sample - from).unwrap();
            let destination = &mut mixed[at];
            destination[0] += value[0] * gain * pan[0];
            destination[1] += value[1] * gain * pan[1];
            sizes[at][0] += (value[0] * gain * pan[0]).abs();
            sizes[at][1] += (value[1] * gain * pan[1]).abs();
            counts[at] += 1.0;
        }
    }
    for sample in mixed.iter_mut().flatten() {
        *sample = sample.clamp(-1.0, 1.0);
    }
    (mixed, sizes, counts)
}

struct Mapping {
    start: i128,
    step: i128,
    denominator: i128,
}

impl Mapping {
    fn new(clip: &Clip, timeline: &Timeline) -> Self {
        let source = clip.source_in().unwrap();
        let sample_at = |frame: i64| {
            i128::from(frame) * i128::from(timeline.sample_rate) * i128::from(timeline.rate.den)
                / i128::from(timeline.rate.num)
        };
        let direction = if clip.reverse { -1 } else { 1 };
        let origin = if clip.reverse {
            sample_at(clip.end()) - 1
        } else {
            -sample_at(clip.start)
        };
        let step = i128::from(clip.speed.num) * i128::from(source.scale);
        Self {
            start: i128::from(source.ticks)
                * i128::from(timeline.sample_rate)
                * i128::from(clip.speed.den)
                + origin * step,
            step: step * direction,
            denominator: i128::from(source.scale) * i128::from(clip.speed.den),
        }
    }

    #[allow(clippy::cast_precision_loss)]
    fn at(&self, sample: i64) -> (i64, f64) {
        let numerator = self.start + i128::from(sample) * self.step;
        let whole = numerator.div_euclid(self.denominator);
        let fraction = numerator.rem_euclid(self.denominator) as f64 / self.denominator as f64;
        (i64::try_from(whole).unwrap(), fraction)
    }
}

#[allow(clippy::cast_possible_truncation)]
fn pan(value: f32) -> Stereo {
    match value {
        -1.0 => [std::f32::consts::SQRT_2, 0.0],
        1.0 => [0.0, std::f32::consts::SQRT_2],
        _ => {
            let theta = (f64::from(value) + 1.0) * std::f64::consts::FRAC_PI_4;
            [
                (std::f64::consts::SQRT_2 * theta.cos()) as f32,
                (std::f64::consts::SQRT_2 * theta.sin()) as f32,
            ]
        }
    }
}

const CROSSINGS: i64 = 16;
const LONGEST: i64 = CROSSINGS * 32;

#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
fn radius(cutoff: f64) -> i64 {
    ((CROSSINGS as f64 / cutoff).ceil() as i64).clamp(CROSSINGS, LONGEST)
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::float_cmp
)]
fn resampled(pcm: &[Stereo], index: usize, phase: f64, cutoff: f64) -> Stereo {
    use std::f64::consts::PI;
    if phase == 0.0 && cutoff == 1.0 {
        return pcm[index];
    }
    let radius = radius(cutoff);
    let reach = radius as f64;
    let first = 1 - radius;
    let distance = first as f64 - phase;
    let (step_sin, step_cos) = (PI * cutoff).sin_cos();
    let (mut sin, mut cos) = (PI * cutoff * distance).sin_cos();
    let (window_step_sin, window_step_cos) = (PI / reach).sin_cos();
    let (mut window_sin, mut window_cos) = (PI * distance / reach).sin_cos();
    let mut sum = [0.0; 2];
    let mut weights = 0.0;
    for tap in first..=radius {
        let angle = PI * cutoff * (tap as f64 - phase);
        let sinc = if angle.abs() < 1e-12 {
            1.0
        } else {
            sin / angle
        };
        let weight = sinc * 0.5_f64.mul_add(window_cos, 0.5);
        let at = index
            .checked_add_signed(isize::try_from(tap).unwrap())
            .unwrap();
        sum[0] += f64::from(pcm[at][0]) * weight;
        sum[1] += f64::from(pcm[at][1]) * weight;
        weights += weight;
        (sin, cos) = (
            sin.mul_add(step_cos, cos * step_sin),
            cos.mul_add(step_cos, -sin * step_sin),
        );
        (window_sin, window_cos) = (
            window_sin.mul_add(window_step_cos, window_cos * window_step_sin),
            window_cos.mul_add(window_step_cos, -window_sin * window_step_sin),
        );
    }
    [(sum[0] / weights) as f32, (sum[1] / weights) as f32]
}

// --- The comparison --------------------------------------------------------------------------

/// The sound of media `media` at its sample `sample`, channel `channel`: noise within ±1, the
/// same for both mixers.
fn noise(media: &str, sample: i64, channel: usize) -> f32 {
    let mut z = sample.cast_unsigned().wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (channel as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9)
        ^ (media.len() as u64 + u64::from(media.as_bytes()[media.len() - 1])) << 48;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    #[allow(clippy::cast_precision_loss)]
    let value = (z >> 40) as f32 / (1_u64 << 23) as f32 - 1.0;
    value
}

/// The windows that are compared: from the start, about each cut and the middle of each clip,
/// and past the end.
fn windows(timeline: &Timeline) -> Vec<i64> {
    let samples = |frame: i64| resolve::samples_at(frame, timeline.rate, timeline.sample_rate);
    let mut starts = vec![0];
    for track in &timeline.tracks {
        for clip in &track.clips {
            starts.push(samples(clip.start) - 1500);
            starts.push(samples(clip.start + clip.length / 2));
            starts.push(samples(clip.end()) - 1500);
        }
    }
    starts.sort_unstable();
    starts.dedup();
    starts
}

#[test]
fn the_projects_of_before_sound_through_the_graph_as_they_did() {
    const WINDOW: usize = 3000;
    let (mut samples, mut differ, mut largest) = (0usize, 0usize, 0.0_f32);
    let mut projects_that_differ = 0;
    for seed in 0..200 {
        let (project, _) = Project::from_value_upgrading(schema3_project(seed)).unwrap();
        let timeline = &project.timelines[0];
        let mut graph = Graph::build(&project, 0, Options::new(1024, Purpose::Preview)).unwrap();
        assert_eq!(graph.latency(), 0);
        assert_eq!(graph.settle(), 0);
        let media: Vec<String> = graph.clips().iter().map(|c| c.media.clone()).collect();
        let mut feed = |clip: usize, range: Range<i64>, out: &mut [&mut [f32]]| {
            for (channel, out) in out.iter_mut().enumerate() {
                for (n, sample) in out.iter_mut().enumerate() {
                    *sample = noise(
                        &media[clip],
                        range.start + i64::try_from(n).unwrap(),
                        channel,
                    );
                }
            }
            Ok::<(), FeedError>(())
        };
        let monitor = graph.monitor().unwrap();
        let mut differs = false;
        for from in windows(timeline) {
            let (want, sizes, counts) =
                block(timeline, &project, from, WINDOW, |_, media, range| {
                    range
                        .map(|n| [noise(&media.id, n, 0), noise(&media.id, n, 1)])
                        .collect()
                });
            graph.start(from, &mut feed).unwrap();
            let mut got = vec![0.0; 2 * WINDOW];
            for (block, into) in got.chunks_mut(2 * 1000).enumerate() {
                let frames = into.len() / 2;
                graph.process(&mut feed, frames).unwrap();
                graph.write_device(monitor, frames, into);
                let _ = block;
            }
            for (n, ((want, size), count)) in want.iter().zip(&sizes).zip(&counts).enumerate() {
                for channel in 0..2 {
                    let got = got[2 * n + channel];
                    samples += 1;
                    if got.to_bits() == want[channel].to_bits() {
                        continue;
                    }
                    differ += 1;
                    differs = true;
                    let difference = (got - want[channel]).abs();
                    largest = largest.max(difference);
                    // Two orders of a sum of `count` terms differ by at most (count − 1)·ε of
                    // the sum of their sizes (Higham, "Accuracy and Stability", §4.2).
                    assert!(
                        difference <= (count - 1.0) * f32::EPSILON * size[channel],
                        "seed {seed}, sample {} of channel {channel}: {got} != {} (the sizes \
                         summed are {})",
                        from + i64::try_from(n).unwrap(),
                        want[channel],
                        size[channel]
                    );
                }
            }
        }
        projects_that_differ += usize::from(differs);
    }
    println!(
        "{samples} samples compared: {differ} differ by the order of a sum, in \
         {projects_that_differ} of 200 projects, by at most {largest:e}"
    );
    assert!(samples > 10_000_000);
}
