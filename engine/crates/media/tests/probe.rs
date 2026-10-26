//! What a file is and what it holds, as `identity` and `probe` find them, of the media that
//! the tests make and of files that are not media.

mod support;

use std::path::Path;

use columbina_media::{identity, probe, Hardware, MediaError, Rate, VideoDecoder};
use support::{media, Sample};

/// Bytes of a file of `size` that are the same wherever they are made.
fn pattern(size: usize) -> Vec<u8> {
    (0..size)
        .map(|i| u8::try_from((i * 31 + 7) % 251).unwrap())
        .collect()
}

#[test]
fn the_digest_is_the_one_that_python_makes() {
    // From `columbina.video.project.identity_of` of the same files, run with `uv run python`.
    let expected = [
        (
            0,
            "5feceb66ffc86f38d952786c6d696c79c2dbc239dd4e91b46729d73a27fb57e9",
        ),
        (
            1000,
            "a88f23ea00b831cc908ab4c2b02a27eab3f8c6cfb0745ffe4a8d782b9312b148",
        ),
        (
            65536,
            "4e7d02ea7951fdd94e325e26b3b46b414e1b11daf32084be9674201552f8f4cc",
        ),
        (
            65537,
            "d84a23839ef20ed7fdc2d228ae7162f9cf697d3bfaff5bd0d00d272a9ebb6baf",
        ),
        (
            100_000,
            "6dac5bac389ddc504d8f1a960849784baeb4f74d87182888f772d8d34c7e0d28",
        ),
        (
            131_072,
            "834dfb78324dcee9ff99de7ab528060415235af6896b4f0f7517fe99352421a4",
        ),
        (
            200_000,
            "540cae1692fe5278f71ec9ecbd6cf4f9610503245fade8a7b17ba9d77c6750c7",
        ),
    ];
    let directory = tempfile::tempdir().unwrap();
    for (size, digest) in expected {
        let path = directory.path().join(format!("f{size}.bin"));
        std::fs::write(&path, pattern(size)).unwrap();
        let found = identity(&path).unwrap();
        assert_eq!(found.digest, digest, "a file of {size} bytes");
        assert_eq!(found.size, u64::try_from(size).unwrap());
    }
}

#[test]
fn the_identity_says_when_the_file_was_written() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("a.bin");
    std::fs::write(&path, b"a").unwrap();
    let written = std::fs::metadata(&path).unwrap().modified().unwrap();
    let since = written.duration_since(std::time::UNIX_EPOCH).unwrap();
    let found = identity(&path).unwrap();
    assert_eq!(found.modified_us, i64::try_from(since.as_micros()).unwrap());
}

#[test]
fn a_file_that_is_not_there_is_named_so() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("gone.mp4");
    for error in [
        identity(&path).unwrap_err(),
        probe(&path).unwrap_err(),
        VideoDecoder::open(&path, Hardware::Auto).err().unwrap(),
    ] {
        assert_eq!(error.reason(), "not_found", "{error}");
    }
}

#[test]
fn files_that_are_not_media_are_refused_without_a_panic() {
    let directory = tempfile::tempdir().unwrap();
    let text = directory.path().join("notes.txt");
    std::fs::write(
        &text,
        "These are notes, and no picture or sound.\n".repeat(50),
    )
    .unwrap();
    let empty = directory.path().join("empty.mp4");
    std::fs::write(&empty, b"").unwrap();
    let noise = directory.path().join("noise.mp4");
    std::fs::write(&noise, pattern(100_000)).unwrap();
    for path in [&text, &empty, &noise] {
        let error = probe(path).unwrap_err();
        assert!(
            matches!(error, MediaError::Open(_) | MediaError::NoStreams),
            "{}: {error}",
            path.display()
        );
        for hardware in [Hardware::Auto, Hardware::Off] {
            assert!(VideoDecoder::open(path, hardware).is_err());
        }
        // The identity of a file is its identity, whatever it holds.
        identity(path).unwrap();
    }
}

#[test]
fn a_directory_is_not_a_file() {
    let directory = tempfile::tempdir().unwrap();
    for error in [
        identity(directory.path()).unwrap_err(),
        probe(directory.path()).unwrap_err(),
    ] {
        assert_eq!(error.reason(), "unreadable", "{error}");
    }
}

#[cfg(windows)]
#[test]
fn a_path_that_is_not_utf8_is_refused() {
    use std::os::windows::ffi::OsStringExt;
    let directory = tempfile::tempdir().unwrap();
    // "a" and a half of a pair of UTF-16, which Windows allows in a name.
    let name = std::ffi::OsString::from_wide(&[u16::from(b'a'), 0xD800]);
    let path = directory.path().join(name);
    if std::fs::write(&path, b"x").is_err() {
        eprintln!("skipped: the file system refuses the name");
        return;
    }
    assert!(matches!(identity(&path), Err(MediaError::NotUtf8)));
    assert!(matches!(probe(&path), Err(MediaError::NotUtf8)));
    assert!(matches!(
        VideoDecoder::open(&path, Hardware::Off),
        Err(MediaError::NotUtf8)
    ));
}

