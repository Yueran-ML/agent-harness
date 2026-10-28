//! Pictures and sound written into a file by each encoder that opens on this machine, and read
//! back: every frame, with the number that it carries, and the sound at its pitch.
//!
//! An encoder of the graphics card that this machine lacks is skipped, unless
//! `COLUMBINA_REQUIRE_GPU` is `1`: then NVENC must open, for H.264 and for HEVC.

mod support;

use std::path::Path;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use columbina_media::{
    encoders_for, probe, AudioDecoder, AudioSettings, Codec, EncoderPath, Hardware, Picture,
    Quality, Rate, Samples, VideoDecoder, VideoSettings, Writer,
};
use support::number_in;

fn gpu_required() -> bool {
    std::env::var("COLUMBINA_REQUIRE_GPU").as_deref() == Ok("1")
}

/// A grey picture of `width` × `height` in 8 or 10 bits, its rows a little longer than they
/// need be, to be numbered by [`stamp`].
fn grey(width: u32, height: u32, bits: u8) -> Picture {
    let bytes = if bits > 8 { 2 } else { 1 };
    let (width_px, height_px) = (width as usize, height as usize);
    let stride = width_px.div_ceil(2) * 2 * bytes + 96;
    let mut picture = Picture {
        frame: 0,
        width,
        height,
        bits,
        luma: vec![0; stride * height_px],
        chroma: vec![0; stride * height_px.div_ceil(2)],
        luma_stride: stride,
        chroma_stride: stride,
    };
    for y in 0..height_px {
        for x in 0..width_px {
            put(&mut picture.luma, y * stride, x, bits, 128);
        }
    }
    for y in 0..height_px.div_ceil(2) {
        for x in 0..width_px.div_ceil(2) * 2 {
            put(&mut picture.chroma, y * stride, x, bits, 128);
        }
    }
    picture
}

/// Write a sample of 8 bits, made 10 where the plane is P010, at `x` of the row from `row`.
fn put(plane: &mut [u8], row: usize, x: usize, bits: u8, value: u16) {
    if bits > 8 {
        let bytes = ((value * 4) << 6).to_le_bytes();
        plane[row + 2 * x..row + 2 * x + 2].copy_from_slice(&bytes);
    } else {
        plane[row + x] = u8::try_from(value).unwrap();
    }
}

/// Number a picture as the media of the tests are: three blocks of 64×64 in its top left
/// corner, each a hexadecimal digit `d`, the least significant leftmost, of luma `16 + 14·d`
/// (`64 + 56·d` in 10 bits).
fn stamp(picture: &mut Picture, frame: i64) {
    picture.frame = frame;
    for y in 0..64 {
        for x in 0..192 {
            let digit = u16::try_from((frame >> (4 * (x / 64))) & 15).unwrap();
            let row = y * picture.luma_stride;
            if picture.bits > 8 {
                let bytes = ((64 + 56 * digit) << 6).to_le_bytes();
                picture.luma[row + 2 * x..row + 2 * x + 2].copy_from_slice(&bytes);
            } else {
                picture.luma[row + x] = u8::try_from(16 + 14 * digit).unwrap();
            }
        }
    }
}

/// One test at a time that encodes on the graphics: Quick Sync of Intel, given a few files to
/// encode at once, may stall for a hundred seconds and then fail, and an export runs alone.
fn one_at_a_time() -> MutexGuard<'static, ()> {
    static ONE: Mutex<()> = Mutex::new(());
    ONE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The encoders of `codec` to be tried here; NVENC must be among them where the graphics card
/// is required.
fn encoders(codec: Codec) -> Vec<EncoderPath> {
    let found = encoders_for(codec);
    eprintln!("{codec:?} opens with {found:?}");
    if gpu_required() && codec != Codec::ProRes {
        assert!(
            found.contains(&EncoderPath::Nvenc),
            "COLUMBINA_REQUIRE_GPU is 1, and NVENC does not open for {codec:?}"
        );
    }
    found
}

