//! Sample-accurate timeline mixing. Media readers supply PCM at the timeline rate, in stereo;
//! the pure mixer maps clip time, resamples speed, applies gain/pan and sums before limiting.

mod reader;

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::sync::Arc;

use columbina_media::AudioFormat;
use columbina_model::{resolve, Clip, Media, Project, Timeline};
use reader::{Layout, Reader};

type Stereo = [f32; 2];

pub struct Mixer {
    project: Arc<Project>,
    timeline: usize,
    readers: BTreeMap<String, Reader>,
}

impl Mixer {
    pub fn new(project: Arc<Project>, timeline: usize) -> Self {
        Self {
            project,
            timeline,
            readers: BTreeMap::new(),
        }
    }

    pub fn block(&mut self, from: i64, frames: usize) -> Result<Vec<Stereo>, String> {
        let timeline = &self.project.timelines[self.timeline];
        let mut active = BTreeSet::new();
        let result = block(
            timeline,
            &self.project,
            from,
            frames,
            |clip, media, range| {
                active.insert(clip.id.clone());
                if !self.readers.contains_key(&clip.id) {
                    self.readers.insert(
                        clip.id.clone(),
                        Reader::new(
                            media,
                            timeline.sample_rate,
                            &Layout::of_clip(clip, 2).stereo(),
                        )?,
                    );
                }
                let pcm = self
                    .readers
                    .get_mut(&clip.id)
                    .expect("opened above")
                    .read(range)?;
                Ok(pcm.as_chunks::<2>().0.to_vec())
            },
        );
        self.readers.retain(|clip, _| active.contains(clip));
        result
    }

    pub fn source(self, format: AudioFormat, from: i64) -> Box<dyn crate::audio::Source> {
        Box::new(OutputSource {
            mixer: self,
            format,
            from,
            played: 0,
        })
    }
}

/// Converts the stereo timeline mix to the device rate/layout without restarting the
/// fractional resampling phase at each 20 ms output block.
struct OutputSource {
    mixer: Mixer,
    format: AudioFormat,
    from: i64,
    played: i64,
}

impl crate::audio::Source for OutputSource {
    #[allow(
        clippy::cast_precision_loss,
        reason = "the resampling phase is a fraction in [0, 1)"
    )]
    fn next(&mut self) -> Result<Option<Vec<f32>>, String> {
        let timeline = &self.mixer.project.timelines[self.mixer.timeline];
        let rate = timeline.sample_rate;
        let frames = self.format.rate / 50;
        let end = i128::from(resolve::duration(timeline))
            * i128::from(rate)
            * i128::from(timeline.rate.den)
            / i128::from(timeline.rate.num);
        let at = |played: i64| {
            let position = i128::from(self.from) * i128::from(self.format.rate)
                + i128::from(played) * i128::from(rate);
            (
                position.div_euclid(i128::from(self.format.rate)),
                position.rem_euclid(i128::from(self.format.rate)),
            )
        };
        let (first, _) = at(self.played);
        if first >= end {
            return Ok(None);
        }
        let (last, _) = at(self.played + i64::from(frames) - 1);
        let cutoff = (f64::from(self.format.rate) / f64::from(rate)).min(1.0);
        let radius = i128::from(radius(cutoff));
        let from = i64::try_from(first - radius)
            .map_err(|_| "output time is outside audio sample time")?;
        let to = i64::try_from(last + radius + 1)
            .map_err(|_| "output time is outside audio sample time")?;
        let pcm = self.mixer.block(
            from,
            usize::try_from(to - from).map_err(|_| "output audio window is too large")?,
        )?;
        let mut output = Vec::with_capacity(
            usize::try_from(frames).expect("device run") * usize::from(self.format.channels),
        );
        for offset in 0..frames {
            let (sample, remainder) = at(self.played + i64::from(offset));
            let index =
                usize::try_from(sample - i128::from(from)).expect("sample in output window");
            let stereo = resampled(
                &pcm,
                index,
                remainder as f64 / f64::from(self.format.rate),
                cutoff,
            );
            if self.format.channels == 1 {
                output.push(
                    ((stereo[0] + stereo[1]) * std::f32::consts::FRAC_1_SQRT_2).clamp(-1.0, 1.0),
                );
            } else {
                output.extend(stereo);
                output.extend(std::iter::repeat_n(
                    0.0,
                    usize::from(self.format.channels) - 2,
                ));
            }
        }
        self.played += i64::from(frames);
        Ok(Some(output))
    }
}