#[cfg(unix)]
#[test]
fn a_path_that_is_not_utf8_is_refused() {
    use std::os::unix::ffi::OsStrExt;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(std::ffi::OsStr::from_bytes(b"a\xff"));
    if std::fs::write(&path, b"x").is_err() {
        eprintln!("skipped: the file system refuses the name");
        return;
    }
    assert!(matches!(identity(&path), Err(MediaError::NotUtf8)));
    assert!(matches!(probe(&path), Err(MediaError::NotUtf8)));
}

#[test]
fn a_path_with_a_nul_is_refused() {
    let path = Path::new("a\0.mp4");
    assert!(matches!(identity(path), Err(MediaError::NotUtf8)));
    assert!(matches!(probe(path), Err(MediaError::NotUtf8)));
}

/// The seconds that a duration lasts.
fn seconds(facts: &columbina_media::Facts) -> f64 {
    #[allow(clippy::cast_precision_loss)]
    let ticks = facts.duration.ticks as f64;
    ticks / f64::from(facts.duration.scale)
}

#[test]
fn the_1080p_h264_file_with_sound() {
    let Some(made) = media(Sample::Fhd) else {
        return;
    };
    let facts = probe(&made.path).unwrap();
    let video = facts.video.clone().unwrap();
    assert_eq!((video.width, video.height), (1920, 1080));
    assert_eq!(video.rate, Rate { num: 25, den: 1 });
    assert_eq!(
        (video.codec.as_str(), video.bits, video.rotation),
        ("h264", 8, 0)
    );
    let audio = facts.audio.clone().unwrap();
    assert_eq!(
        (audio.rate, audio.channels, audio.codec.as_str()),
        (48000, 2, "aac")
    );
    // The duration of the picture, in the time base of the picture: 100 frames.
    assert!((seconds(&facts) - 4.0).abs() < 1e-9, "{:?}", facts.duration);
    assert_eq!(facts.duration.ticks % 100, 0, "{:?}", facts.duration);
}

#[test]
fn the_4k_hevc_main10_file() {
    let Some(made) = media(Sample::Uhd) else {
        return;
    };
    eprintln!("encoded with {}", made.encoder);
    let facts = probe(&made.path).unwrap();
    let video = facts.video.clone().unwrap();
    assert_eq!((video.width, video.height), (3840, 2160));
    assert_eq!(video.rate, Rate { num: 60, den: 1 });
    assert_eq!(
        (video.codec.as_str(), video.bits, video.rotation),
        ("hevc", 10, 0)
    );
    assert_eq!(facts.audio, None);
    assert!((seconds(&facts) - 3.0).abs() < 1e-9, "{:?}", facts.duration);
}

#[test]
fn the_prores_file() {
    let Some(made) = media(Sample::Prores) else {
        return;
    };
    let facts = probe(&made.path).unwrap();
    let video = facts.video.clone().unwrap();
    assert_eq!((video.width, video.height), (1920, 1080));
    assert_eq!(video.rate, Rate { num: 25, den: 1 });
    assert_eq!(
        (video.codec.as_str(), video.bits, video.rotation),
        ("prores", 10, 0)
    );
    assert_eq!(facts.audio, None);
    assert!((seconds(&facts) - 2.0).abs() < 1e-9, "{:?}", facts.duration);
}

#[test]
fn the_portrait_file_is_turned_clockwise_by_270_to_be_shown() {
    let Some(made) = media(Sample::Portrait) else {
        return;
    };
    let facts = probe(&made.path).unwrap();
    let video = facts.video.clone().unwrap();
    // As it is stored; `-display_rotation 90` turns it a quarter turn counterclockwise, which
    // is three quarters clockwise.
    assert_eq!((video.width, video.height), (640, 360));
    assert_eq!(video.rotation, 270);
    assert_eq!(video.rate, Rate { num: 30, den: 1 });
    assert!((seconds(&facts) - 2.0).abs() < 1e-9, "{:?}", facts.duration);
}

#[test]
fn the_wav_file_has_sound_only_and_lasts_in_samples() {
    let Some(made) = media(Sample::Wav) else {
        return;
    };
    let facts = probe(&made.path).unwrap();
    assert_eq!(facts.video, None);
    let audio = facts.audio.clone().unwrap();
    assert_eq!(
        (audio.rate, audio.channels, audio.codec.as_str()),
        (48000, 2, "pcm_s16le")
    );
    assert_eq!(
        facts.duration,
        columbina_media::Time {
            ticks: 96000,
            scale: 48000
        }
    );
    let error = VideoDecoder::open(&made.path, Hardware::Auto)
        .err()
        .unwrap();
    assert!(matches!(error, MediaError::NoVideo), "{error}");
}

