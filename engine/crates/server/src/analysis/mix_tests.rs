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
    let single = make_wave(&media, Some(0), None, &running).unwrap();
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

#[tokio::test]
async fn jobs_deduplicate_mix_requests_notify_the_exact_selection_and_cache_offline() {
    let directory = tempfile::tempdir().unwrap();
    let media = pcm(directory.path(), |_| [24576, -8192]);
    let (sent, mut events) = tokio::sync::mpsc::unbounded_channel();
    let analysis = Analysis::new(
        directory.path(),
        Box::new(move |media, stream, mix| {
            sent.send((media.id.clone(), stream, mix.map(<[ChannelSource]>::to_vec)))
                .unwrap();
        }),
    );
    assert!(analysis
        .waveform(media.clone(), Some(0), Some(stereo()))
        .await
        .is_err());
    assert!(analysis
        .waveform(media.clone(), None, Some(vec![]))
        .await
        .is_err());
    let (pending, job) = analysis
        .waveform(media.clone(), None, Some(stereo()))
        .await
        .unwrap();
    assert_eq!(pending, json!({"pending":true}));
    let (_, shared) = analysis
        .waveform(media.clone(), None, Some(stereo()))
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
        .waveform(media.clone(), None, Some(stereo()))
        .await
        .unwrap();
    assert!(job.is_none());
    assert_eq!(
        cached,
        json!({"rate":100,"channels":1,"peaks":STANDARD.encode([32; 4])})
    );
    // A different mix never consumes the first mix's cache, even while the file is offline.
    let (_, different) = analysis
        .waveform(media, None, Some(stereo()[..1].to_vec()))
        .await
        .unwrap();
    assert!(different.is_some());
    analysis.stop();
}