fn block(
    timeline: &Timeline,
    project: &Project,
    from: i64,
    frames: usize,
    mut read: impl FnMut(&Clip, &Media, Range<i64>) -> Result<Vec<Stereo>, String>,
) -> Result<Vec<Stereo>, String> {
    let to = from
        .checked_add(i64::try_from(frames).map_err(|_| "audio block is too long")?)
        .ok_or("audio block is outside sample time")?;
    let mut mixed = vec![[0.0; 2]; frames];
    for piece in resolve::audio_between(timeline, project, from, to) {
        let clip = timeline
            .tracks
            .iter()
            .find(|track| track.id == piece.track)
            .and_then(|track| track.clips.iter().find(|clip| clip.id == piece.clip))
            .ok_or("the resolved audio clip is missing")?;
        let media = project
            .media
            .iter()
            .find(|media| media.id == piece.media)
            .ok_or("the resolved audio media is missing")?;
        let mapping = Mapping::new(clip, timeline);
        let (first, _) = mapping.at(piece.from)?;
        let (last, _) = mapping.at(piece.to - 1)?;
        let cutoff = (f64::from(clip.speed.den) / f64::from(clip.speed.num)).min(1.0);
        let radius = radius(cutoff);
        let range = first
            .min(last)
            .checked_sub(radius)
            .ok_or("audio source time is too small")?
            ..first
                .max(last)
                .checked_add(radius + 1)
                .ok_or("audio source time is too large")?;
        let pcm = read(clip, media, range.clone())?;
        if i64::try_from(pcm.len()).ok() != range.end.checked_sub(range.start) {
            return Err("the audio reader returned the wrong number of samples".into());
        }
        let pan = pan(piece.pan);
        for sample in piece.from..piece.to {
            let (index, phase) = mapping.at(sample)?;
            let index =
                usize::try_from(index - range.start).expect("source window covers the piece");
            let value = resampled(&pcm, index, phase, cutoff);
            let gain = piece.gain.at(sample);
            let destination =
                &mut mixed[usize::try_from(sample - from).expect("piece is in the block")];
            destination[0] += value[0] * gain * pan[0];
            destination[1] += value[1] * gain * pan[1];
        }
    }
    // Limiting individual clips before addition would give a different mix.
    for sample in mixed.iter_mut().flatten() {
        *sample = sample.clamp(-1.0, 1.0);
    }
    Ok(mixed)
}

/// Absolute rational positions keep fractional speed/trim phase across audio block boundaries.
struct Mapping {
    start: i128,
    step: i128,
    denominator: i128,
}

impl Mapping {
    fn new(clip: &Clip, timeline: &Timeline) -> Self {
        let source = clip.source_in().expect("resolved media clip");
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

    #[allow(
        clippy::cast_precision_loss,
        reason = "only a rational fraction in [0, 1) is converted to f64"
    )]
    fn at(&self, sample: i64) -> Result<(i64, f64), String> {
        let numerator = self.start + i128::from(sample) * self.step;
        let whole = numerator.div_euclid(self.denominator);
        let fraction = numerator.rem_euclid(self.denominator) as f64 / self.denominator as f64;
        Ok((
            i64::try_from(whole).map_err(|_| "audio source is outside sample time")?,
            fraction,
        ))
    }
}

