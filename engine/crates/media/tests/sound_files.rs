//! Sound written alone, into WAV and MPEG-4 audio, and as PCM beside a picture in `QuickTime`
//! (plan of phase 3, T6), and read back: PCM of floats to the bit, of 24 bits to its step, the
//! speakers of the channels as the mask says, and AAC at its pitch and its rate of bits.

use std::path::Path;

use columbina_media::{
    probe, AudioCodec, AudioDecoder, AudioSettings, Codec, Picture, Quality, Rate, Samples,
    SoundWriter, VideoSettings, Writer,
};

/// Every sample of the first stream of sound of `path`, in its own channels and order.
fn read_back(path: &Path) -> Vec<f32> {
    let facts = probe(path).unwrap();
    let stream = facts.audio.as_ref().unwrap().streams.as_ref().unwrap()[0].index;
    let mut decoder = AudioDecoder::open_unmixed(path, stream as usize, None).unwrap();
    let mut data = Vec::new();
    while let Some(Samples { data: run, .. }) = decoder.next().unwrap() {
        data.extend(run);
    }
    data
}

/// `frames` instants of `channels` channels, each channel its own tone, louder than 1 in places
/// where `hot`.
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
fn tones(frames: usize, channels: usize, hot: bool) -> Vec<f32> {
    (0..frames)
        .flat_map(|n| {
            (0..channels).map(move |channel| {
                let phase =
                    2.0 * std::f64::consts::PI * (200.0 + 100.0 * channel as f64) * n as f64
                        / 48000.0;
                let size = if hot { 1.5 } else { 0.7 };
                (size * phase.sin()) as f32 + 1e-7 * channel as f32
            })
        })
        .collect()
}

/// `count` samples of white noise of −0.5 to 0.5, the same each time.
#[allow(clippy::cast_precision_loss)]
fn noise(count: usize) -> Vec<f32> {
    let mut state = 0x2545_f491_u32;
    (0..count)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state as f32 / u32::MAX as f32 - 0.5
        })
        .collect()
}

#[test]
fn floats_written_into_a_wav_are_read_back_to_the_bit_hot_ones_too() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("mix.wav.part");
    // 5.1 as SMPTE keeps it and WAV says it: FL FR FC LFE BL BR.
    let settings = AudioSettings {
        rate: 96000,
        channels: 6,
        mask: 0x3F,
        codec: AudioCodec::PcmF32,
    };
    let mut writer = SoundWriter::create(&file, settings).unwrap();
    assert_eq!(writer.encoder(), "pcm_f32le");
    let given = tones(10_007, 6, true);
    for run in given.chunks(6 * 777) {
        writer.audio(run).unwrap();
    }
    writer.finish().unwrap();
    let facts = probe(&file).unwrap();
    let audio = facts.audio.as_ref().unwrap();
    assert_eq!((audio.rate, audio.channels), (96000, 6));
    assert_eq!(audio.codec, "pcm_f32le");
    assert_eq!(audio.layout.as_deref(), Some("5.1"), "{audio:?}");
    let read = read_back(&file);
    assert_eq!(read.len(), given.len());
    assert!(
        read.iter()
            .zip(&given)
            .all(|(a, b)| a.to_bits() == b.to_bits()),
        "floats are written as they are, over 1 too"
    );
    // The surrounds at the sides, as the mask says, and not libav's default for six.
    let side = directory.path().join("side.wav");
    let mut writer = SoundWriter::create(
        &side,
        AudioSettings {
            mask: 0x60F,
            ..settings
        },
    )
    .unwrap();
    writer.audio(&given).unwrap();
    writer.finish().unwrap();
    let audio = probe(&side).unwrap().audio.unwrap();
    assert_eq!(audio.layout.as_deref(), Some("5.1(side)"), "{audio:?}");
}

#[test]
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
fn samples_of_24_bits_are_rounded_to_their_step_and_clipped() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("mix.wav");
    let settings = AudioSettings {
        channels: 1,
        codec: AudioCodec::PcmS24,
        ..AudioSettings::default()
    };
    let mut writer = SoundWriter::create(&file, settings).unwrap();
    let step = 1.0 / 8_388_608.0_f64;
    let given: Vec<f32> = vec![
        0.0,
        0.5,
        -0.5,
        1.0,
        -1.0,
        1.5,
        -1.5,
        // Below half a step, and above it.
        (0.4 * step) as f32,
        (0.6 * step) as f32,
        f32::NAN,
    ];
    writer.audio(&given).unwrap();
    writer.finish().unwrap();
    let audio = probe(&file).unwrap().audio.unwrap();
    assert_eq!((audio.codec.as_str(), audio.channels), ("pcm_s24le", 1));
    let read: Vec<f64> = read_back(&file).into_iter().map(f64::from).collect();
    let largest = 1.0 - step;
    let wanted = [0.0, 0.5, -0.5, largest, -1.0, largest, -1.0, 0.0, step, 0.0];
    assert_eq!(read.len(), wanted.len());
    for (got, want) in read.iter().zip(wanted) {
        assert!((got - want).abs() < step / 4.0, "{got} is not {want}");
    }
}

