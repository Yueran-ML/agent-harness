//! The frames of the media that the tests make, decoded on the graphics card and without it:
//! each is the frame that was asked for, read back from the number it carries.

mod support;

use std::time::{Duration, Instant};

use columbina_media::{
    Converter, Decoded, Hardware, HardwarePath, MediaError, Picture, VideoDecoder,
};
use support::{media, number_in, Made, Sample};

/// Numbers that are the same each time: splitmix64.
struct Numbers(u64);

impl Numbers {
    fn below(&mut self, most: i64) -> i64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        i64::try_from(z % u64::try_from(most).unwrap()).unwrap()
    }
}

/// The number that a picture carries.
fn carried(picture: &Picture) -> Option<i64> {
    number_in(&picture.luma, picture.luma_stride, picture.bits)
}

/// Check that a picture is whole: the number it is given is the number it carries, and its
/// planes are as long as its strides and size say.
fn check(picture: &Picture, frame: i64) {
    assert_eq!(picture.frame, frame, "{picture:?}");
    assert_eq!(carried(picture), Some(frame), "{picture:?}");
    let bytes = if picture.bits > 8 { 2 } else { 1 };
    let width = usize::try_from(picture.width).unwrap();
    let height = usize::try_from(picture.height).unwrap();
    assert!(picture.luma_stride >= width * bytes);
    assert!(picture.chroma_stride >= width.div_ceil(2) * 2 * bytes);
    assert!(picture.luma.len() >= picture.luma_stride * height);
    assert!(picture.chroma.len() >= picture.chroma_stride * height.div_ceil(2));
}

fn open(made: &Made, hardware: Hardware) -> VideoDecoder {
    let decoder = VideoDecoder::open(&made.path, hardware).unwrap();
    eprintln!(
        "{}: {hardware:?} decodes with {}; tried before: {:?}",
        made.path.display(),
        decoder.path_used(),
        decoder.fallbacks()
    );
    if hardware == Hardware::Off {
        assert_eq!(decoder.path_used(), HardwarePath::Software);
    }
    decoder
}

/// A decoder of the file on the processor, one as `Auto` chooses, and, on Windows, one on
/// Direct3D 11, which `Auto` does not reach on a computer with CUDA, where the file is decoded
/// on it.
fn decoders(made: &Made) -> Vec<VideoDecoder> {
    let mut decoders = vec![open(made, Hardware::Off), open(made, Hardware::Auto)];
    if cfg!(windows) {
        let decoder = VideoDecoder::open_trying(&made.path, &[HardwarePath::D3d11va]).unwrap();
        if decoder.path_used() == HardwarePath::D3d11va {
            decoders.push(decoder);
        } else {
            eprintln!("not on d3d11va: {:?}", decoder.fallbacks());
        }
    }
    decoders
}

/// Ask for 30 frames at random, then 10 after the last of them one at a time.
fn random_frames(sample: Sample, bits: u8) {
    let Some(made) = media(sample) else {
        return;
    };
    let frames = sample.frames().unwrap();
    for mut decoder in decoders(&made) {
        let mut numbers = Numbers(20_261_027);
        for _ in 0..30 {
            let frame = numbers.below(frames);
            let picture = decoder.frame_at(frame).unwrap();
            assert_eq!(picture.bits, bits);
            check(&picture, frame);
        }
        let start = numbers.below(frames - 11);
        check(&decoder.frame_at(start).unwrap(), start);
        for step in 1..=10 {
            check(&decoder.next().unwrap().unwrap(), start + step);
        }
    }
}

#[test]
fn any_frame_of_the_1080p_h264_file() {
    random_frames(Sample::Fhd, 8);
}

#[test]
fn any_frame_of_the_4k_hevc_main10_file() {
    random_frames(Sample::Uhd, 10);
}

#[test]
fn any_frame_of_the_prores_file_converted_from_4_2_2() {
    random_frames(Sample::Prores, 10);
}

#[test]
fn any_frame_of_the_portrait_file_as_it_is_stored() {
    random_frames(Sample::Portrait, 8);
    let Some(made) = media(Sample::Portrait) else {
        return;
    };
    let picture = VideoDecoder::open(&made.path, Hardware::Off)
        .unwrap()
        .frame_at(0)
        .unwrap();
    assert_eq!((picture.width, picture.height), (640, 360));
}

#[test]
fn every_frame_comes_in_order_from_the_start_to_the_end() {
    let Some(made) = media(Sample::Fhd) else {
        return;
    };
    for mut decoder in decoders(&made) {
        for frame in 0..100 {
            check(&decoder.next().unwrap().unwrap(), frame);
        }
        assert_eq!(decoder.next().unwrap(), None);
        assert_eq!(decoder.next().unwrap(), None);
        assert_eq!(decoder.seeks(), 0);
        // A frame after the end is past the last, which is now known.
        let error = decoder.frame_at(100).unwrap_err();
        assert!(
            matches!(
                error,
                MediaError::OutOfRange {
                    frame: 100,
                    last: Some(99)
                }
            ),
            "{error}"
        );
        // And once the end was reached, a frame is found again.
        check(&decoder.frame_at(42).unwrap(), 42);
        check(&decoder.next().unwrap().unwrap(), 43);
    }
}