/// Equal-power pan, normalized so a centred stereo clip keeps its original level. Each channel
/// of the media is scaled by its own side's gain, so on stereo media pan is a balance: a hard
/// pan keeps one channel, louder by 3 dB, and drops the other.
#[allow(
    clippy::cast_possible_truncation,
    reason = "audio coefficients are f32"
)]
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

/// How many zero crossings of the sinc lie on each side of its centre, at full bandwidth.
const CROSSINGS: i64 = 16;

/// The most source samples on each side of the centre that one output sample reads: 32 times
/// the reach at full bandwidth, which keeps 16 zero crossings up to a speed of 32.
const LONGEST: i64 = CROSSINGS * 32;

/// How many source samples on each side of the centre the filter reads at `cutoff` (the new
/// band as a part of the old). The sinc is stretched as the band narrows, so that a speed-up
/// keeps as many zero crossings, and with them as deep a stopband, as playing at speed 1 does:
/// a 32-tap filter at 16 times would have barely two lobes and let much of what is above the
/// new Nyquist frequency fold back into what is heard.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    reason = "the reach is at most LONGEST"
)]
fn radius(cutoff: f64) -> i64 {
    ((CROSSINGS as f64 / cutoff).ceil() as i64).clamp(CROSSINGS, LONGEST)
}

/// Windowed sinc resampling low-passes a speed-up before decimation. Unity speed at an
/// integer position is exact, so the ordinary playback path does no filtering at all. The sine
/// and cosine of each tap are stepped from the last by a rotation, which a filter of a
/// thousand taps needs to be cheap.
#[allow(
    clippy::cast_possible_truncation,
    reason = "normalized audio samples are f32"
)]
#[allow(
    clippy::cast_precision_loss,
    reason = "filter taps are at most LONGEST from the centre"
)]
#[allow(
    clippy::float_cmp,
    reason = "the exact unity-rate, integer-phase case must preserve PCM samples"
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
    // The angle of the sinc and of the window at the first tap, each stepped by a rotation.
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
            .checked_add_signed(isize::try_from(tap).expect("filter taps fit an isize"))
            .expect("the source window includes filter support");
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

#[cfg(test)]
mod tests {
    use super::*;
    use columbina_model::{Alignment, Speed, Time, Track, TrackKind, Transition, TransitionKind};

    fn samples(range: Range<i64>, value: impl Fn(i64) -> Stereo) -> Vec<Stereo> {
        range.map(value).collect()
    }

    fn number(value: i64) -> f32 {
        f32::from(i16::try_from(value).unwrap())
    }

    #[test]
    fn two_tones_crossfade_sample_by_sample_at_equal_power() {
        let mut project = Project::sample();
        let timeline = &mut project.timelines[0];
        let first = Clip::of_media(
            "c1",
            "m1",
            Time {
                ticks: 0,
                scale: 15360,
            },
            0,
            50,
        );
        let second = Clip::of_media(
            "c2",
            "m1",
            Time {
                ticks: 30720,
                scale: 15360,
            },
            50,
            50,
        );
        let mut track = Track::new("A1", TrackKind::Audio).with_clips(vec![first, second]);
        track.transitions.push(Transition {
            id: "x1".into(),
            kind: TransitionKind::Crossfade,
            left: "c1".into(),
            right: "c2".into(),
            length: 10,
            align: Alignment::Center,
            fade_in_level_db: columbina_model::CrossfadeLevel::Plus3,
            fade_out_level_db: columbina_model::CrossfadeLevel::Plus3,
        });
        timeline.tracks = vec![track];
        let timeline = &project.timelines[0];
        let tone = |id: &str, sample: i64| {
            let phase = f64::from(i32::try_from(sample).unwrap());
            #[allow(clippy::cast_possible_truncation)]
            let value = if id == "c1" {
                0.2 * (phase * 0.007).sin()
            } else {
                0.3 * (phase * 0.009).cos()
            } as f32;
            [value, value]
        };
        let got = block(timeline, &project, 86400, 19200, |clip, _, range| {
            Ok(samples(range, |sample| tone(&clip.id, sample)))
        })
        .unwrap();
        for (i, pair) in got.iter().enumerate() {
            let sample = 86400 + i64::try_from(i).unwrap();
            let theta = number(i64::try_from(i).unwrap()) / 19200.0 * std::f32::consts::FRAC_PI_2;
            let expected =
                tone("c1", sample)[0] * theta.cos() + tone("c2", sample)[0] * theta.sin();
            for channel in pair {
                assert!(
                    (channel - expected).abs() < 1e-5,
                    "sample {sample}: {pair:?} != {expected}"
                );
            }
        }
    }