#[test]
fn aac_alone_is_written_into_m4a_at_its_rate_of_bits() {
    let directory = tempfile::tempdir().unwrap();
    let mut sizes = vec![];
    for bitrate in [128_000, 320_000] {
        let file = directory.path().join(format!("mix-{bitrate}.m4a"));
        let settings = AudioSettings {
            codec: AudioCodec::Aac { bitrate },
            ..AudioSettings::default()
        };
        let mut writer = SoundWriter::create(&file, settings).unwrap();
        assert_eq!(writer.encoder(), "aac");
        // 4 s of noise, which AAC spends all its bits on.
        let given = noise(192_000 * 2);
        writer.audio(&given).unwrap();
        writer.finish().unwrap();
        let audio = probe(&file).unwrap().audio.unwrap();
        assert_eq!((audio.codec.as_str(), audio.rate), ("aac", 48000));
        let read = read_back(&file);
        assert!(read.len() >= given.len(), "{}", read.len());
        sizes.push(std::fs::metadata(&file).unwrap().len());
    }
    assert!(sizes[1] > sizes[0] * 3 / 2, "{sizes:?}");
}

#[test]
fn pcm_is_written_beside_a_picture_in_quicktime() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("cut.mov");
    let video = VideoSettings {
        codec: Codec::ProRes,
        width: 64,
        height: 36,
        rate: Rate { num: 25, den: 1 },
        bits: 10,
        quality: Quality::Small,
    };
    let audio = AudioSettings {
        codec: AudioCodec::PcmS24,
        ..AudioSettings::default()
    };
    let mut writer = Writer::create(&file, video, Some(audio)).unwrap();
    let picture = Picture {
        width: 64,
        height: 36,
        bits: 8,
        luma: vec![16; 64 * 36],
        chroma: vec![128; 64 * 18],
        luma_stride: 64,
        chroma_stride: 64,
        ..Picture::default()
    };
    let given = tones(1920 * 5, 2, false);
    for frame in 0..5 {
        writer.video(&picture).unwrap();
        writer
            .audio(&given[frame * 3840..(frame + 1) * 3840])
            .unwrap();
    }
    writer.finish().unwrap();
    let facts = probe(&file).unwrap();
    assert_eq!(facts.audio.as_ref().unwrap().codec, "pcm_s24le");
    let read = read_back(&file);
    assert_eq!(read.len(), given.len());
    let step = 1.0 / 8_388_608.0;
    assert!(read
        .iter()
        .zip(&given)
        .all(|(a, b)| (f64::from(*a) - f64::from(*b)).abs() <= step));
}

#[test]
fn sound_that_cannot_be_written_so_is_refused() {
    let directory = tempfile::tempdir().unwrap();
    let refused = |name: &str, settings: AudioSettings| {
        SoundWriter::create(&directory.path().join(name), settings)
            .err()
            .unwrap()
    };
    let pcm = AudioSettings {
        codec: AudioCodec::PcmS24,
        ..AudioSettings::default()
    };
    let aac = AudioSettings::default();
    // PCM in MPEG-4 audio, AAC in WAV, sound alone in a container of pictures.
    assert_eq!(refused("a.m4a", pcm).reason(), "unsupported");
    assert_eq!(refused("a.wav", aac).reason(), "unsupported");
    assert_eq!(refused("a.mov", pcm).reason(), "unsupported");
    assert_eq!(refused("a.flac", pcm).reason(), "unsupported");
    // Nine channels of AAC, 25 of PCM, a mask of two speakers for six channels.
    let nine = AudioSettings { channels: 9, ..aac };
    assert_eq!(refused("b.m4a", nine).reason(), "unsupported");
    let many = AudioSettings {
        channels: 25,
        ..pcm
    };
    assert_eq!(refused("b.wav", many).reason(), "unsupported");
    let masked = AudioSettings {
        channels: 6,
        mask: 0x3,
        ..pcm
    };
    assert_eq!(refused("c.wav", masked).reason(), "unsupported");
    // PCM in MP4 beside a picture.
    let video = VideoSettings {
        codec: Codec::ProRes,
        width: 64,
        height: 36,
        rate: Rate { num: 25, den: 1 },
        bits: 10,
        quality: Quality::Small,
    };
    let h264 = VideoSettings {
        codec: Codec::H264,
        bits: 8,
        ..video
    };
    let error = Writer::create(&directory.path().join("d.mp4"), h264, Some(pcm))
        .err()
        .unwrap();
    assert_eq!(error.reason(), "unsupported", "{error}");
    // A picture into WAV.
    let error = Writer::create(&directory.path().join("e.wav"), video, Some(pcm))
        .err()
        .unwrap();
    assert_eq!(error.reason(), "unsupported", "{error}");
}

#[test]
fn eight_channels_of_aac_and_twenty_four_of_pcm_are_written() {
    let directory = tempfile::tempdir().unwrap();
    // 7.1 as libav keeps it: FL FR FC LFE BL BR SL SR.
    let aac = AudioSettings {
        channels: 8,
        mask: 0x63F,
        ..AudioSettings::default()
    };
    let file = directory.path().join("71.m4a");
    let mut writer = SoundWriter::create(&file, aac).unwrap();
    writer.audio(&tones(48_000, 8, false)).unwrap();
    writer.finish().unwrap();
    assert_eq!(probe(&file).unwrap().audio.unwrap().channels, 8);
    let pcm = AudioSettings {
        channels: 24,
        codec: AudioCodec::PcmF32,
        ..AudioSettings::default()
    };
    let file = directory.path().join("24.wav");
    let mut writer = SoundWriter::create(&file, pcm).unwrap();
    let given = tones(4800, 24, false);
    writer.audio(&given).unwrap();
    writer.finish().unwrap();
    assert_eq!(probe(&file).unwrap().audio.unwrap().channels, 24);
    assert_eq!(read_back(&file), given);
}