#[test]
fn a_frame_after_the_end_or_before_the_start_is_out_of_range() {
    let Some(made) = media(Sample::Uhd) else {
        return;
    };
    for mut decoder in decoders(&made) {
        let error = decoder.frame_at(-1).unwrap_err();
        assert!(
            matches!(error, MediaError::OutOfRange { frame: -1, .. }),
            "{error}"
        );
        let error = decoder.frame_at(180).unwrap_err();
        assert!(
            matches!(
                error,
                MediaError::OutOfRange {
                    frame: 180,
                    last: Some(179)
                }
            ),
            "{error}"
        );
        assert_eq!(decoder.next().unwrap(), None);
        let error = decoder.frame_at(10_000).unwrap_err();
        assert!(
            matches!(error, MediaError::OutOfRange { frame: 10_000, .. }),
            "{error}"
        );
        check(&decoder.frame_at(179).unwrap(), 179);
        assert_eq!(decoder.next().unwrap(), None);
        // A hardware decoder that has decoded to the end and seeks again gives no frame left
        // from before.
        check(&decoder.frame_at(3).unwrap(), 3);
        check(&decoder.next().unwrap().unwrap(), 4);
    }
}

#[test]
fn frames_further_on_in_the_same_group_are_decoded_on_without_seeking() {
    let Some(made) = media(Sample::Uhd) else {
        return;
    };
    // Key frames at 0 and 120.
    for mut decoder in decoders(&made) {
        for frame in [0, 5, 6, 30, 31, 90, 119] {
            check(&decoder.frame_at(frame).unwrap(), frame);
        }
        assert_eq!(
            decoder.seeks(),
            0,
            "on from where it was, within the group of frame 0"
        );
        check(&decoder.frame_at(150).unwrap(), 150);
        assert_eq!(decoder.seeks(), 0, "on through the key frame at 120");
        check(&decoder.frame_at(140).unwrap(), 140);
        assert_eq!(decoder.seeks(), 1, "back: it seeks");
        check(&decoder.frame_at(140).unwrap(), 140);
        assert_eq!(decoder.seeks(), 2, "the same frame again: it seeks");
        check(&decoder.frame_at(100).unwrap(), 100);
        assert_eq!(decoder.seeks(), 3);
        check(&decoder.frame_at(130).unwrap(), 130);
        assert_eq!(
            decoder.seeks(),
            4,
            "a key frame lies between: it seeks to it rather than decode on to it"
        );
    }
}

#[test]
fn the_next_frames_after_opening_start_at_the_first() {
    let Some(made) = media(Sample::Prores) else {
        return;
    };
    let mut decoder = open(&made, Hardware::Auto);
    for frame in 0..10 {
        check(&decoder.next().unwrap().unwrap(), frame);
    }
}

#[test]
fn a_decoded_frame_is_made_a_picture_on_another_thread() {
    let Some(made) = media(Sample::Uhd) else {
        return;
    };
    let mut decoder = open(&made, Hardware::Auto);
    let (sender, receiver) = std::sync::mpsc::sync_channel::<Decoded>(2);
    let maker = std::thread::spawn(move || {
        let mut converter = Converter::new();
        let mut made = Vec::new();
        for decoded in receiver {
            made.push(converter.picture(&decoded).unwrap());
        }
        made
    });
    for frame in 0..40 {
        let given = decoder.next_decoded().unwrap().unwrap();
        assert_eq!(given.frame(), frame);
        sender.send(given).unwrap();
    }
    drop(sender);
    let pictures = maker.join().unwrap();
    for (frame, picture) in (0..).zip(&pictures) {
        check(picture, frame);
    }
}

#[test]
fn a_decoder_may_be_sent_to_another_thread() {
    fn sent<T: Send>() {}
    sent::<VideoDecoder>();
    sent::<Decoded>();
    sent::<Converter>();
}

/// What it takes to decode the 4K file: run with `--ignored --nocapture`.
#[test]
#[ignore = "a measurement, not a test"]
fn measure_the_4k_file() {
    let Some(made) = media(Sample::Uhd) else {
        return;
    };
    let median = |times: &mut Vec<Duration>| {
        times.sort();
        (times[times.len() / 2], times[times.len() - 1])
    };
    let tries: [&[HardwarePath]; 3] = [&[HardwarePath::Cuda], &[HardwarePath::D3d11va], &[]];
    for devices in tries {
        let started = Instant::now();
        let mut decoder = VideoDecoder::open_trying(&made.path, devices).unwrap();
        let opened = started.elapsed();
        let mut numbers = Numbers(7);
        let mut random = Vec::new();
        for _ in 0..50 {
            let frame = numbers.below(180);
            let started = Instant::now();
            check(&decoder.frame_at(frame).unwrap(), frame);
            random.push(started.elapsed());
        }
        decoder.frame_at(10).unwrap();
        let mut steps = Vec::new();
        for frame in 11..71 {
            let started = Instant::now();
            check(&decoder.next().unwrap().unwrap(), frame);
            steps.push(started.elapsed());
        }
        // Frames made pictures on another thread while the next is decoded.
        decoder.frame_at(0).unwrap();
        let started = Instant::now();
        let (sender, receiver) = std::sync::mpsc::sync_channel::<Decoded>(2);
        let maker = std::thread::spawn(move || {
            // One picture, written over for each frame, as a player that shows each once.
            let mut converter = Converter::new();
            let mut picture = Picture::default();
            receiver
                .into_iter()
                .map(|decoded| converter.picture_into(&decoded, &mut picture).unwrap())
                .count()
        });
        let mut sent = 0;
        while let Some(decoded) = decoder.next_decoded().unwrap() {
            sender.send(decoded).unwrap();
            sent += 1;
        }
        drop(sender);
        assert_eq!(maker.join().unwrap(), sent);
        let pipelined = started.elapsed();
        let (random_median, random_most) = median(&mut random);
        let (step_median, step_most) = median(&mut steps);
        eprintln!(
            "{}: open {opened:?}; any frame median {random_median:?}, most \
             {random_most:?}; the next frame median {step_median:?}, most {step_most:?}; \
             {sent} frames with the copy on another thread at {:.0} a second",
            decoder.path_used(),
            f64::from(u32::try_from(sent).unwrap()) / pipelined.as_secs_f64()
        );
    }
}