    #[test]
    fn gain_pan_muting_and_final_limiting_are_applied_to_the_mix() {
        let mut project = Project::sample();
        let track = &mut project.timelines[0].tracks[1];
        track.clips[0].gain_db = (-6.0).into();
        track.clips[0].pan = (-1.0).into();
        let read = |_: &Clip, _: &Media, range: Range<i64>| Ok(samples(range, |_| [0.25, 0.25]));
        let got = block(&project.timelines[0], &project, 0, 100, read).unwrap();
        let half = 0.125 * std::f32::consts::SQRT_2;
        assert!((got[50][0] - half).abs() < half * 0.01);
        assert_eq!(got[50][1], 0.0);
        project.timelines[0].tracks[1].enabled = false;
        let muted = block(&project.timelines[0], &project, 0, 100, |_, _, _| {
            panic!("disabled tracks must not decode")
        })
        .unwrap();
        assert_eq!(muted, vec![[0.0; 2]; 100]);
        project.timelines[0].tracks[1].enabled = true;
        project.timelines[0].tracks[1].clips[0].gain_db = 24.0.into();
        project.timelines[0].tracks[1].clips[0].pan = 0.0.into();
        let limited = block(&project.timelines[0], &project, 0, 100, |_, _, range| {
            Ok(samples(range, |_| [0.8, -0.8]))
        })
        .unwrap();
        assert_eq!(limited, vec![[1.0, -1.0]; 100]);

        // Opposing hot tracks must cancel before the final limiter, not after clipping each.
        let mut second = project.timelines[0].tracks[1].clone();
        second.id = "A2".into();
        second.clips[0].id = "other".into();
        project.timelines[0].tracks.push(second);
        let cancelled = block(&project.timelines[0], &project, 0, 100, |clip, _, range| {
            let value = if clip.id == "other" { -0.75 } else { 0.8 };
            Ok(samples(range, |_| [value, value]))
        })
        .unwrap();
        let expected = 0.05 * 10.0_f32.powf(24.0 / 20.0);
        assert!(cancelled
            .iter()
            .flatten()
            .all(|value| (value - expected).abs() < 1e-5));
    }

    #[test]
    fn reverse_fractional_speed_preserves_sample_phase_across_blocks() {
        let mut project = Project::sample();
        let clip = &mut project.timelines[0].tracks[1].clips[0];
        clip.speed = Speed { num: 3, den: 2 };
        clip.reverse = true;
        let read = |_: &Clip, _: &Media, range: Range<i64>| {
            Ok(samples(range, |sample| {
                #[allow(clippy::cast_possible_truncation)]
                let value = (f64::from(i32::try_from(sample).unwrap()) / 1_000_000.0) as f32;
                [value, -value]
            }))
        };
        let timeline = &project.timelines[0];
        let whole = block(timeline, &project, 101, 400, read).unwrap();
        let mut split = block(timeline, &project, 101, 91, read).unwrap();
        split.extend(block(timeline, &project, 192, 309, read).unwrap());
        assert_eq!(whole, split);
        for (i, pair) in whole.iter().enumerate() {
            let offset = 101 + i64::try_from(i).unwrap();
            #[allow(clippy::cast_possible_truncation)]
            let expected =
                (f64::from(i32::try_from(191_999 - offset).unwrap()) * 1.5 / 1_000_000.0) as f32;
            assert!(
                (pair[0] - expected).abs() < 1e-5,
                "sample {offset}: {pair:?} != {expected}"
            );
            assert_eq!(pair[1], -pair[0]);
        }
    }

