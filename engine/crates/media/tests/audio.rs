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
