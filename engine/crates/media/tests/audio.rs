//! The sound of the media that the tests make: every sample, how many there are, and where a
//! seek lands, at the rate of the file and converted to another rate and number of channels.

mod support;

use columbina_media::{AudioDecoder, AudioFormat, MediaError, Samples};
use support::{media, Sample};

/// Every sample from where the decoder is to the end, interleaved; checked to come in runs that
/// follow one another.
fn all(decoder: &mut AudioDecoder) -> (i64, Vec<f32>) {
    let channels = decoder.format().channels;
    let mut first = None;
    let mut next = None;
    let mut data = Vec::new();
    while let Some(Samples { start, data: run }) = decoder.next().unwrap() {
        assert_ne!(run.len(), 0);
        assert_eq!(run.len() % usize::from(channels), 0);
        if let Some(next) = next {
            assert_eq!(start, next, "the runs follow one another");
        }
        first.get_or_insert(start);
        next = Some(start + i64::try_from(run.len() / usize::from(channels)).unwrap());
        data.extend(run);
    }
    (first.unwrap(), data)
}

/// The samples of the sine of the test media: 440 Hz at 48 kHz, an eighth of full scale made
/// stereo (each channel at -3 dB).
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
fn sine(sample: i64) -> f32 {
    let wave = (std::f64::consts::TAU * 440.0 * sample as f64 / 48000.0).sin();
    (wave * std::f64::consts::FRAC_1_SQRT_2 / 8.0) as f32
}

/// The most that two runs of samples differ by, sample for sample.
fn most_apart(a: &[f32], b: &[f32]) -> f32 {
    a.iter()
        .zip(b)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0, f32::max)
}

#[test]
fn every_sample_of_the_wav_file_at_its_own_rate() {
    let Some(made) = media(Sample::Wav) else {
        return;
    };
    let mut decoder = AudioDecoder::open(&made.path, None).unwrap();
    let format = AudioFormat {
        rate: 48000,
        channels: 2,
    };
    assert_eq!(decoder.source(), format);
    assert_eq!(decoder.format(), format);
    let (start, data) = all(&mut decoder);
    assert_eq!(start, 0);
    // 2 s at 48 kHz, two channels alike.
    assert_eq!(data.len(), 2 * 96000);
    let left: Vec<f32> = data.iter().step_by(2).copied().collect();
    let right: Vec<f32> = data.iter().skip(1).step_by(2).copied().collect();
    assert_eq!(left, right);
    let wanted: Vec<f32> = (0..96000).map(sine).collect();
    // 16 bits: within a step of 1/32768, and a little for the sine of ffmpeg.
    let apart = most_apart(&left, &wanted);
    assert!(apart < 2e-4, "{apart}");
    assert!(decoder.next().unwrap().is_none());
}

#[test]
fn a_seek_in_the_wav_file_lands_on_the_sample_sought() {
    let Some(made) = media(Sample::Wav) else {
        return;
    };
    let mut decoder = AudioDecoder::open(&made.path, None).unwrap();
    let (_, whole) = all(&mut decoder);
    for sought in [48123, 0, 95_999, 7, 60_000] {
        decoder.seek(sought).unwrap();
        let (start, data) = all(&mut decoder);
        assert_eq!(start, sought);
        let from = usize::try_from(sought).unwrap() * 2;
        assert_eq!(data, whole[from..], "from {sought}");
    }
    // After its end there is nothing; before its start, silence up to it.
    decoder.seek(96_000).unwrap();
    assert!(decoder.next().unwrap().is_none());
    decoder.seek(-480).unwrap();
    let (start, data) = all(&mut decoder);
    assert_eq!(start, -480);
    assert!(data[..960].iter().all(|&sample| sample == 0.0));
    assert_eq!(data[960..], whole[..]);
}

