use super::*;
use base64::{engine::general_purpose::STANDARD, Engine as _};

async fn mixed(client: &mut Client, sources: Value) -> Value {
    let params = json!({"media":"m1","mix":sources});
    waveform(client, params).await
}

async fn waveform(client: &mut Client, params: Value) -> Value {
    let answer = client.result("media.waveform", Some(params.clone())).await;
    if answer["pending"] != true {
        return answer;
    }
    let ready = |note: &Value| note["method"] == "media.waveform" && note["params"] == params;
    if !client.take_notifications().iter().any(ready) {
        loop {
            if ready(&client.message().await) {
                break;
            }
        }
    }
    client.result("media.waveform", Some(params)).await
}

#[tokio::test]
async fn automatic_remix_matches_playback_decoder_and_preserves_default_peaks_and_project() {
    let made = support::media(support::Sample::Mono).expect("local PCM fixture");
    let directory = project_dir();
    let path = directory.path().join("source.wav");
    std::fs::copy(&made.path, &path).unwrap();
    let engine = engine(directory.path(), &["--headless", "--idle-seconds", "1"]).await;
    let mut client = engine.client("agent").await;
    client
        .result("media.import", Some(json!({"paths":[path]})))
        .await;
    let before = client.result("project.get", None).await;
    let original = waveform(&mut client, json!({"media":"m1"})).await;
    assert_eq!(original["channels"], 1);
    let result = waveform(&mut client, json!({"media":"m1","remix":2})).await;
    assert_eq!(result["channels"], 2);
    let mut decoder = columbina_media::AudioDecoder::open(
        &path,
        Some(columbina_media::AudioFormat {
            rate: 48000,
            channels: 2,
        }),
    )
    .unwrap();
    let mut samples = vec![];
    while let Some(packet) = decoder.next().unwrap() {
        samples.extend(packet.data);
    }
    drop(decoder);
    let mut expected = vec![];
    for bin in samples.chunks(480 * 2) {
        for channel in 0..2 {
            let quantized = bin
                .iter()
                .skip(channel)
                .step_by(2)
                .map(|value| {
                    #[allow(
                        clippy::cast_possible_truncation,
                        reason = "rounded to signed byte range"
                    )]
                    let sample = (value.clamp(-1.0, 1.0) * 127.0).round() as i8;
                    sample
                })
                .collect::<Vec<_>>();
            expected.push(quantized.iter().min().unwrap().cast_unsigned());
            expected.push(quantized.iter().max().unwrap().cast_unsigned());
        }
    }
    assert_eq!(
        STANDARD.decode(result["peaks"].as_str().unwrap()).unwrap(),
        expected
    );
    assert_eq!(client.result("project.get", None).await, before);
    std::fs::remove_file(&path).unwrap();
    assert_eq!(waveform(&mut client, json!({"media":"m1"})).await, original);
    assert_eq!(
        waveform(&mut client, json!({"media":"m1","remix":2})).await,
        result
    );
    drop(client);
    assert_eq!(engine.ended().await, 0);
}

fn native(path: &Path, stream: usize) -> Vec<f32> {
    let mut decoder = columbina_media::AudioDecoder::open_unmixed(path, stream, None).unwrap();
    let mut samples = vec![];
    while let Some(packet) = decoder.next().unwrap() {
        samples.extend(packet.data);
    }
    samples
}

#[tokio::test]
async fn mixed_waveforms_match_real_cross_stream_pcm_and_leave_the_project_unchanged() {
    let made = support::media(support::Sample::TwoMonoStreams).expect("local PCM fixture");
    let directory = project_dir();
    let path = directory.path().join("source.mov");
    std::fs::copy(&made.path, &path).unwrap();
    let engine = engine(directory.path(), &["--headless", "--idle-seconds", "1"]).await;
    let mut client = engine.client("agent").await;
    client
        .result("media.import", Some(json!({"paths":[path]})))
        .await;
    let before = client.result("project.get", None).await;
    let sources = json!([{"stream":0,"channel":0},{"stream":1,"channel":0}]);
    let result = mixed(&mut client, sources.clone()).await;
    assert_eq!(result["channels"], 1);
    assert_eq!(result["rate"], 100);
    let (left, right) = (native(&path, 0), native(&path, 1));
    assert_eq!(left.len(), right.len());
    let mut expected = vec![];
    for (a, b) in left.chunks(480).zip(right.chunks(480)) {
        let quantized = a
            .iter()
            .zip(b)
            .map(|(a, b)| {
                #[allow(
                    clippy::cast_possible_truncation,
                    reason = "rounded to signed byte range"
                )]
                let sample = (((a + b) * 0.5).clamp(-1.0, 1.0) * 127.0).round() as i8;
                sample
            })
            .collect::<Vec<_>>();
        expected.push(quantized.iter().min().unwrap().cast_unsigned());
        expected.push(quantized.iter().max().unwrap().cast_unsigned());
    }
    assert_eq!(
        STANDARD.decode(result["peaks"].as_str().unwrap()).unwrap(),
        expected
    );
    assert_eq!(client.result("project.get", None).await, before);
    // Cache hits, including after the original goes offline, retain the exact source list.
    std::fs::remove_file(&path).unwrap();
    assert_eq!(mixed(&mut client, sources).await, result);
    drop(client);
    assert_eq!(engine.ended().await, 0);
}
