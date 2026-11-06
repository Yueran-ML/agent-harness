use super::*;

fn pcm(directory: &Path, sample: impl Fn(usize) -> [i16; 2]) -> Media {
    let path = directory.join("source.wav");
    let frames = 960_u32;
    let size = frames * 4;
    let mut file = std::fs::File::create(&path).unwrap();
    file.write_all(b"RIFF").unwrap();
    file.write_all(&(36 + size).to_le_bytes()).unwrap();
    file.write_all(b"WAVEfmt \x10\0\0\0\x01\0\x02\0").unwrap();
    file.write_all(&48000_u32.to_le_bytes()).unwrap();
    file.write_all(&192_000_u32.to_le_bytes()).unwrap();
    file.write_all(b"\x04\0\x10\0data").unwrap();
    file.write_all(&size.to_le_bytes()).unwrap();
    for n in 0..frames {
        for value in sample(usize::try_from(n).unwrap()) {
            file.write_all(&value.to_le_bytes()).unwrap();
        }
    }
    drop(file);
    let mut media = columbina_model::Project::sample().media.remove(0);
    media.path = path.to_string_lossy().into_owned();
    media.facts = columbina_media::probe(&path).unwrap();
    media
}

fn stereo() -> Vec<ChannelSource> {
    vec![
        ChannelSource {
            stream: 0,
            channel: 0,
        },
        ChannelSource {
            stream: 0,
            channel: 1,
        },
    ]
}

#[test]
fn mixing_samples_preserves_cancellation_that_channel_peak_sums_cannot_recover() {
    let directory = tempfile::tempdir().unwrap();
    let media = pcm(directory.path(), |n| {
        let value = if n % 2 == 0 { 24576 } else { -16384 };
        [value, -value]
    });
    let running = AtomicBool::new(false);
    let single = make_wave(&media, Some(0), None, None, &running).unwrap();
    assert!(single.peaks.iter().any(|value| *value != 0));
    let mixed = make_mix(&media, &stereo(), &running).unwrap();
    assert_eq!(mixed.channels, 1);
    assert_eq!(mixed.peaks, vec![0; 4]);
}

#[test]
fn the_mean_is_computed_before_quantizing_and_cancellation_stops_analysis() {
    let directory = tempfile::tempdir().unwrap();
    let media = pcm(directory.path(), |_| [24576, -8192]);
    let running = AtomicBool::new(false);
    assert_eq!(
        make_mix(&media, &stereo(), &running).unwrap().peaks,
        vec![32; 4]
    );
    assert_eq!(
        make_mix(&media, &stereo()[..1], &running).unwrap().peaks,
        vec![95; 4]
    );
    assert!(make_mix(
        &media,
        &[ChannelSource {
            stream: 0,
            channel: 2
        }],
        &running
    )
    .is_err());
    running.store(true, Ordering::Release);
    assert!(make_mix(&media, &stereo(), &running).is_err());
}

#[test]
fn automatic_remix_uses_playback_layout_weights_instead_of_the_explicit_mix_mean() {
    let directory = tempfile::tempdir().unwrap();
    let media = pcm(directory.path(), |_| [24576, -8192]);
    let running = AtomicBool::new(false);
    let wave = make_wave(&media, None, Some(1), None, &running).unwrap();
    assert_eq!(wave.channels, 1);
    // Stereo to mono has equal-power weights: (.75 - .25) / sqrt(2).
    assert_eq!(wave.peaks, vec![45; 4]);
    assert_eq!(
        make_mix(&media, &stereo(), &running).unwrap().peaks,
        vec![32; 4]
    );
    let native = make_wave(&media, None, None, None, &running).unwrap();
    assert_eq!(native.channels, 2);
    assert_eq!(
        native.peaks,
        [95_i8, 95, -32, -32, 95, 95, -32, -32].map(i8::cast_unsigned)
    );
    running.store(true, Ordering::Release);
    assert!(make_wave(&media, None, Some(1), None, &running).is_err());
}