    #[test]
    fn speed_up_filters_frequencies_above_the_new_nyquist_limit() {
        let mut project = Project::sample();
        project.timelines[0].tracks[1].clips[0].speed = Speed { num: 2, den: 1 };
        let got = block(&project.timelines[0], &project, 100, 100, |_, _, range| {
            Ok(samples(range, |n| {
                let value = if n % 2 == 0 { 0.5 } else { -0.5 };
                [value, value]
            }))
        })
        .unwrap();
        assert!(
            got.iter().flatten().all(|value| value.abs() < 0.002),
            "aliased Nyquist tone: {got:?}"
        );
    }

    #[test]
    fn pan_is_equal_power_between_the_hard_pans_and_keeps_its_sides() {
        let close = |got: Stereo, want: [f64; 2]| {
            for (got, want) in got.into_iter().zip(want) {
                assert!((f64::from(got) - want).abs() < 1e-6, "{got} != {want}");
            }
        };
        let sqrt2 = std::f64::consts::SQRT_2;
        let (sin, cos) = std::f64::consts::FRAC_PI_8.sin_cos();
        close(pan(0.0), [1.0, 1.0]);
        // Halfway to the left, the left side is the louder: cos and sin of an eighth of pi.
        close(pan(-0.5), [sqrt2 * cos, sqrt2 * sin]);
        close(pan(0.5), [sqrt2 * sin, sqrt2 * cos]);
        close(pan(-1.0), [sqrt2, 0.0]);
        close(pan(1.0), [0.0, sqrt2]);
        for value in [-0.9_f32, -0.5, -0.2, 0.0, 0.3, 0.5, 0.8] {
            let [left, right] = pan(value);
            assert!((left * left + right * right - 2.0).abs() < 1e-5, "{value}");
        }
    }

    #[test]
    fn a_half_pan_of_a_stereo_clip_is_a_balance_of_its_two_channels() {
        let mut project = Project::sample();
        project.timelines[0].tracks[1].clips[0].pan = 0.5.into();
        let read = |_: &Clip, _: &Media, range: Range<i64>| Ok(samples(range, |_| [0.2, 0.1]));
        let got = block(&project.timelines[0], &project, 0, 100, read).unwrap();
        let (sin, cos) = std::f64::consts::FRAC_PI_8.sin_cos();
        let sqrt2 = std::f64::consts::SQRT_2;
        let want = [0.2 * sqrt2 * sin, 0.1 * sqrt2 * cos];
        for pair in got {
            assert!((f64::from(pair[0]) - want[0]).abs() < 1e-6, "{pair:?}");
            assert!((f64::from(pair[1]) - want[1]).abs() < 1e-6, "{pair:?}");
        }
    }

    /// How loud a mix is: the root of its mean square, of the left channel.
    fn loudness(mix: &[Stereo]) -> f64 {
        let sum: f64 = mix.iter().map(|pair| f64::from(pair[0]).powi(2)).sum();
        #[allow(clippy::cast_precision_loss)]
        let count = mix.len() as f64;
        (sum / count).sqrt()
    }

    /// A clip at `speed` of a tone of `hertz` at 48 kHz and amplitude 0.5, mixed for 4800
    /// samples from sample 2400.
    fn sped(speed: u32, hertz: f64) -> Vec<Stereo> {
        let mut project = Project::sample();
        project.timelines[0].tracks[1].clips[0].speed = Speed { num: speed, den: 1 };
        block(
            &project.timelines[0],
            &project,
            2400,
            4800,
            |_, _, range| {
                Ok(samples(range, |n| {
                    let phase =
                        2.0 * std::f64::consts::PI * hertz * f64::from(i32::try_from(n).unwrap())
                            / 48000.0;
                    #[allow(clippy::cast_possible_truncation)]
                    let value = (0.5 * phase.sin()) as f32;
                    [value, value]
                }))
            },
        )
        .unwrap()
    }