/// Write 60 numbered frames of 1080p with `encoder`, and read them back: how many there are,
/// the number that each carries, and the size, rate, codec and bits that the file says.
fn written_and_read(
    file: &Path,
    encoder: EncoderPath,
    codec: Codec,
    bits_in: u8,
    bits_out: u8,
    rate: Rate,
) {
    let settings = VideoSettings {
        codec,
        width: 1920,
        height: 1080,
        rate,
        bits: bits_out,
        quality: Quality::High,
    };
    let mut writer = Writer::create_on(file, encoder, settings, None).unwrap();
    assert_eq!(writer.encoder(), encoder);
    let mut picture = grey(1920, 1080, bits_in);
    let started = Instant::now();
    for frame in 0..60 {
        stamp(&mut picture, frame);
        writer.video(&picture).unwrap();
    }
    writer.finish().unwrap();
    eprintln!(
        "{}: 60 frames with {encoder:?} in {:?}",
        file.display(),
        started.elapsed()
    );

    let facts = probe(file).unwrap();
    let video = facts.video.unwrap();
    assert_eq!((video.width, video.height), (1920, 1080));
    assert_eq!(video.rate, rate);
    let name = match codec {
        Codec::H264 => "h264",
        Codec::Hevc => "hevc",
        Codec::ProRes => "prores",
    };
    assert_eq!(video.codec, name);
    assert_eq!(
        video.bits,
        if codec == Codec::ProRes { 10 } else { bits_out }
    );
    assert!(facts.audio.is_none());

    let mut decoder = VideoDecoder::open(file, Hardware::Off).unwrap();
    let mut count = 0;
    while let Some(picture) = decoder.next().unwrap() {
        assert_eq!(picture.frame, count);
        assert_eq!(
            number_in(&picture.luma, picture.luma_stride, picture.bits),
            Some(count),
            "frame {count} of {}",
            file.display()
        );
        count += 1;
    }
    assert_eq!(count, 60);

    // The index of the file is before its data, so that it plays as it is downloaded.
    let bytes = std::fs::read(file).unwrap();
    let at = |name: &[u8]| bytes.windows(4).position(|window| window == name).unwrap();
    assert!(at(b"moov") < at(b"mdat"), "{}", file.display());
}

#[test]
fn h264_of_8_bits_is_written_and_read_back_frame_by_frame() {
    let _one = one_at_a_time();
    let directory = tempfile::tempdir().unwrap();
    for encoder in encoders(Codec::H264) {
        let file = directory.path().join(format!("{encoder:?}.mp4"));
        let ntsc = Rate {
            num: 30000,
            den: 1001,
        };
        written_and_read(&file, encoder, Codec::H264, 8, 8, ntsc);
    }
}

#[test]
fn hevc_of_10_bits_is_written_and_read_back_frame_by_frame() {
    let _one = one_at_a_time();
    let directory = tempfile::tempdir().unwrap();
    for encoder in encoders(Codec::Hevc) {
        let file = directory.path().join(format!("{encoder:?}.mp4"));
        let rate = Rate { num: 25, den: 1 };
        written_and_read(&file, encoder, Codec::Hevc, 10, 10, rate);
    }
}

#[test]
fn hevc_of_10_bits_is_written_from_pictures_of_8_bits() {
    let _one = one_at_a_time();
    let directory = tempfile::tempdir().unwrap();
    for encoder in encoders(Codec::Hevc) {
        let file = directory.path().join(format!("{encoder:?}.mov"));
        let rate = Rate { num: 50, den: 1 };
        written_and_read(&file, encoder, Codec::Hevc, 8, 10, rate);
    }
}

#[test]
fn prores_is_written_from_pictures_of_10_and_8_bits_and_read_back_frame_by_frame() {
    let found = encoders(Codec::ProRes);
    assert_eq!(found, [EncoderPath::ProResKs]);
    let directory = tempfile::tempdir().unwrap();
    let rate = Rate { num: 24, den: 1 };
    for bits in [10, 8] {
        let file = directory.path().join(format!("{bits}.mov"));
        written_and_read(&file, EncoderPath::ProResKs, Codec::ProRes, bits, 10, rate);
    }
}