#[tokio::test]
async fn remix_jobs_validate_layouts_and_keep_distinct_offline_caches_and_notifications() {
    let directory = tempfile::tempdir().unwrap();
    let media = pcm(directory.path(), |_| [24576, -8192]);
    let (sent, mut events) = tokio::sync::mpsc::unbounded_channel();
    let analysis = Analysis::new(
        directory.path(),
        Box::new(move |media, stream, mix, remix| {
            sent.send((
                media.id.clone(),
                stream,
                mix.map(<[ChannelSource]>::to_vec),
                remix,
            ))
            .unwrap();
        }),
    );
    for channels in [0, 25] {
        assert!(analysis
            .waveform(media.clone(), None, None, Some(channels))
            .await
            .is_err());
    }
    assert!(analysis
        .waveform(media.clone(), Some(0), None, Some(1))
        .await
        .is_err());
    assert!(analysis
        .waveform(media.clone(), None, Some(stereo()), Some(1))
        .await
        .is_err());
    for channels in [1, 2] {
        let (_, job) = analysis
            .waveform(media.clone(), None, None, Some(channels))
            .await
            .unwrap();
        let (_, shared) = analysis
            .waveform(media.clone(), None, None, Some(channels))
            .await
            .unwrap();
        assert!(shared.is_none());
        job.unwrap().start();
        let notice = tokio::time::timeout(std::time::Duration::from_secs(5), events.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(notice, (media.id.clone(), None, None, Some(channels)));
    }
    std::fs::remove_file(&media.path).unwrap();
    for channels in [1, 2] {
        let (cached, job) = analysis
            .waveform(media.clone(), None, None, Some(channels))
            .await
            .unwrap();
        assert!(job.is_none());
        assert_eq!(cached["channels"], channels);
        let bytes = STANDARD.decode(cached["peaks"].as_str().unwrap()).unwrap();
        assert_eq!(bytes.len(), usize::from(channels) * 4);
    }
    for (mix, remix) in [(None, None), (Some(stereo()), None), (None, Some(6))] {
        let (_, job) = analysis
            .waveform(media.clone(), None, mix, remix)
            .await
            .unwrap();
        assert!(job.is_some());
    }
    analysis.stop();
}

#[tokio::test]
async fn jobs_deduplicate_mix_requests_notify_the_exact_selection_and_cache_offline() {
    let directory = tempfile::tempdir().unwrap();
    let media = pcm(directory.path(), |_| [24576, -8192]);
    let (sent, mut events) = tokio::sync::mpsc::unbounded_channel();
    let analysis = Analysis::new(
        directory.path(),
        Box::new(move |media, stream, mix, _remix| {
            sent.send((media.id.clone(), stream, mix.map(<[ChannelSource]>::to_vec)))
                .unwrap();
        }),
    );
    assert!(analysis
        .waveform(media.clone(), Some(0), Some(stereo()), None)
        .await
        .is_err());
    assert!(analysis
        .waveform(media.clone(), None, Some(vec![]), None)
        .await
        .is_err());
    let (pending, job) = analysis
        .waveform(media.clone(), None, Some(stereo()), None)
        .await
        .unwrap();
    assert_eq!(pending, json!({"pending":true}));
    let (_, shared) = analysis
        .waveform(media.clone(), None, Some(stereo()), None)
        .await
        .unwrap();
    assert!(shared.is_none());
    job.unwrap().start();
    let notice = tokio::time::timeout(std::time::Duration::from_secs(5), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(notice, (media.id.clone(), None, Some(stereo())));
    std::fs::remove_file(&media.path).unwrap();
    let (cached, job) = analysis
        .waveform(media.clone(), None, Some(stereo()), None)
        .await
        .unwrap();
    assert!(job.is_none());
    assert_eq!(
        cached,
        json!({"rate":100,"channels":1,"peaks":STANDARD.encode([32; 4])})
    );
    // A different mix never consumes the first mix's cache, even while the file is offline.
    let (_, different) = analysis
        .waveform(media, None, Some(stereo()[..1].to_vec()), None)
        .await
        .unwrap();
    assert!(different.is_some());
    analysis.stop();
}

#[test]
fn long_mix_lists_are_hashed_into_a_short_distinct_cache_name() {
    let media = columbina_model::Project::sample().media.remove(0);
    let list = |count: u32| -> Vec<ChannelSource> {
        (0..count)
            .map(|channel| ChannelSource {
                stream: 0,
                channel: channel % 10,
            })
            .collect()
    };
    // 16 sources of "0.c" joined by "-" are 63 characters, written as they are.
    let short = mix_key(&media, &list(16));
    assert!(
        short.ends_with("-mix-0.0-0.1-0.2-0.3-0.4-0.5-0.6-0.7-0.8-0.9-0.0-0.1-0.2-0.3-0.4-0.5.bin")
    );
    let long = mix_key(&media, &list(17));
    let huge = mix_key(&media, &list(5000));
    assert!(
        long.contains("-mix-h") && huge.contains("-mix-h"),
        "{long} {huge}"
    );
    assert_ne!(long, huge);
    assert_eq!(long.len(), huge.len());
    assert!(huge.len() < 100, "{huge}");
    assert_eq!(huge, mix_key(&media, &list(5000)));
}