    #[test]
    fn at_16_times_tones_above_the_new_nyquist_frequency_do_not_fold_back() {
        // At 16 times, what is above 1500 Hz in the media would be above 24 kHz when heard.
        let mut hertz = 2000.0;
        while hertz < 20000.0 {
            let heard = loudness(&sped(16, hertz));
            // A tone of amplitude 0.5 is 0.35 loud; folded back, it must be 40 dB quieter.
            assert!(heard < 0.0035, "{hertz} Hz at 16 times is {heard} loud");
            hertz *= 1.21;
        }
        // What is below the new Nyquist frequency is heard, as loud as it was.
        let heard = loudness(&sped(16, 600.0));
        assert!(
            (heard - 0.3535).abs() < 0.01,
            "600 Hz at 16 times is {heard} loud"
        );
        let heard = loudness(&sped(4, 3000.0));
        assert!(
            (heard - 0.3535).abs() < 0.01,
            "3000 Hz at 4 times is {heard} loud"
        );
    }

    fn real_project(sample: crate::test_media::Sample) -> Option<Project> {
        let made = crate::test_media::media(sample)?;
        let mut project = Project::sample();
        let facts = columbina_media::probe(&made.path).unwrap();
        project.media[0].path = made.path.to_string_lossy().into_owned();
        project.media[0].facts = facts.clone();
        let timeline = &mut project.timelines[0];
        timeline
            .tracks
            .retain(|track| track.kind == TrackKind::Audio);
        let clip = &mut timeline.tracks[0].clips[0];
        clip.source = columbina_model::Source::Media {
            media: "m1".into(),
            source_in: Time {
                ticks: 0,
                scale: facts.duration.scale,
            },
        };
        clip.length = facts.duration.ticks * 25 / i64::from(facts.duration.scale);
        Some(project)
    }

    #[test]
    fn real_pcm_reads_keep_sample_positions_across_windows_and_reverse_jumps() {
        let Some(project) = real_project(crate::test_media::Sample::Wav) else {
            return;
        };
        let mut decoder = columbina_media::AudioDecoder::open(
            std::path::Path::new(&project.media[0].path),
            Some(AudioFormat {
                rate: 48000,
                channels: 2,
            }),
        )
        .unwrap();
        let mut expected = Vec::new();
        while let Some(packet) = decoder.next().unwrap() {
            expected.extend(packet.data.as_chunks::<2>().0.iter().copied());
        }
        let mut mixer = Mixer::new(Arc::new(project), 0);
        for from in [0, 960, 12000, 60000, 59000, 2000, 0] {
            let actual = mixer.block(from, 960).unwrap();
            let at = usize::try_from(from).unwrap();
            assert_eq!(actual, &expected[at..at + 960], "PCM at {from}");
            assert_eq!(mixer.readers.len(), 1);
        }
    }

    #[test]
    fn real_mixing_keeps_the_video_origin_after_seeking_back_to_zero() {
        let Some(project) = real_project(crate::test_media::Sample::MkvOffsetAudio) else {
            return;
        };
        let mut mixer = Mixer::new(Arc::new(project), 0);
        for from in [0, 60000, 0] {
            let actual = mixer.block(from, 960).unwrap();
            for (sample, pair) in actual.iter().enumerate() {
                assert_eq!(
                    *pair,
                    [0.25, -0.75],
                    "sound before the picture leaked into {from}+{sample}"
                );
            }
        }
    }

