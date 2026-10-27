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

#[test]
fn a_seek_in_the_aac_sound_lands_within_a_frame_of_aac() {
    let Some(made) = media(Sample::Fhd) else {
        return;
    };
    let mut decoder = AudioDecoder::open(&made.path, None).unwrap();
    let (_, whole) = all(&mut decoder);
    let left: Vec<f32> = whole.iter().step_by(2).copied().collect();
    for sought in [100_000_usize, 48_000, 150_017, 5000] {
        decoder.seek(i64::try_from(sought).unwrap()).unwrap();
        let first = decoder.next().unwrap().unwrap();
        assert_eq!(first.start, i64::try_from(sought).unwrap());
        let (_, rest) = all(&mut decoder);
        let mut data = first.data;
        data.extend(rest);
        let run: Vec<f32> = data
            .iter()
            .step_by(2)
            .skip(2048)
            .take(4800)
            .copied()
            .collect();
        // A frame after the seek, the samples are those of the whole at the same place: the
        // first frame decoded after a seek lacks the frame before it, which AAC overlaps.
        let (offset, apart) = best_place(&left, &run, sought + 2048, 1024);
        eprintln!("sought {sought}: best {offset} samples off, {apart} apart");
        assert!(offset.abs() <= 1024);
        assert_eq!(offset, 0, "{apart}");
        assert!(apart < 1e-3, "{apart}");
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