#[test]
fn a_relative_path_is_read_as_the_absolute_one() {
    let Some(made) = media(Sample::Wav) else {
        return;
    };
    let current = std::env::current_dir().unwrap();
    let Ok(relative) = made.path.strip_prefix(&current) else {
        return;
    };
    assert_eq!(probe(relative).unwrap(), probe(&made.path).unwrap());
}

#[test]
fn every_sample_is_probed_twice_alike() {
    for sample in Sample::ALL {
        let Some(made) = media(sample) else {
            return;
        };
        assert_eq!(probe(&made.path).unwrap(), probe(&made.path).unwrap());
        assert_eq!(identity(&made.path).unwrap(), identity(&made.path).unwrap());
    }
}

#[test]
fn the_duration_of_mpeg_ts_and_ps_counts_from_the_start_of_the_picture() {
    for sample in [
        Sample::TsH264,
        Sample::TsMpeg2,
        Sample::PsMpeg2,
        Sample::M2ts,
    ] {
        let Some(made) = media(sample) else {
            return;
        };
        let facts = probe(&made.path).unwrap();
        assert_eq!(facts.duration.scale, 90000, "{sample:?}");
        assert!(
            (seconds(&facts) - 4.0).abs() < 1e-9,
            "{sample:?}: {:?}",
            facts.duration
        );
        let video = facts.video.unwrap();
        assert_eq!(video.rate, Rate { num: 25, den: 1 }, "{sample:?}");
        let codec = if matches!(sample, Sample::TsMpeg2 | Sample::PsMpeg2) {
            "mpeg2video"
        } else {
            "h264"
        };
        assert_eq!(video.codec, codec);
    }
}

#[test]
fn the_duration_of_matroska_is_that_of_its_picture_from_its_start() {
    // 4 s of picture and 6 s of sound; then 4 s of picture from 1 s, and 5 s of sound from 0:
    // Matroska says how long the whole file lasts, and not how long the picture does.
    for sample in [Sample::MkvLongSound, Sample::MkvLateStart] {
        let Some(made) = media(sample) else {
            return;
        };
        let facts = probe(&made.path).unwrap();
        assert_eq!(
            facts.duration,
            columbina_media::Time {
                ticks: 4000,
                scale: 1000
            },
            "{sample:?}"
        );
        assert!(facts.audio.is_some());
    }
}

#[test]
fn the_duration_of_webm_is_that_of_its_picture() {
    let Some(made) = media(Sample::Webm) else {
        return;
    };
    let facts = probe(&made.path).unwrap();
    assert!((seconds(&facts) - 2.0).abs() < 1e-9, "{:?}", facts.duration);
    assert_eq!(facts.video.unwrap().codec, "vp9");
    assert_eq!(facts.audio.unwrap().codec, "opus");
}

#[test]
fn a_file_whose_frames_come_at_changing_times_is_numbered_at_the_standard_rate_nearest() {
    let Some(made) = media(Sample::Vfr) else {
        return;
    };
    let facts = probe(&made.path).unwrap();
    assert_eq!(facts.video.unwrap().rate, Rate { num: 25, den: 1 });
}

#[test]
fn a_picture_that_cannot_be_decoded_is_refused_and_not_taken_for_sound_alone() {
    let Some(made) = media(Sample::Evc) else {
        return;
    };
    let error = probe(&made.path).unwrap_err();
    assert_eq!(error.reason(), "unsupported", "{error}");
    let error = VideoDecoder::open(&made.path, Hardware::Off).err().unwrap();
    assert_eq!(error.reason(), "unsupported", "{error}");
}

#[test]
fn a_list_of_other_files_is_not_media() {
    let Some(made) = media(Sample::Fhd) else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let target = made.path.to_str().unwrap().replace('\\', "/");
    let lists = [
        (
            "list.m3u8",
            format!(
                "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:4\n#EXTINF:4.0,\n{target}\n#EXT-X-ENDLIST\n"
            ),
        ),
        ("list.ffconcat", format!("ffconcat version 1.0\nfile '{target}'\n")),
        ("list.txt", format!("file '{target}'\n")),
    ];
    for (name, text) in lists {
        let path = directory.path().join(name);
        std::fs::write(&path, text).unwrap();
        let error = probe(&path).unwrap_err();
        assert_eq!(error.reason(), "not_media", "{name}: {error}");
        assert!(VideoDecoder::open(&path, Hardware::Off).is_err());
    }
}
