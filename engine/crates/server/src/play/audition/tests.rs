use super::*;
use columbina_model::{AudioParam, Speed};
use serde_json::json;

fn target() -> Target {
    Target::Media { media: "m1".into() }
}
fn at(ticks: i64) -> Time {
    Time { ticks, scale: RATE }
}

fn fixture() -> Project {
    let file = crate::test_media::media(crate::test_media::Sample::LeftRight)
        .expect("local stereo PCM fixture");
    let mut project = Project::sample();
    project.media[0].path = file.path.to_string_lossy().into_owned();
    project.media[0].facts = columbina_media::probe(&file.path).unwrap();
    project
}

#[test]
fn clip_projection_keeps_maps_and_bounds_without_instance_processing() {
    let mut project = Project::sample();
    project.media[0].audio_config = Some(vec![]);
    let track = &mut project.timelines[0].tracks[1];
    track.audio.mute = true;
    track.enabled = false;
    let clip = &mut track.clips[0];
    clip.gain_db = AudioParam::Fixed(18.0);
    clip.pan = AudioParam::Fixed(-1.0);
    clip.fade_in = 20;
    clip.fade_out = 20;
    clip.speed = Speed { num: 2, den: 1 };
    clip.reverse = true;
    clip.enabled = false;
    *clip.source_in_mut().unwrap() = Time {
        ticks: 15360,
        scale: 15360,
    };
    clip.channels = Some(
        serde_json::from_value(json!([
            {"source":{"stream":0,"channel":1},"level_db":-6},
            {"source":{"stream":0,"channel":0},"mute":true}
        ]))
        .unwrap(),
    );
    let before = project.clone();
    let target = Target::Clip {
        timeline: "t1".into(),
        clip: "c2".into(),
    };
    let made = plan(&project, &target, at(-1), Some(0), false).unwrap();
    assert_eq!((made.from, made.end), (48_000, 432_000));
    assert_eq!(made.project.timelines.len(), 1);
    assert_eq!(made.project.timelines[0].tracks.len(), 1);
    let projected = &made.project.timelines[0].tracks[0];
    assert!(projected.enabled);
    assert!(!projected.audio.mute);
    assert_eq!(projected.clips.len(), 1);
    let clip = &projected.clips[0];
    assert_eq!(clip.gain_db, AudioParam::Fixed(0.0));
    assert_eq!(clip.pan, AudioParam::Fixed(0.0));
    assert_eq!((clip.fade_in, clip.fade_out), (0, 0));
    assert_eq!(clip.speed, Speed::ONE);
    assert!(!clip.reverse);
    assert_eq!(
        clip.channels,
        before.timelines[0].tracks[1].clips[0].channels
    );
    assert_eq!(project, before);
}

#[test]
fn film_automatic_lanes_keep_their_speaker_and_media_rows_keep_native_indices() {
    let mut project = Project::sample();
    let track = &mut project.timelines[0].tracks[1];
    track.audio.format = TrackFormat::FiveOne;
    track.audio.channels = 6;
    track.audio.order = ChannelOrder::Film;
    let made = plan(
        &project,
        &Target::Clip {
            timeline: "t1".into(),
            clip: "c2".into(),
        },
        at(0),
        None,
        false,
    )
    .unwrap();
    let channels = made.project.timelines[0].tracks[0].clips[0]
        .channels
        .as_ref()
        .unwrap();
    assert_eq!(
        channels
            .iter()
            .map(|c| c.remix.unwrap().channel)
            .collect::<Vec<_>>(),
        [0, 2, 1, 4, 5, 3]
    );
    project.media[0].facts.audio.as_mut().unwrap().streams =
        Some(vec![columbina_model::AudioStream {
            index: 3,
            channels: 2,
            layout: Some("stereo".into()),
        }]);
    let rows = original_rows(&project.media[0]);
    assert_eq!(
        rows[0].channels[1].source.0,
        [ChannelSource {
            stream: 3,
            channel: 1
        }]
    );
}