#[test]
fn the_first_encoder_that_opens_writes_the_file() {
    let _one = one_at_a_time();
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("first.mov");
    let settings = VideoSettings {
        codec: Codec::Hevc,
        width: 640,
        height: 360,
        rate: Rate { num: 25, den: 1 },
        bits: 8,
        quality: Quality::Small,
    };
    if let Some(&first) = encoders(Codec::Hevc).first() {
        let writer = Writer::create(&file, settings, None).unwrap();
        assert_eq!(writer.encoder(), first);
        writer.finish().unwrap();
    } else {
        let error = Writer::create(&file, settings, None).err().unwrap();
        assert_eq!(error.reason(), "no_encoder", "{error}");
    }
}

/// The samples of a tone of 1 kHz at 48 kHz in stereo, at half of full scale, from `start`.
fn tone(start: usize, count: usize) -> Vec<f32> {
    (start..start + count)
        .flat_map(|sample| {
            #[allow(clippy::cast_precision_loss)]
            let phase = 2.0 * std::f64::consts::PI * 1000.0 * sample as f64 / 48000.0;
            #[allow(clippy::cast_possible_truncation)]
            let value = (0.5 * phase.sin()) as f32;
            [value, value]
        })
        .collect()
}

#[test]
fn a_tone_of_1_khz_is_read_back_at_1_khz() {
    let _one = one_at_a_time();
    let directory = tempfile::tempdir().unwrap();
    // H.264 in MP4 where it can be written, else ProRes in QuickTime.
    let (codec, file) = if encoders(Codec::H264).is_empty() {
        (Codec::ProRes, directory.path().join("tone.mov"))
    } else {
        (Codec::H264, directory.path().join("tone.mp4"))
    };
    let settings = VideoSettings {
        codec,
        width: 640,
        height: 360,
        rate: Rate { num: 25, den: 1 },
        bits: 8,
        quality: Quality::Medium,
    };
    let mut writer = Writer::create(&file, settings, Some(AudioSettings::default())).unwrap();
    let mut picture = grey(640, 360, 8);
    // 2 s of each, the sound in runs of a length that no frame of AAC has.
    let mut sent = 0;
    for frame in 0..50 {
        stamp(&mut picture, frame);
        writer.video(&picture).unwrap();
        let until = usize::try_from(frame + 1).unwrap() * 1920;
        while sent < until {
            let count = 777.min(until - sent);
            writer.audio(&tone(sent, count)).unwrap();
            sent += count;
        }
    }
    writer.finish().unwrap();

    let facts = probe(&file).unwrap();
    let audio = facts.audio.unwrap();
    assert_eq!((audio.rate, audio.channels), (48000, 2));
    assert_eq!(audio.codec, "aac");

    let mut decoder = AudioDecoder::open(&file, None).unwrap();
    let mut data = Vec::new();
    let mut first = None;
    while let Some(Samples { start, data: run }) = decoder.next().unwrap() {
        first.get_or_insert(start);
        data.extend(run);
    }
    let instants = data.len() / 2;
    eprintln!("{instants} instants from {first:?}");
    assert!(first.unwrap().abs() <= 1, "{first:?}");
    // libav decodes the padding of the last frame of AAC too: up to a frame more.
    assert!((96000..=96000 + 1024).contains(&instants), "{instants}");
    // Rising crossings of zero in the second in the middle: a thousand.
    let left: Vec<f32> = data.iter().step_by(2).copied().collect();
    let crossings = left[24000..72000]
        .windows(2)
        .filter(|pair| pair[0] < 0.0 && pair[1] >= 0.0)
        .count();
    assert!(crossings.abs_diff(1000) <= 2, "{crossings}");
    let peak = left[24000..72000]
        .iter()
        .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
    assert!((0.45..0.55).contains(&peak), "{peak}");
}