#[test]
fn the_aac_sound_of_the_1080p_file_lasts_as_long_as_its_picture() {
    let Some(made) = media(Sample::Fhd) else {
        return;
    };
    let mut decoder = AudioDecoder::open(&made.path, None).unwrap();
    assert_eq!(
        decoder.source(),
        AudioFormat {
            rate: 48000,
            channels: 2
        }
    );
    let (start, data) = all(&mut decoder);
    let length = i64::try_from(data.len() / 2).unwrap();
    eprintln!("AAC: from sample {start}, {length} samples");
    assert_eq!(start, 0);
    // 4 s, give or take a frame of AAC (1024 samples) at the end.
    assert!((length - 192_000).abs() <= 1024, "{length}");
    let left: Vec<f32> = data.iter().step_by(2).copied().collect();
    // The sine, a frame in, as AAC keeps it.
    let wanted: Vec<f32> = (2048..2048 + 4800).map(sine).collect();
    let apart = most_apart(&left[2048..2048 + 4800], &wanted);
    assert!(apart < 5e-3, "{apart}");
}

/// The samples of a run that is most like `run` among those of `whole` from `around - reach` to
/// `around + reach`: the offset from `around` of the best, and how far it is from `run`.
fn best_place(whole: &[f32], run: &[f32], around: usize, reach: usize) -> (i64, f32) {
    (around - reach..=around + reach)
        .map(|at| {
            let offset = i64::try_from(at).unwrap() - i64::try_from(around).unwrap();
            (offset, most_apart(&whole[at..at + run.len()], run))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap()
}

/// The samples from where the decoder is to the end, of the first channel.
fn left_from(decoder: &mut AudioDecoder) -> (i64, Vec<f32>) {
    let channels = usize::from(decoder.format().channels);
    let (start, data) = all(decoder);
    (start, data.iter().step_by(channels).copied().collect())
}

#[test]
fn a_seek_in_aac_gives_the_samples_sought_from_the_first_as_decoding_from_the_start_does() {
    let (Some(noise), Some(fhd)) = (media(Sample::Noise), media(Sample::Fhd)) else {
        return;
    };
    // The sine of the 1080p file was coded with perceptual noise substitution (PNS), whose noise
    // the decoder makes up afresh after a seek: a little of it differs, by up to about 2e-4.
    for (made, most) in [(&noise, 1e-6), (&fhd, 1e-3)] {
        let mut decoder = AudioDecoder::open(&made.path, None).unwrap();
        let (_, whole) = all(&mut decoder);
        let length = i64::try_from(whole.len() / 2).unwrap();
        // At a frame of AAC (1024 samples), within one, near the start and near the end.
        for sought in [
            100_000,
            48_000,
            1024,
            5000,
            1,
            0,
            2047,
            length - 3000,
            77_777,
        ] {
            decoder.seek(sought).unwrap();
            let (start, data) = all(&mut decoder);
            assert_eq!(start, sought, "{}", made.path.display());
            let from = usize::try_from(sought).unwrap() * 2;
            let apart = most_apart(&data, &whole[from..]);
            assert_eq!(data.len(), whole.len() - from, "{sought}");
            assert!(apart <= most, "{}: {sought}: {apart}", made.path.display());
        }
    }
    // Noise tells a sample from those near it: a seek a sample off would be far off.
    let mut decoder = AudioDecoder::open(&noise.path, None).unwrap();
    assert_eq!(
        decoder.start().ticks,
        0,
        "the sound starts at time 0 of the file"
    );
    let (_, whole) = left_from(&mut decoder);
    decoder.seek(60_001).unwrap();
    let (_, sought) = left_from(&mut decoder);
    let (offset, apart) = best_place(&whole, &sought[..4800], 60_001, 8);
    assert_eq!((offset, apart), (0, 0.0));
    let (_, wrong) = best_place(&whole[1..], &sought[..4800], 60_001, 0);
    assert!(wrong > 0.01, "{wrong}");
}

#[test]
fn a_seek_in_aac_converted_to_another_rate_lands_on_the_sample_sought() {
    let Some(noise) = media(Sample::Noise) else {
        return;
    };
    let format = AudioFormat {
        rate: 44100,
        channels: 1,
    };
    let mut decoder = AudioDecoder::open(&noise.path, Some(format)).unwrap();
    let (_, whole) = left_from(&mut decoder);
    for sought in [44_100_usize, 3, 70_001] {
        decoder.seek(i64::try_from(sought).unwrap()).unwrap();
        let (start, data) = left_from(&mut decoder);
        assert_eq!(start, i64::try_from(sought).unwrap());
        // The converter starts afresh on the same grid of samples, before the pre-roll.
        let (offset, apart) = best_place(&whole, &data[..4410], sought, 8.min(sought));
        eprintln!("44.1 kHz from {sought}: {offset} samples off, {apart} apart");
        assert_eq!(offset, 0);
        assert!(apart <= 1e-6, "{apart}");
    }
}

#[test]
fn the_aac_sound_is_converted_to_another_rate_and_to_one_channel() {
    let Some(made) = media(Sample::Fhd) else {
        return;
    };
    let format = AudioFormat {
        rate: 44100,
        channels: 1,
    };
    let mut decoder = AudioDecoder::open(&made.path, Some(format)).unwrap();
    assert_eq!(decoder.format(), format);
    let (start, data) = all(&mut decoder);
    let length = i64::try_from(data.len()).unwrap();
    eprintln!("AAC at 44.1 kHz in mono: from sample {start}, {length} samples");
    assert!(start.abs() <= 1, "{start}");
    assert!((length - 176_400).abs() <= 1024, "{length}");
    // The two channels alike, mixed into one: the sine, at 44.1 kHz, as loud as each was
    // (swresample mixes stereo down to mono at -3 dB each: 2 × 0.707).
    #[allow(clippy::cast_precision_loss)]
    let peak = data[4410..4410 + 4410]
        .iter()
        .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
    assert!((0.12..0.2).contains(&peak), "{peak}");

    // Seeking at the new rate lands on its samples.
    decoder.seek(88_200).unwrap();
    assert_eq!(decoder.next().unwrap().unwrap().start, 88_200);

    // Up to six channels: the stereo is laid on the front left and right.
    let six = AudioFormat {
        rate: 48000,
        channels: 6,
    };
    let mut decoder = AudioDecoder::open(&made.path, Some(six)).unwrap();
    let run = decoder.next().unwrap().unwrap();
    assert_eq!(run.data.len() % 6, 0);
    let (_, data) = all(&mut decoder);
    let loud = |channel: usize| {
        data.iter()
            .skip(channel)
            .step_by(6)
            .fold(0.0_f32, |peak, sample| peak.max(sample.abs()))
    };
    assert!(loud(0) > 0.08 && loud(1) > 0.08, "{} {}", loud(0), loud(1));
    assert!(loud(3) < 1e-3, "no low frequencies: {}", loud(3));
}

#[test]
fn a_file_without_sound_has_no_sound_to_decode() {
    let Some(made) = media(Sample::Uhd) else {
        return;
    };
    let error = AudioDecoder::open(&made.path, None).err().unwrap();
    assert!(matches!(error, MediaError::NoStreams), "{error}");
}

/// How strong a frequency is in a run of samples at 48 kHz: the amplitude of the sine of
/// `hertz` that it holds, by the Goertzel filter (a bin of the DFT at any frequency).
#[allow(clippy::cast_precision_loss)]
fn amplitude(wave: &[f32], hertz: f64) -> f64 {
    let step = std::f64::consts::TAU * hertz / 48000.0;
    let coefficient = 2.0 * step.cos();
    let (mut before, mut last) = (0.0_f64, 0.0_f64);
    for &sample in wave {
        let next = f64::from(sample) + coefficient * last - before;
        before = last;
        last = next;
    }
    let power = last * last + before * before - coefficient * last * before;
    2.0 * power.max(0.0).sqrt() / wave.len() as f64
}

/// The frequency, in whole hertz from 20 to 1000, that is strongest in a second of samples at
/// 48 kHz: on a second, a whole number of hertz is a bin of its own.
fn strongest(wave: &[f32]) -> u32 {
    (20..=1000)
        .max_by(|&a, &b| amplitude(wave, f64::from(a)).total_cmp(&amplitude(wave, f64::from(b))))
        .unwrap()
}

/// A channel of samples interleaved `channels` an instant: a second of it, from 0.5 s.
fn second_of(data: &[f32], channels: usize, channel: usize) -> Vec<f32> {
    data.iter()
        .skip(channel)
        .step_by(channels)
        .skip(24000)
        .take(48000)
        .copied()
        .collect()
}

#[test]
fn the_six_channels_of_a_wav_in_5_1_are_given_each_as_it_is() {
    let Some(made) = media(Sample::SixSines) else {
        return;
    };
    let source = AudioDecoder::open(&made.path, None).unwrap();
    assert_eq!(
        source.format(),
        AudioFormat {
            rate: 48000,
            channels: 6
        }
    );
    // As the stream has them, mixed into the default layout of six (5.1) or not mixed at all.
    for mut decoder in [
        source,
        AudioDecoder::open_unmixed(&made.path, 0, None).unwrap(),
    ] {
        assert_eq!(decoder.format().channels, 6);
        let (start, data) = all(&mut decoder);
        assert_eq!(start, 0);
        assert_eq!(data.len(), 6 * 96000);
        for channel in 0..6 {
            let wave = second_of(&data, 6, channel);
            let hertz = 100 * (u32::try_from(channel).unwrap() + 1);
            assert_eq!(strongest(&wave), hertz, "channel {channel}");
            // -20 dBFS, in 16 bits.
            let level = amplitude(&wave, f64::from(hertz));
            assert!((level - 0.1).abs() < 1e-3, "channel {channel}: {level}");
        }
    }
}

#[test]
fn a_layout_that_is_not_the_default_of_its_count_is_given_unmixed() {
    let Some(made) = media(Sample::ThreeZero) else {
        return;
    };
    let mut decoder = AudioDecoder::open_unmixed(&made.path, 0, None).unwrap();
    assert_eq!(
        decoder.format(),
        AudioFormat {
            rate: 48000,
            channels: 3
        }
    );
    let (_, data) = all(&mut decoder);
    // L, R and C, each alone in its channel, as the file has them.
    for (channel, hertz) in [(0, 440), (1, 660), (2, 300)] {
        let wave = second_of(&data, 3, channel);
        let level = amplitude(&wave, f64::from(hertz));
        assert!((level - 0.25).abs() < 1e-3, "channel {channel}: {level}");
        for other in [440, 660, 300] {
            if other != hertz {
                let leak = amplitude(&wave, f64::from(other));
                assert!(leak < 1e-5, "{other} Hz in channel {channel}: {leak}");
            }
        }
    }
}

#[test]
fn unmixed_sound_is_converted_to_another_rate_and_seeks_to_its_samples() {
    let Some(made) = media(Sample::ThreeZero) else {
        return;
    };
    let mut decoder = AudioDecoder::open_unmixed(&made.path, 0, Some(44100)).unwrap();
    assert_eq!(
        decoder.format(),
        AudioFormat {
            rate: 44100,
            channels: 3
        }
    );
    let (_, whole) = all(&mut decoder);
    assert!(
        (whole.len() / 3).abs_diff(88200) <= 64,
        "{}",
        whole.len() / 3
    );
    decoder.seek(44_100).unwrap();
    let (start, data) = all(&mut decoder);
    assert_eq!(start, 44_100);
    let apart = most_apart(&data[..3 * 4410], &whole[3 * 44_100..3 * (44_100 + 4410)]);
    assert!(apart <= 1e-6, "{apart}");
}

#[test]
fn a_stream_of_sound_is_opened_by_its_index() {
    let Some(made) = media(Sample::TwoMonoStreams) else {
        return;
    };
    let mono = AudioFormat {
        rate: 48000,
        channels: 1,
    };
    // The best stream is the first; each is opened as it is, or unmixed.
    let tone = |decoder: &mut AudioDecoder| {
        let (start, data) = all(decoder);
        assert_eq!(start, 0);
        assert_eq!(data.len(), 96000);
        strongest(&second_of(&data, 1, 0))
    };
    assert_eq!(
        tone(&mut AudioDecoder::open(&made.path, None).unwrap()),
        440
    );
    for stream in [0, 1] {
        let hertz = 440 * (u32::try_from(stream).unwrap() + 1);
        let mut decoder = AudioDecoder::open_stream(&made.path, stream, None).unwrap();
        assert_eq!(decoder.source(), mono);
        assert_eq!(tone(&mut decoder), hertz, "stream {stream}");
        let mut unmixed = AudioDecoder::open_unmixed(&made.path, stream, None).unwrap();
        assert_eq!(unmixed.format(), mono);
        assert_eq!(tone(&mut unmixed), hertz, "stream {stream}, unmixed");
    }
    // In stereo, the second stream is laid on both sides.
    let stereo = AudioFormat {
        rate: 48000,
        channels: 2,
    };
    let mut decoder = AudioDecoder::open_stream(&made.path, 1, Some(stereo)).unwrap();
    assert_eq!(decoder.format(), stereo);
    let (_, data) = all(&mut decoder);
    assert_eq!(strongest(&second_of(&data, 2, 1)), 880);
    // A seek in the second stream lands where it does in the first.
    let mut decoder = AudioDecoder::open_stream(&made.path, 1, None).unwrap();
    let (_, whole) = all(&mut decoder);
    for sought in [48_123, 7, 0, 90_000] {
        decoder.seek(sought).unwrap();
        let (start, data) = all(&mut decoder);
        assert_eq!(start, sought);
        assert_eq!(
            data,
            whole[usize::try_from(sought).unwrap()..],
            "from {sought}"
        );
    }
    // There is no third stream.
    for stream in [2, 99] {
        let error = AudioDecoder::open_stream(&made.path, stream, None)
            .err()
            .unwrap();
        assert!(matches!(error, MediaError::NoStreams), "{error}");
        let error = AudioDecoder::open_unmixed(&made.path, stream, None)
            .err()
            .unwrap();
        assert!(matches!(error, MediaError::NoStreams), "{error}");
    }
}

#[test]
fn a_stream_of_pictures_is_not_opened_as_sound() {
    let Some(made) = media(Sample::Fhd) else {
        return;
    };
    let error = AudioDecoder::open_stream(&made.path, 0, None)
        .err()
        .unwrap();
    assert!(matches!(error, MediaError::NoStreams), "{error}");
    assert!(AudioDecoder::open_stream(&made.path, 1, None).is_ok());
}

#[test]
fn mono_made_stereo_by_swresample_is_3_db_down_on_each_side() {
    let Some(made) = media(Sample::Mono) else {
        return;
    };
    let mut mono = AudioDecoder::open(&made.path, None).unwrap();
    assert_eq!(mono.format().channels, 1);
    let (_, alone) = all(&mut mono);
    let mut stereo = AudioDecoder::open(
        &made.path,
        Some(AudioFormat {
            rate: 48000,
            channels: 2,
        }),
    )
    .unwrap();
    let (_, both) = all(&mut stereo);
    let level = amplitude(&second_of(&alone, 1, 0), 440.0);
    for side in 0..2 {
        let made = amplitude(&second_of(&both, 2, side), 440.0);
        let db = 20.0 * (made / level).log10();
        assert!((db + 3.0103).abs() < 0.01, "side {side}: {db} dB");
    }
}

#[test]
fn the_sound_of_a_file_starts_where_its_decoder_says() {
    for sample in [
        Sample::Wav,
        Sample::Fhd,
        Sample::MkvLateStart,
        Sample::TwoMonoStreams,
    ] {
        let Some(made) = media(sample) else {
            continue;
        };
        let decoder = AudioDecoder::open(&made.path, None).unwrap();
        assert_eq!(
            columbina_media::sound_start(&made.path).unwrap(),
            decoder.start(),
            "{sample:?}"
        );
    }
    if let Some(made) = media(Sample::Uhd) {
        let error = columbina_media::sound_start(&made.path).err().unwrap();
        assert!(matches!(error, MediaError::NoStreams), "{error}");
    }
}
