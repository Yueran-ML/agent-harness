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

/// Check that a picture is whole: its planes are as long as its strides and size say.
fn whole(picture: &Picture) {
    let bytes = if picture.bits > 8 { 2 } else { 1 };
    let width = usize::try_from(picture.width).unwrap();
    let height = usize::try_from(picture.height).unwrap();
    assert!(picture.luma_stride >= width * bytes);
    assert!(picture.chroma_stride >= width.div_ceil(2) * 2 * bytes);
    assert!(picture.luma.len() >= picture.luma_stride * height);
    assert!(picture.chroma.len() >= picture.chroma_stride * height.div_ceil(2));
}

/// Check that a picture is whole, and that the number it is given is the number it carries.
fn check(picture: &Picture, frame: i64) {
    assert_eq!(picture.frame, frame, "{picture:?}");
    assert_eq!(carried(picture), Some(frame), "{picture:?}");
    whole(picture);
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

/// Every frame from the start to the end, as `(number, number carried)`.
fn sequence(decoder: &mut VideoDecoder) -> Vec<(i64, i64)> {
    let mut frames = Vec::new();
    while let Some(picture) = decoder.next().unwrap() {
        whole(&picture);
        frames.push((picture.frame, carried(&picture).unwrap()));
    }
    frames
}

/// The frame that is shown at `frame`, of the frames in order: the first numbered so, or, if
/// none is, the last one before it.
fn shown_at(frames: &[(i64, i64)], frame: i64) -> usize {
    frames
        .iter()
        .position(|&(number, _)| number == frame)
        .unwrap_or_else(|| {
            frames
                .iter()
                .rposition(|&(number, _)| number < frame)
                .unwrap()
        })
}

/// Decode every frame of a sample in order, on each path, and check that each is there, once
/// and in order; then, on the same decoder, after it reached the end, ask for `random` frames
/// at random,
/// those of its first group of frames and its last, and from the end back to the start, each
/// with the two after it: each must be the frame that is shown there. Gives the most seeks for
/// one frame asked for, over all paths, rounded up.
fn every_frame_of(sample: Sample, bits: u8, random: usize) -> Option<u64> {
    let made = media(sample)?;
    let frames = sample.frames().unwrap();
    let mut most_seeks = 0;
    for mut decoder in decoders(&made) {
        let path = decoder.path_used();
        let all = sequence(&mut decoder);
        let held: Vec<i64> = all.iter().map(|&(_, held)| held).collect();
        assert_eq!(held, (0..frames).collect::<Vec<_>>(), "{path}");
        let last = all.last().unwrap().0;
        let mut numbers = Numbers(20_261_027);
        let mut targets: Vec<i64> = (0..random).map(|_| numbers.below(last + 1)).collect();
        targets.extend([0, 1, 2, 5, 9, 14, 3, last, last - 1, last, 0]);
        targets.extend((0..8).rev().map(|eighth| last * eighth / 8));
        let seeks = decoder.seeks();
        for &target in &targets {
            let picture = decoder.frame_at(target).unwrap();
            assert_eq!(picture.bits, bits);
            let at = shown_at(&all, target);
            let found = (picture.frame, carried(&picture).unwrap());
            assert_eq!(found, all[at], "{path}: frame {target}");
            for after in &all[at + 1..all.len().min(at + 3)] {
                let picture = decoder.next().unwrap().unwrap();
                assert_eq!(
                    (picture.frame, carried(&picture).unwrap()),
                    *after,
                    "{path}: after frame {target}"
                );
            }
        }
        let error = decoder.frame_at(last + 1).unwrap_err();
        assert!(matches!(error, MediaError::OutOfRange { .. }), "{error}");
        let sought = (decoder.seeks() - seeks).div_ceil(targets.len() as u64);
        eprintln!(
            "{path}: {} seeks for {} frames",
            decoder.seeks() - seeks,
            targets.len()
        );
        most_seeks = most_seeks.max(sought);
    }
    Some(most_seeks)
}

#[test]
fn every_frame_of_the_1080p_h264_file() {
    every_frame_of(Sample::Fhd, 8, 30);
}

#[test]
fn every_frame_of_the_4k_hevc_main10_file() {
    every_frame_of(Sample::Uhd, 10, 12);
}

#[test]
fn every_frame_of_the_prores_file_converted_from_4_2_2() {
    every_frame_of(Sample::Prores, 10, 30);
}

#[test]
fn every_frame_of_the_portrait_file_as_it_is_stored() {
    every_frame_of(Sample::Portrait, 8, 30);
    let Some(made) = media(Sample::Portrait) else {
        return;
    };
    let picture = VideoDecoder::open(&made.path, Hardware::Off)
        .unwrap()
        .frame_at(0)
        .unwrap();
    assert_eq!((picture.width, picture.height), (640, 360));
}

/// MPEG-TS and MPEG-PS are sought by time with no index of their key frames, and the frames of
/// their first group are found again from their first byte.
#[test]
fn every_frame_of_mpeg_ts_of_h264() {
    if let Some(seeks) = every_frame_of(Sample::TsH264, 8, 30) {
        assert!(seeks <= 2, "{seeks} seeks for a frame");
    }
}

#[test]
fn every_frame_of_mpeg_ts_of_mpeg_2() {
    if let Some(seeks) = every_frame_of(Sample::TsMpeg2, 8, 30) {
        assert!(seeks <= 2, "{seeks} seeks for a frame");
    }
}

#[test]
fn every_frame_of_mpeg_ps_of_mpeg_2() {
    if let Some(seeks) = every_frame_of(Sample::PsMpeg2, 8, 30) {
        assert!(seeks <= 2, "{seeks} seeks for a frame");
    }
}

#[test]
fn every_frame_of_m2ts() {
    if let Some(seeks) = every_frame_of(Sample::M2ts, 8, 30) {
        assert!(seeks <= 2, "{seeks} seeks for a frame");
    }
}

#[test]
fn every_frame_of_matroska_whose_picture_starts_late() {
    every_frame_of(Sample::MkvLateStart, 8, 30);
}

#[test]
fn every_frame_of_matroska_whose_sound_lasts_longer() {
    every_frame_of(Sample::MkvLongSound, 8, 30);
}

#[test]
fn every_frame_of_a_file_whose_frames_come_at_changing_times() {
    every_frame_of(Sample::Vfr, 8, 30);
}

#[test]
fn frames_with_the_same_timestamp_are_all_given() {
    let Some(made) = media(Sample::SameTimes) else {
        return;
    };
    for mut decoder in decoders(&made) {
        let all = sequence(&mut decoder);
        // Each frame is there, in order; every fourth has the number of the one before it.
        let held: Vec<i64> = all.iter().map(|&(_, held)| held).collect();
        assert_eq!(held, (0..100).collect::<Vec<_>>());
        for &(number, held) in &all {
            assert_eq!(number, held - i64::from(held % 4 == 3));
        }
        // The frame shown at a number of two is the first of them, then the second comes.
        let picture = decoder.frame_at(6).unwrap();
        assert_eq!((picture.frame, carried(&picture)), (6, Some(6)));
        let picture = decoder.next().unwrap().unwrap();
        assert_eq!((picture.frame, carried(&picture)), (6, Some(7)));
        // The same number again, after the second was given: the first again, as from the
        // start, and then the second again.
        let picture = decoder.frame_at(6).unwrap();
        assert_eq!((picture.frame, carried(&picture)), (6, Some(6)));
        let picture = decoder.next().unwrap().unwrap();
        assert_eq!((picture.frame, carried(&picture)), (6, Some(7)));
        // A number that no frame has: the last before it, the second of the two.
        let picture = decoder.frame_at(7).unwrap();
        assert_eq!((picture.frame, carried(&picture)), (6, Some(7)));
        let picture = decoder.next().unwrap().unwrap();
        assert_eq!((picture.frame, carried(&picture)), (8, Some(8)));
        // From a decoder that has given nothing, the same frames.
        let mut fresh = VideoDecoder::open_trying(&made.path, &[decoder.path_used()]).unwrap();
        let picture = fresh.frame_at(6).unwrap();
        assert_eq!((picture.frame, carried(&picture)), (6, Some(6)));
        let mut fresh = VideoDecoder::open_trying(&made.path, &[decoder.path_used()]).unwrap();
        let picture = fresh.frame_at(7).unwrap();
        assert_eq!((picture.frame, carried(&picture)), (6, Some(7)));
    }
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
        assert_eq!(decoder.seeks(), 1, "the same frame again: it is kept");
        check(&decoder.next().unwrap().unwrap(), 141);
        check(&decoder.frame_at(100).unwrap(), 100);
        assert_eq!(decoder.seeks(), 2);
        check(&decoder.frame_at(130).unwrap(), 130);
        assert_eq!(
            decoder.seeks(),
            3,
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
fn one_converter_takes_frames_of_8_and_10_bits_of_one_size_in_turn() {
    let (Some(eight), Some(ten)) = (media(Sample::Fhd), media(Sample::Fhd10)) else {
        return;
    };
    for (mut eights, mut tens) in decoders(&eight).into_iter().zip(decoders(&ten)) {
        let mut converter = Converter::new();
        let mut kept = Picture::default();
        for frame in 0..6 {
            for (decoder, bits) in [(&mut eights, 8), (&mut tens, 10)] {
                let decoded = decoder.next_decoded().unwrap().unwrap();
                let picture = converter.picture(&decoded).unwrap();
                assert_eq!(picture.bits, bits, "{}", decoder.path_used());
                check(&picture, frame);
                converter.picture_into(&decoded, &mut kept).unwrap();
                assert_eq!(kept.bits, bits);
                check(&kept, frame);
            }
        }
    }
}

#[test]
fn a_decoder_whose_card_fails_goes_on_on_the_processor() {
    let Some(made) = media(Sample::Fhd) else {
        return;
    };
    for devices in [&[HardwarePath::Cuda][..], &[HardwarePath::D3d11va]] {
        let mut decoder = VideoDecoder::open_trying(&made.path, devices).unwrap();
        let path = decoder.path_used();
        if path == HardwarePath::Software {
            eprintln!("skipped: not on {devices:?}");
            continue;
        }
        check(&decoder.frame_at(10).unwrap(), 10);
        decoder.fail_device();
        check(&decoder.frame_at(60).unwrap(), 60);
        assert_eq!(decoder.path_used(), HardwarePath::Software);
        assert_eq!(decoder.fallbacks().last().unwrap().0, path);
        check(&decoder.next().unwrap().unwrap(), 61);

        // One that fails while frames are taken one at a time goes on with the next.
        let mut decoder = VideoDecoder::open_trying(&made.path, devices).unwrap();
        for frame in 0..3 {
            check(&decoder.next().unwrap().unwrap(), frame);
        }
        decoder.fail_device();
        for frame in 3..6 {
            check(&decoder.next().unwrap().unwrap(), frame);
        }
        assert_eq!(decoder.path_used(), HardwarePath::Software);
        let given = decoder.next_decoded().unwrap().unwrap();
        assert_eq!(given.frame(), 6);

        // One whose frame a converter of the caller's own could not copy from the card is
        // told so, and gives the frame again from the processor.
        let mut decoder = VideoDecoder::open_trying(&made.path, devices).unwrap();
        let mut converter = Converter::new();
        for frame in 0..3 {
            let given = decoder.next_decoded().unwrap().unwrap();
            check(&converter.picture(&given).unwrap(), frame);
        }
        decoder.device_failed("the copy of frame 2 failed");
        let again = decoder.decoded_at(2).unwrap();
        assert!(!again.on_device());
        check(&converter.picture(&again).unwrap(), 2);
        assert_eq!(decoder.path_used(), HardwarePath::Software);
        assert_eq!(
            decoder.fallbacks().last().unwrap(),
            &(
                path,
                "the graphics card failed: the copy of frame 2 failed".to_owned()
            )
        );
        check(&decoder.next().unwrap().unwrap(), 3);
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
        let seeks = decoder.seeks();
        for _ in 0..50 {
            let frame = numbers.below(180);
            let started = Instant::now();
            check(&decoder.frame_at(frame).unwrap(), frame);
            random.push(started.elapsed());
        }
        let seeks = decoder.seeks() - seeks;
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
            "{}: open {opened:?}; any frame median {random_median:?}, most {random_most:?}, \
             {seeks} seeks for 50; the next frame median {step_median:?}, most {step_most:?}; \
             {sent} frames with the copy on another thread at {:.0} a second",
            decoder.path_used(),
            f64::from(u32::try_from(sent).unwrap()) / pipelined.as_secs_f64()
        );
    }
}
