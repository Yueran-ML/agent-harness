use super::*;
use crate::play::mix::{
    reader::{Layout, Reader},
    Mixer,
};
use crate::test_media::{media as fixture, Sample};
use columbina_audio::graph::Purpose;
use columbina_media::AudioFormat;
use serde_json::json;
use std::sync::Arc;

const RATE: Rate = Rate { num: 25, den: 1 };

fn media(sample: Sample) -> Media {
    let file = fixture(sample).expect("the local PCM fixture");
    let mut media = Project::sample().media.remove(0);
    media.path = file.path.to_string_lossy().into_owned();
    media.facts = columbina_media::probe(&file.path).unwrap();
    media
}

fn stereo() -> Vec<MediaAudioTrack> {
    serde_json::from_value(json!([{
        "name":"Stereo", "format":"stereo", "channels":[
            {"source":{"stream":0,"channel":1}, "level_db":-6.020_599_913_279_624},
            {"source":{"stream":0,"channel":0}}
        ]
    }]))
    .unwrap()
}

fn read(project: Project, rate: u32, from: i64, blocks: usize) -> Vec<f32> {
    let mut source = Mixer::new(Arc::new(project), 0, Purpose::Preview)
        .source(AudioFormat { rate, channels: 2 }, from);
    let mut pcm = Vec::new();
    for _ in 0..blocks {
        let Some(block) = source.next().unwrap() else {
            break;
        };
        assert!(block.len() <= usize::try_from(rate / 50).unwrap() * 2);
        pcm.extend(block);
    }
    pcm
}

#[test]
fn solo_keeps_native_pcm_and_channel_level_without_the_other_channel() {
    let media = media(Sample::LeftRight);
    let rows = stereo();
    for selected in [None, Some(0), Some(1)] {
        let projected = project(&media, RATE, 50, &rows, selected).unwrap();
        let clip = &projected.timelines[0].tracks[0].clips[0];
        let mut reader = Reader::new(&media, 48000, &Layout::of_clip(clip, 2)).unwrap();
        let expected = reader.read(24000..33600).unwrap();
        let actual = read(projected, 48000, 24000, 10);
        assert_eq!(actual.len(), expected.len());
        for (actual, expected) in actual.iter().zip(&expected) {
            assert!((actual - expected).abs() < 1e-7);
        }
        for lane in 0..2 {
            let silent = actual
                .iter()
                .skip(lane)
                .step_by(2)
                .all(|sample| *sample == 0.0);
            assert_eq!(silent, selected.is_some_and(|index| index != lane));
        }
    }
}

#[test]
fn solo_across_multiple_rows_is_global_and_never_changes_the_inputs() {
    let media = media(Sample::LeftRight);
    let mut rows = stereo();
    let mut second = rows[0].clone();
    second.channels.swap(0, 1);
    rows.push(second);
    let before_media = media.clone();
    let before_rows = rows.clone();
    let selected = read(
        project(&media, RATE, 50, &rows, Some(2)).unwrap(),
        48000,
        24000,
        10,
    );
    let only_second = read(
        project(&media, RATE, 50, &rows[1..], Some(0)).unwrap(),
        48000,
        24000,
        10,
    );
    assert_eq!(selected, only_second);
    assert!(selected.iter().step_by(2).any(|sample| sample.abs() > 0.1));
    assert!(selected
        .iter()
        .skip(1)
        .step_by(2)
        .all(|sample| *sample == 0.0));
    let all = read(
        project(&media, RATE, 50, &rows, None).unwrap(),
        48000,
        24000,
        10,
    );
    assert!(all
        .iter()
        .skip(1)
        .step_by(2)
        .any(|sample| sample.abs() > 0.1));
    assert_eq!(media, before_media);
    assert_eq!(rows, before_rows);
}

#[test]
fn solo_addresses_original_positions_and_respects_inactive_and_muted_channels() {
    let media = media(Sample::LeftRight);
    let mut rows = stereo();
    rows[0].channels[0].enabled = false;
    for selected in [0, 1] {
        let projected = project(&media, RATE, 50, &rows, Some(selected)).unwrap();
        let pcm = read(projected, 48000, 0, 3);
        assert_eq!(pcm.iter().all(|sample| *sample == 0.0), selected == 0);
    }
    rows[0].channels[1].mute = true;
    assert!(read(
        project(&media, RATE, 50, &rows, Some(1)).unwrap(),
        48000,
        0,
        3
    )
    .iter()
    .all(|sample| *sample == 0.0));
    assert!(project(&media, RATE, 50, &rows, Some(2)).is_err());
    assert!(project(&media, RATE, 50, &[], Some(0)).is_err());
    assert_eq!(
        project(&media, RATE, 50, &[], None).unwrap().timelines[0].tracks,
        [] as [Track; 0]
    );
}

#[test]
fn automatic_outputs_and_nonzero_media_origins_keep_existing_samples() {
    for sample in [Sample::Mono, Sample::MkvOffsetAudio] {
        let media = media(sample);
        let rows: Vec<MediaAudioTrack> = serde_json::from_value(json!([{
            "name":"Auto", "format":"stereo", "channels":[
                {"remix":{"channels":2,"channel":0}},
                {"remix":{"channels":2,"channel":1}}
            ]
        }]))
        .unwrap();
        let mut original = Reader::new(&media, 48000, &Layout::Auto(2)).unwrap();
        let expected = original.read(24000..33600).unwrap();
        let actual = read(
            project(&media, RATE, 50, &rows, Some(1)).unwrap(),
            48000,
            24000,
            10,
        );
        assert_eq!(actual.len(), expected.len());
        for (actual, expected) in actual
            .as_chunks::<2>()
            .0
            .iter()
            .zip(expected.as_chunks::<2>().0)
        {
            assert_eq!(actual[0], 0.0);
            assert!((actual[1] - expected[1]).abs() < 1e-7);
        }
    }
}