    #[test]
    fn a_real_mix_converts_to_the_device_rate_and_front_channels() {
        let Some(project) = real_project(crate::test_media::Sample::Wav) else {
            return;
        };
        let project = Arc::new(project);
        let mut source = Mixer::new(Arc::clone(&project), 0).source(
            AudioFormat {
                rate: 44100,
                channels: 6,
            },
            0,
        );
        let mut played = Vec::new();
        for _ in 0..5 {
            played.extend(source.next().unwrap().unwrap());
        }
        assert_eq!(played.len(), 4410 * 6);
        for frame in played.as_chunks::<6>().0 {
            assert_eq!(frame[0], frame[1]);
            assert_eq!(frame[2..], [0.0; 4]);
        }
        // Same 440 Hz tone after sample-rate conversion; no zero or duplicated block.
        let left: Vec<f32> = played
            .as_chunks::<6>()
            .0
            .iter()
            .map(|frame| frame[0])
            .collect();
        let rises = left
            .windows(2)
            .filter(|pair| pair[0] <= 0.0 && pair[1] > 0.0)
            .count();
        assert!((43..=45).contains(&rises), "{rises} cycles in 100 ms");
        assert!(left.iter().any(|value| value.abs() > 0.08));
    }

    #[test]
    #[allow(clippy::float_cmp, reason = "a hard pan drops the other side exactly")]
    fn a_hard_pan_drops_the_other_side_exactly() {
        assert_eq!(pan(-1.0), [std::f32::consts::SQRT_2, 0.0]);
        assert_eq!(pan(1.0), [0.0, std::f32::consts::SQRT_2]);
    }

