use super::*;
use crate::test_media::{media, Sample};
use columbina_model::Project;
use serde_json::json;

fn input(sample: Sample) -> Media {
    let made = media(sample).expect("the local PCM fixture must be available");
    let mut input = Project::sample().media.remove(0);
    input.path = made.path.to_string_lossy().into_owned();
    input.facts = columbina_media::probe(&made.path).unwrap();
    input
}

fn mapped(channels: u16) -> Clip {
    let mut clip = Project::sample().timelines[0].tracks[1].clips[0].clone();
    clip.channels = Some(
        (0..channels)
            .map(|channel| {
                serde_json::from_value(json!({
                    "remix":{"channels":channels,"channel":channel}
                }))
                .unwrap()
            })
            .collect(),
    );
    clip
}

#[test]
fn retained_automatic_outputs_match_auto_across_layouts_forward_seeks_and_rewinds() {
    for (sample, channels) in [
        (Sample::Mono, 2),
        (Sample::LeftRight, 1),
        (Sample::SixSines, 2),
        (Sample::SixSines, 6),
        (Sample::MkvOffsetAudio, 2),
    ] {
        let input = input(sample);
        let mut original = Reader::new(&input, 48000, &Layout::Auto(channels)).unwrap();
        let mut retained =
            Reader::new(&input, 48000, &Layout::of_clip(&mapped(channels), channels)).unwrap();
        // All output lanes share one automatic decoder, not one decoder per channel.
        assert_eq!(retained.source.len(), 1);
        for range in [0..4096, 4096..8192, 60000..64096, 20000..24096, 0..4096] {
            assert_eq!(
                retained.read(range.clone()).unwrap(),
                original.read(range).unwrap()
            );
        }
    }
}

#[test]
fn replacing_one_upmixed_output_keeps_the_other_output_at_its_original_gain() {
    let input = input(Sample::Mono);
    let mut clip = mapped(2);
    clip.channels.as_mut().unwrap()[1] =
        serde_json::from_value(json!({"source":{"stream":0,"channel":0}})).unwrap();
    let mut actual = Reader::new(&input, 48000, &Layout::of_clip(&clip, 2)).unwrap();
    let mut original = Reader::new(&input, 48000, &Layout::Auto(2)).unwrap();
    let mut native = Reader::new(&input, 48000, &Layout::Auto(1)).unwrap();
    assert_eq!(actual.source.len(), 2);
    let actual = actual.read(0..48000).unwrap();
    let original = original.read(0..48000).unwrap();
    let native = native.read(0..48000).unwrap();
    for ((actual, original), native) in actual
        .as_chunks::<2>()
        .0
        .iter()
        .zip(original.as_chunks::<2>().0.iter())
        .zip(native)
    {
        assert_eq!(actual[0], original[0]);
        assert_eq!(actual[1], native);
    }
}

#[test]
fn automatic_output_levels_mute_and_distinct_conversion_layouts_are_independent() {
    let input = input(Sample::SixSines);
    let mut clip = mapped(2);
    let maps = clip.channels.as_mut().unwrap();
    maps[0].level_db = -6.0;
    maps[1].remix = Some(RemixSource {
        channels: 6,
        channel: 4,
    });
    let mut actual = Reader::new(&input, 48000, &Layout::of_clip(&clip, 2)).unwrap();
    assert_eq!(actual.source.len(), 2);
    let two = Reader::new(&input, 48000, &Layout::Auto(2))
        .unwrap()
        .read(0..4096)
        .unwrap();
    let six = Reader::new(&input, 48000, &Layout::Auto(6))
        .unwrap()
        .read(0..4096)
        .unwrap();
    let gain = Lane::mean(vec![], true, false, -6.0).gain;
    for ((actual, two), six) in actual
        .read(0..4096)
        .unwrap()
        .as_chunks::<2>()
        .0
        .iter()
        .zip(two.as_chunks::<2>().0.iter())
        .zip(six.as_chunks::<6>().0.iter())
    {
        assert_eq!(actual[0], two[0] * gain);
        assert_eq!(actual[1], six[4]);
    }
    clip.channels.as_mut().unwrap()[0].mute = true;
    clip.channels.as_mut().unwrap()[1].enabled = false;
    let silent = Reader::new(&input, 48000, &Layout::of_clip(&clip, 2))
        .unwrap()
        .read(0..4096)
        .unwrap();
    assert!(silent.iter().all(|sample| *sample == 0.0));
}