#[test]
fn ranges_channels_missing_sources_and_time_are_validated() {
    let project = Project::sample();
    assert!(plan(
        &project,
        &target(),
        Time { ticks: 0, scale: 0 },
        None,
        false
    )
    .is_err());
    assert!(plan(&project, &target(), at(0), Some(2), false).is_err());
    assert!(plan(
        &project,
        &Target::Media {
            media: "absent".into()
        },
        at(0),
        None,
        false
    )
    .is_err());
    let made = plan(&project, &target(), at(i64::MAX), None, false).unwrap();
    assert_eq!(made.from, made.end);
    let made = plan(&project, &target(), at(48000), None, true).unwrap();
    assert_eq!(made.end - made.from, 3840);
    let mut silent = project;
    silent.media[0].facts.audio = None;
    assert!(plan(&silent, &target(), at(0), None, false).is_err());
}

async fn wait(audition: &Audition, ready: impl Fn(&Value) -> bool) -> Value {
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            let state = audition.state();
            if ready(&state) {
                return state;
            }
            tokio::time::sleep(POLL).await;
        }
    })
    .await
    .expect("audition reached expected state")
}

async fn play(audition: &Audition, project: &Project, at: Time, skim: bool) -> u64 {
    audition
        .begin(7, plan(project, &target(), at, None, skim).unwrap())
        .unwrap()
        .await
        .unwrap();
    audition.state()["audition"].as_u64().unwrap()
}

#[tokio::test]
async fn owner_and_id_protect_newer_playback_and_cancellation_stops_output() {
    let project = fixture();
    let audition = Audition::new(audio::Kind::Null);
    let first = play(&audition, &project, at(0), false).await;
    let second = play(&audition, &project, at(0), false).await;
    assert!(second > first);
    assert_eq!(audition.stop(7, first).await.unwrap()["playing"], true);
    assert_eq!(audition.stop(8, second).await.unwrap()["playing"], true);
    audition.cancel(Some(8));
    assert_eq!(audition.state()["playing"], true);
    assert_eq!(audition.stop(7, second).await.unwrap()["playing"], false);
    play(&audition, &project, at(0), false).await;
    audition.cancel(Some(7));
    assert_eq!(audition.state()["playing"], false);
    audition.stopped().await.unwrap();
}

#[tokio::test]
async fn skim_eof_endpoint_decode_failure_and_device_failure_end_status() {
    let project = fixture();
    let audition = Audition::new(audio::Kind::Null);
    play(&audition, &project, at(0), true).await;
    let state = wait(&audition, |s| s["playing"] == false).await;
    assert!(state["error"].is_null(), "{state}");
    assert_eq!(state["position"]["ticks"], 3840);
    play(&audition, &project, at(i64::MAX), false).await;
    assert_eq!(audition.state()["playing"], false);
    let mut broken = project.clone();
    broken.media[0].path = "missing-audition-file.wav".into();
    play(&audition, &broken, at(0), false).await;
    let state = wait(&audition, |s| s["playing"] == false).await;
    assert!(state["error"].is_string(), "{state}");
    let audition = Audition::new(audio::Kind::FailAfter(Duration::from_millis(80)));
    play(&audition, &project, at(0), false).await;
    let state = wait(&audition, |s| s["playing"] == false).await;
    assert!(state["error"].is_string(), "{state}");
}

#[tokio::test]
async fn capture_uses_an_independent_filename_and_stops_at_the_skim_bound() {
    let project = fixture();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("monitor.f32");
    let audition = Audition::new(audio::Kind::Capture(path.clone()));
    play(&audition, &project, at(0), true).await;
    let state = wait(&audition, |s| s["playing"] == false).await;
    assert!(state["error"].is_null(), "{state}");
    audition.stopped().await.unwrap();
    drop(audition);
    let capture = directory.path().join("monitor.f32.audition");
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if std::fs::metadata(&capture).is_ok_and(|m| m.len() == 3840 * 2 * 4) {
                break;
            }
            tokio::time::sleep(POLL).await;
        }
    })
    .await
    .unwrap();
    assert!(!path.exists());
}
