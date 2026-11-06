//! Source-monitor audio uses the same channel maps and routing as placed clips.
//! The temporary timeline never enters the project or its undo history. Building its graph
//! and reading its files happen on the existing output feeder, away from the UI/device thread.

use std::sync::Arc;

use columbina_audio::graph::Purpose;
use columbina_media::AudioFormat;
use columbina_model::{Media, Project, Rate, Time};

use crate::play::mix::Mixer;

/// The temporary timeline, or why the layout cannot be played.
pub(super) fn project(media: &Media, rate: Rate) -> Result<Project, String> {
    let length = super::clock::last_frame(media.facts.duration, rate).saturating_add(1);
    crate::play::mix::raw::project(
        media,
        rate,
        length,
        media.audio_config.as_deref().unwrap_or_default(),
        None,
    )
}

/// Renaming, source marks and proxy completion do not restart the sound.
pub(super) fn same(left: Option<&Media>, right: Option<&Media>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => {
            left.id == right.id
                && left.path == right.path
                && left.identity == right.identity
                && left.facts == right.facts
                && left.audio_config == right.audio_config
        }
        _ => false,
    }
}

pub(super) fn source(
    media: &Media,
    rate: Rate,
    place: i128,
    format: AudioFormat,
) -> Result<Box<dyn crate::audio::Source>, String> {
    let project = project(media, rate)?;
    let nanos = super::clock::Clock::at(place, 1, rate, 0).media_nanos(0);
    let from = super::clock::sample_at(
        Time { ticks: 0, scale: 1 },
        nanos,
        project.timelines[0].sample_rate,
    );
    Ok(Mixer::new(Arc::new(project), 0, Purpose::Preview).source(format, from))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn configured() -> Media {
        let made = crate::test_media::media(crate::test_media::Sample::LeftRight)
            .expect("the local ffmpeg stereo PCM fixture");
        let mut media = Project::sample().media.remove(0);
        media.path = made.path.to_string_lossy().into_owned();
        media.facts = columbina_media::probe(&made.path).unwrap();
        media.audio_config = Some(
            serde_json::from_value(json!([{
                "name":"Stereo", "format":"stereo", "channels":[
                    {"source":{"stream":0,"channel":1}},
                    {"source":{"stream":0,"channel":0}}
                ]
            }]))
            .unwrap(),
        );
        media
    }

    fn read(media: &Media, format: AudioFormat, frame: i64) -> Vec<f32> {
        let rate = super::super::GRID;
        let place = super::super::clock::Clock::starting(frame, 1, rate, 0).place(0);
        let mut source = source(media, rate, place, format).unwrap();
        let mut output = Vec::new();
        while let Some(block) = source.next().unwrap() {
            assert!(
                block.len()
                    <= usize::try_from(format.rate / 50).unwrap() * usize::from(format.channels)
            );
            output.extend(block);
        }
        output
    }

    #[allow(
        clippy::cast_precision_loss,
        reason = "short deterministic test signal"
    )]
    fn amplitude(pcm: &[f32], format: AudioFormat, channel: usize, hertz: f64) -> f64 {
        let values: Vec<_> = pcm
            .iter()
            .skip(channel)
            .step_by(usize::from(format.channels))
            .skip(usize::try_from(format.rate / 2).unwrap())
            .take(usize::try_from(format.rate).unwrap())
            .copied()
            .collect();
        let step = std::f64::consts::TAU * hertz / f64::from(format.rate);
        let (sin, cos) = values
            .iter()
            .enumerate()
            .fold((0.0, 0.0), |(sin, cos), (i, sample)| {
                (
                    sin + f64::from(*sample) * (i as f64 * step).sin(),
                    cos + f64::from(*sample) * (i as f64 * step).cos(),
                )
            });
        2.0 * sin.hypot(cos) / values.len() as f64
    }

    #[test]
    fn configured_playback_uses_real_channel_choices_mute_and_trim() {
        let mut media = configured();
        let format = AudioFormat {
            rate: 48000,
            channels: 2,
        };
        let pcm = read(&media, format, 0);
        assert!((amplitude(&pcm, format, 0, 880.0) - 0.25).abs() < 1e-3);
        assert!(amplitude(&pcm, format, 0, 440.0) < 1e-5);
        assert!((amplitude(&pcm, format, 1, 440.0) - 0.25).abs() < 1e-3);
        let channels = &mut media.audio_config.as_mut().unwrap()[0].channels;
        channels[0].level_db = -6.020_599_913_279_624;
        channels[1].mute = true;
        let pcm = read(&media, format, 0);
        assert!((amplitude(&pcm, format, 0, 880.0) - 0.125).abs() < 1e-3);
        assert!(pcm.iter().skip(1).step_by(2).all(|sample| *sample == 0.0));
    }

    #[test]
    fn sample_rate_conversion_and_nonzero_start_end_at_the_source_boundary() {
        let media = configured();
        for rate in [44100, 48000, 96000] {
            let format = AudioFormat { rate, channels: 2 };
            for frame in [0, 25, 26] {
                let pcm = read(&media, format, frame);
                let end =
                    super::super::clock::last_frame(media.facts.duration, super::super::GRID) + 1;
                let samples = (end - frame) * i64::from(rate) / 25;
                assert_eq!(
                    pcm.len(),
                    usize::try_from(samples * 2).unwrap(),
                    "{rate} Hz, frame {frame}"
                );
                assert!((amplitude(&pcm, format, 0, 880.0) - 0.25).abs() < 1e-3);
            }
        }
    }

    #[test]
    fn an_unplayable_layout_is_an_error_for_the_feeder_not_a_panic() {
        let mut media = Project::sample().media.remove(0);
        media.audio_config = Some(
            serde_json::from_value(json!([{
                "name": "Wide", "format": "adaptive",
                "channels": vec![json!({"source": {"stream": 0, "channel": 0}}); 256]
            }]))
            .unwrap(),
        );
        let format = AudioFormat {
            rate: 48000,
            channels: 2,
        };
        assert!(source(&media, super::super::GRID, 0, format).is_err());
    }

    #[test]
    fn projection_drops_inactive_channels_and_never_changes_the_media_or_project() {
        let mut media = configured();
        media.bin = Some("interviews".into());
        media.audio_config.as_mut().unwrap()[0].channels[0].enabled = false;
        let before = media.clone();
        let projected = project(&media, super::super::GRID).unwrap();
        assert_eq!(media, before);
        assert_eq!(projected.timelines[0].tracks.len(), 1);
        assert_eq!(
            projected.timelines[0].tracks[0].audio.format,
            columbina_model::audio::TrackFormat::Mono
        );
        assert_eq!(
            projected.timelines[0].tracks[0].clips[0]
                .channels
                .as_ref()
                .unwrap()
                .len(),
            1
        );
        assert!(projected.media[0].bin.is_none());
        media.name = "Renamed".into();
        media.mark_in = Some(Time {
            ticks: 10,
            scale: 48000,
        });
        assert!(same(Some(&before), Some(&media)));
        media.audio_config = Some(vec![]);
        assert!(!same(Some(&before), Some(&media)));
        assert_eq!(
            read(
                &media,
                AudioFormat {
                    rate: 48000,
                    channels: 2
                },
                0
            ),
            [] as [f32; 0]
        );
    }

    #[tokio::test]
    async fn loaded_source_hears_configuration_changes_and_restores_the_original_decoder() {
        use crate::transport::{lock, Command, Setup, Transport};
        use std::sync::Mutex;
        use std::time::Duration;

        async fn wait(mut ready: impl FnMut() -> bool) {
            tokio::time::timeout(Duration::from_secs(8), async {
                while !ready() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("source transport did not reach the expected state");
        }

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("source.f32");
        let transport = Transport::new(
            false,
            Setup {
                sound: crate::audio::Kind::Capture(path.clone()),
                fail_card_at: None,
                test_hooks: false,
            },
            Arc::new(Mutex::new(None)),
            Arc::new(|_| {}),
            Arc::new(|_, _| {}),
            crate::scopes::Scopes::new(true, crate::gpu::Graphics::new()),
        );
        let mut media = configured();
        transport
            .begin_load(&media, None, Some(super::super::GRID))
            .unwrap()
            .done()
            .await
            .unwrap();
        transport.send(Command::Play { speed: 1 }).unwrap();
        wait(|| transport.status().frame.is_some_and(|frame| frame >= 10)).await;
        // Ordinary project metadata must not interrupt the playing session.
        media.name = "Renamed while playing".into();
        let mut project = Project::sample();
        project.media = vec![media.clone()];
        transport.project_changed(&project);
        wait(|| transport.status().frame.is_some_and(|frame| frame >= 20)).await;
        assert!(!crate::audio::capture_file(&path, 2).exists());

        let channels = &mut media.audio_config.as_mut().unwrap()[0].channels;
        channels[0].level_db = -6.020_599_913_279_624;
        channels[1].mute = true;
        project.media = vec![media.clone()];
        transport.project_changed(&project);
        wait(|| transport.status().frame.is_some_and(|frame| frame >= 45)).await;
        transport.pause_for_switch().await.unwrap();

        media.audio_config = None;
        project.media = vec![media];
        transport.project_changed(&project);
        let seek = serde_json::value::to_raw_value(&json!({"frame":0})).unwrap();
        transport
            .prepare("transport.seek", Some(&seek))
            .unwrap()
            .1
            .send();
        transport.send(Command::Play { speed: 1 }).unwrap();
        // The pause acknowledgment above still exposes the old frame until seek is handled.
        wait(|| transport.status().playing && transport.status().frame.is_some_and(|n| n < 10))
            .await;
        wait(|| transport.status().frame.is_some_and(|frame| frame >= 20)).await;
        transport.stop();
        wait(|| lock(&transport.shared).stopped).await;

        let format = AudioFormat {
            rate: 48000,
            channels: 2,
        };
        let captures: Vec<Vec<f32>> = (1..=3)
            .map(|session| {
                std::fs::read(crate::audio::capture_file(&path, session))
                    .unwrap()
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|word| f32::from_le_bytes(*word))
                    .collect()
            })
            .collect();
        assert!((amplitude(&captures[0], format, 0, 880.0) - 0.25).abs() < 1e-3);
        assert!((amplitude(&captures[1], format, 0, 880.0) - 0.125).abs() < 1e-3);
        assert!(captures[1]
            .iter()
            .skip(1)
            .step_by(2)
            .all(|sample| *sample == 0.0));
        assert!((amplitude(&captures[2], format, 0, 440.0) - 0.25).abs() < 1e-3);
        assert!((amplitude(&captures[2], format, 1, 880.0) - 0.25).abs() < 1e-3);
        assert!(!crate::audio::capture_file(&path, 4).exists());
    }
}