#[test]
fn a_writer_may_be_sent_to_another_thread() {
    fn sent<T: Send>() {}
    sent::<Writer>();
}

#[test]
fn a_file_named_part_is_written_as_the_container_named_before_it() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("cut.mov.part");
    let settings = VideoSettings {
        codec: Codec::ProRes,
        width: 640,
        height: 360,
        rate: Rate { num: 25, den: 1 },
        bits: 10,
        quality: Quality::Small,
    };
    let mut writer = Writer::create(&file, settings, None).unwrap();
    let mut picture = grey(640, 360, 10);
    for frame in 0..5 {
        stamp(&mut picture, frame);
        writer.video(&picture).unwrap();
    }
    writer.finish().unwrap();
    let video = probe(&file).unwrap().video.unwrap();
    assert_eq!((video.codec.as_str(), video.width), ("prores", 640));
}

#[test]
fn what_cannot_be_written_is_refused() {
    let directory = tempfile::tempdir().unwrap();
    let settings = VideoSettings {
        codec: Codec::ProRes,
        width: 640,
        height: 360,
        rate: Rate { num: 25, den: 1 },
        bits: 10,
        quality: Quality::High,
    };
    let refused = |name: &str, settings: VideoSettings| {
        Writer::create(&directory.path().join(name), settings, None)
            .err()
            .unwrap()
    };
    // ProRes in MP4; a container that is neither MP4 nor QuickTime; H.264 of 10 bits.
    assert_eq!(refused("a.mp4", settings).reason(), "unsupported");
    assert_eq!(refused("a.avi", settings).reason(), "unsupported");
    let h264 = VideoSettings {
        codec: Codec::H264,
        ..settings
    };
    assert_eq!(refused("b.mp4", h264).reason(), "unsupported");
    let odd = VideoSettings {
        width: 641,
        ..settings
    };
    assert_eq!(refused("c.mov", odd).reason(), "unsupported");
    // A folder that is not there.
    let error = Writer::create(&directory.path().join("no/d.mov"), settings, None)
        .err()
        .unwrap();
    assert_eq!(error.reason(), "write", "{error}");

    // A picture of another size than the file's.
    let mut writer = Writer::create(&directory.path().join("e.mov"), settings, None).unwrap();
    let error = writer.video(&grey(320, 180, 10)).err().unwrap();
    assert_eq!(error.reason(), "encode", "{error}");
    // Sound, where the file has none.
    let error = writer.audio(&tone(0, 100)).err().unwrap();
    assert_eq!(error.reason(), "encode", "{error}");
    writer.finish().unwrap();
}

/// How fast each encoder of the graphics card writes 1080p and 4K: run with
/// `--ignored --nocapture`.
#[test]
#[ignore = "a measurement, not a test"]
fn measure_the_encoders() {
    let _one = one_at_a_time();
    let directory = tempfile::tempdir().unwrap();
    for (codec, bits) in [(Codec::H264, 8), (Codec::Hevc, 10), (Codec::ProRes, 10)] {
        for encoder in encoders_for(codec) {
            for (width, height) in [(1920, 1080), (3840, 2160)] {
                let file = directory.path().join(format!("{encoder:?}-{width}.mov"));
                let settings = VideoSettings {
                    codec,
                    width,
                    height,
                    rate: Rate { num: 30, den: 1 },
                    bits,
                    quality: Quality::Medium,
                };
                let mut writer = Writer::create_on(&file, encoder, settings, None).unwrap();
                let mut picture = grey(width, height, bits);
                let started = Instant::now();
                for frame in 0..240 {
                    stamp(&mut picture, frame);
                    writer.video(&picture).unwrap();
                }
                writer.finish().unwrap();
                let seconds = started.elapsed().as_secs_f64();
                eprintln!(
                    "{codec:?} {bits} bits {width}×{height} with {encoder:?}: {:.0} frames a second",
                    240.0 / seconds
                );
            }
        }
    }
}