    #[test]
    #[allow(
        clippy::cast_precision_loss,
        clippy::float_cmp,
        reason = "taps and samples far within an f64; the case that is given back as it is"
    )]
    fn a_sample_between_samples_is_a_windowed_sinc_of_those_about_it() {
        use std::f64::consts::PI;
        let pcm: Vec<Stereo> = (0..2048_i32)
            .map(|n| {
                let n = f64::from(n);
                #[allow(clippy::cast_possible_truncation)]
                let pair = [
                    (0.4 * (n * 0.37).sin()) as f32,
                    (0.3 * (n * 1.91).cos()) as f32,
                ];
                pair
            })
            .collect();
        for cutoff in [1.0, 0.91875, 0.5, 1.0 / 16.0] {
            for phase in [0.0, 0.25, 0.5, 0.875] {
                if phase == 0.0 && cutoff == 1.0 {
                    continue;
                }
                let index = 1000;
                let reach = radius(cutoff);
                let (mut sum, mut weights) = ([0.0; 2], 0.0);
                for tap in 1 - reach..=reach {
                    let x = tap as f64 - phase;
                    let angle = PI * cutoff * x;
                    let sinc = if angle.abs() < 1e-12 {
                        1.0
                    } else {
                        angle.sin() / angle
                    };
                    let weight = sinc * (0.5 + 0.5 * (PI * x / reach as f64).cos());
                    let at = usize::try_from(index + tap).unwrap();
                    sum[0] += f64::from(pcm[at][0]) * weight;
                    sum[1] += f64::from(pcm[at][1]) * weight;
                    weights += weight;
                }
                let got = resampled(&pcm, 1000, phase, cutoff);
                for side in 0..2 {
                    let want = sum[side] / weights;
                    assert!(
                        (f64::from(got[side]) - want).abs() < 1e-5,
                        "{cutoff} at {phase}: {} != {want}",
                        got[side]
                    );
                }
            }
        }
    }

    /// The blocks that a mix of the sound of `project` played on a device of `format` from
    /// sample `from` gives, at most `most` of them, until it ends.
    fn played(
        project: &Arc<Project>,
        format: AudioFormat,
        from: i64,
        most: usize,
    ) -> Vec<Vec<f32>> {
        let mut source = Mixer::new(Arc::clone(project), 0).source(format, from);
        let mut blocks = vec![];
        while let Some(block) = source.next().unwrap() {
            blocks.push(block);
            if blocks.len() == most {
                break;
            }
        }
        blocks
    }

    #[test]
    fn a_real_mix_goes_on_from_block_to_block_and_ends_with_the_timeline() {
        let Some(project) = real_project(crate::test_media::Sample::Wav) else {
            return;
        };
        let project = Arc::new(project);
        let stereo = AudioFormat {
            rate: 44100,
            channels: 2,
        };
        // Blocks of 882 frames, 960 samples of the timeline: the second from 0 is the first from
        // 960.
        let from_start = played(&project, stereo, 0, 2);
        let from_second = played(&project, stereo, 960, 1);
        assert_eq!(from_start[1], from_second[0]);
        assert_ne!(from_start[0], from_start[1]);
        // A second from its end, of two, the timeline ends after 50 blocks.
        assert_eq!(played(&project, stereo, 48000, 60).len(), 50);
        // 59 frames at 30000/1001 a second end at sample 94494; from 48000, blocks start 960
        // samples apart.
        let mut ntsc = (*project).clone();
        ntsc.timelines[0].rate = columbina_model::Rate {
            num: 30000,
            den: 1001,
        };
        ntsc.timelines[0].tracks[0].clips[0].length = 59;
        assert_eq!(played(&Arc::new(ntsc), stereo, 48000, 60).len(), 49);
        // For one speaker, the two sides at equal power.
        let one = AudioFormat {
            rate: 44100,
            channels: 1,
        };
        let mono = played(&project, one, 0, 2);
        for (mono, stereo) in mono.iter().zip(&from_start) {
            assert_eq!(mono.len() * 2, stereo.len());
            for (one, pair) in mono.iter().zip(stereo.as_chunks::<2>().0) {
                let want = ((pair[0] + pair[1]) * std::f32::consts::FRAC_1_SQRT_2).clamp(-1.0, 1.0);
                assert!((one - want).abs() < 1e-6, "{one} != {want}");
            }
        }
        assert!(mono[0].iter().any(|value| value.abs() > 0.1));
        assert!(mono[0].iter().any(|value| *value < -0.1));
    }

    /// A WAV of 16 bits, stereo, at 48 kHz, of `frames` frames that `sample` gives.
    fn wav(path: &std::path::Path, frames: u32, sample: impl Fn(u32) -> i16) {
        let data = frames * 4;
        let mut bytes = Vec::with_capacity(44 + data as usize);
        bytes.extend(b"RIFF");
        bytes.extend((36 + data).to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16_u32.to_le_bytes());
        bytes.extend(1_u16.to_le_bytes());
        bytes.extend(2_u16.to_le_bytes());
        bytes.extend(48000_u32.to_le_bytes());
        bytes.extend((48000_u32 * 4).to_le_bytes());
        bytes.extend(4_u16.to_le_bytes());
        bytes.extend(16_u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend(data.to_le_bytes());
        for n in 0..frames {
            let value = sample(n).to_le_bytes();
            bytes.extend(value);
            bytes.extend(value);
        }
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn a_mix_played_on_a_device_of_a_lower_rate_drops_what_is_above_its_nyquist_frequency() {
        if crate::test_media::media(crate::test_media::Sample::Wav).is_none() {
            return;
        }
        // A tone at 24 kHz, the most that 48 kHz holds, is above the 22.05 kHz of 44.1 kHz.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("high.wav");
        wav(&path, 96000, |n| if n % 2 == 0 { 16000 } else { -16000 });
        let mut project = real_project(crate::test_media::Sample::Wav).unwrap();
        let facts = columbina_media::probe(&path).unwrap();
        project.media[0].path = path.to_string_lossy().into_owned();
        project.media[0].facts = facts;
        let project = Arc::new(project);
        let format = AudioFormat {
            rate: 44100,
            channels: 2,
        };
        let blocks = played(&project, format, 24000, 10);
        let left: Vec<f64> = blocks
            .iter()
            .flat_map(|block| {
                block
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|pair| f64::from(pair[0]))
            })
            .collect();
        #[allow(clippy::cast_precision_loss)]
        let loudness =
            (left.iter().map(|value| value * value).sum::<f64>() / left.len() as f64).sqrt();
        assert!(loudness < 0.05, "{loudness}");
    }
}
