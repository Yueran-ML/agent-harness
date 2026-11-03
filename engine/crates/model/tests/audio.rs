//! The sound of schema 4: what is read where it is left out and written back as it was, what is
//! refused and why, and how a muted, soloed, keyed, faded or crossfaded clip is heard.

use columbina_model::audio::fx::{Ducker, GainFx};
use columbina_model::resolve::audio_between;
use columbina_model::{
    apply, AudioFade, AudioKey, AudioParam, Bus, ChannelMap, CrossfadeLevel, Effect, Insert,
    KeyInterp, Op, OpError, Output, Project, Send, SourceRef, Strip, Time, Track, TrackFormat,
    TrackKind, Vca,
};
use serde_json::json;

/// The sample, with a second bus `n` that the track outputs to.
fn bus(n: u32) -> Bus {
    Bus {
        id: format!("B{n}"),
        name: format!("Bus {n}"),
        ..Bus::first()
    }
}

fn output(bus: &str) -> Output {
    Output {
        bus: bus.into(),
        on: true,
    }
}

/// Read a project after `change` is made to the sample as JSON, and give back why it is refused.
fn refused(change: impl FnOnce(&mut serde_json::Value)) -> OpError {
    let mut value = serde_json::to_value(Project::sample()).unwrap();
    change(&mut value);
    match Project::from_value(value) {
        Err(columbina_model::ReadError::Invalid(invalid)) => invalid.error,
        other => panic!("not refused as invalid: {other:?}"),
    }
}

fn reason(error: &OpError) -> String {
    serde_json::to_value(error).unwrap()["reason"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[test]
fn what_schema_4_adds_may_be_left_out_and_is_read_as_its_default_and_written_back_alike() {
    let sample = Project::sample();
    let written = serde_json::to_string_pretty(&sample).unwrap();
    let mut value: serde_json::Value = serde_json::from_str(&written).unwrap();
    let timeline = value["timelines"][0].as_object_mut().unwrap();
    for name in ["buses", "vcas", "track_groups", "audio"] {
        assert!(timeline.remove(name).is_some(), "{name} is written");
    }
    for track in timeline["tracks"].as_array_mut().unwrap() {
        let track = track.as_object_mut().unwrap();
        let sound = track["kind"] == "audio";
        for name in [
            "format",
            "channels",
            "order",
            "mute",
            "solo",
            "solo_safe",
            "adc",
            "automation_arm",
            "strip",
            "lanes",
        ] {
            assert_eq!(track.remove(name).is_some(), sound, "{name}");
        }
        for clip in track["clips"].as_array_mut().unwrap() {
            let clip = clip.as_object_mut().unwrap();
            for name in [
                "audio_fade_in",
                "audio_fade_out",
                "eq",
                "effects",
                "pitch_correction",
                "pitch",
                "channels",
            ] {
                assert!(clip.remove(name).is_some(), "{name} is written");
            }
        }
    }
    let read = Project::from_value(value).unwrap();
    assert_eq!(read, sample);
    // Written again byte for byte.
    assert_eq!(serde_json::to_string_pretty(&read).unwrap(), written);
}

#[test]
fn a_gain_written_as_a_number_is_read_as_one_value_and_written_so() {
    let mut value = serde_json::to_value(Project::sample()).unwrap();
    value["timelines"][0]["tracks"][1]["clips"][0]["gain_db"] = json!(-6);
    let project = Project::from_value(value).unwrap();
    let clip = &project.timelines[0].tracks[1].clips[0];
    assert_eq!(clip.gain_db, AudioParam::Fixed(-6.0));
    let written = serde_json::to_value(&project).unwrap();
    assert_eq!(
        written["timelines"][0]["tracks"][1]["clips"][0]["gain_db"],
        json!(-6.0)
    );
    assert!(written["timelines"][0]["tracks"][1]["clips"][0]["gain_db"].is_number());
}

#[test]
fn a_track_of_picture_has_no_field_of_sound_and_an_adaptive_track_names_its_channels() {
    let mut value = serde_json::to_value(Project::sample()).unwrap();
    value["timelines"][0]["tracks"][0]["mute"] = true.into();
    let error = Project::from_value(value).unwrap_err().to_string();
    assert!(error.contains("a video track has no `mute`"), "{error}");
    let mut value = serde_json::to_value(Project::sample()).unwrap();
    value["timelines"][0]["tracks"][1]["format"] = "adaptive".into();
    value["timelines"][0]["tracks"][1]
        .as_object_mut()
        .unwrap()
        .remove("channels");
    let error = Project::from_value(value).unwrap_err().to_string();
    assert!(error.contains("missing field `channels`"), "{error}");
}

#[test]
#[allow(clippy::too_many_lines, reason = "one refusal after another")]
fn routing_and_strips_that_cannot_be_played_are_refused_with_their_reasons() {
    // Seven buses, each feeding the next, are 7 deep; six are not.
    let chain = |count: u32| {
        move |value: &mut serde_json::Value| {
            let buses: Vec<Bus> = (1..=count)
                .map(|n| {
                    let mut bus = bus(n);
                    if n < count {
                        bus.strip.outputs = vec![output(&format!("B{}", n + 1))];
                    }
                    bus
                })
                .collect();
            value["timelines"][0]["buses"] = serde_json::to_value(buses).unwrap();
        }
    };
    let mut value = serde_json::to_value(Project::sample()).unwrap();
    chain(6)(&mut value);
    Project::from_value(value).unwrap();
    assert_eq!(reason(&refused(chain(7))), "bus_depth");
    // A to B to A.
    let error = refused(|value| {
        let (mut a, mut b) = (bus(1), bus(2));
        a.strip.outputs = vec![output("B2")];
        b.strip.sends = vec![Send {
            bus: "B1".into(),
            on: true,
            pre: false,
            level_db: 0.0,
            pan: columbina_model::Pan::default(),
        }];
        value["timelines"][0]["buses"] = serde_json::to_value([a, b]).unwrap();
    });
    assert_eq!(reason(&error), "bus_cycle");
    // A track that ducks under the bus it outputs to goes round in a loop too.
    let error = refused(|value| {
        let mut strip = Strip::to_first_bus();
        strip.track_fx = Some(Ducker {
            sources: vec![SourceRef::Bus("B1".into())],
            ..Ducker::default()
        });
        value["timelines"][0]["tracks"][1]["strip"] = serde_json::to_value(strip).unwrap();
    });
    assert_eq!(reason(&error), "bus_cycle");
    // Seven inserts.
    let error = refused(|value| {
        let mut strip = Strip::to_first_bus();
        strip.inserts = vec![
            Insert {
                on: true,
                effect: Effect::Gain(GainFx { gain_db: 0.0 }),
            };
            7
        ];
        value["timelines"][0]["tracks"][1]["strip"] = serde_json::to_value(strip).unwrap();
    });
    assert_eq!(reason(&error), "too_many_inserts");
    // An adaptive track of 25 channels, and a clip that names 3 channels on a stereo track.
    let error = refused(|value| {
        value["timelines"][0]["tracks"][1]["format"] = "adaptive".into();
        value["timelines"][0]["tracks"][1]["channels"] = 25.into();
    });
    assert_eq!(reason(&error), "channel_count");
    let error = refused(|value| {
        let map = ChannelMap {
            source: columbina_model::ChannelSources::default(),
            enabled: true,
            mute: false,
            level_db: 0.0,
        };
        value["timelines"][0]["tracks"][1]["clips"][0]["channels"] =
            serde_json::to_value(vec![map; 3]).unwrap();
    });
    assert_eq!(reason(&error), "channel_count");
    // An output to a bus that is not there, a VCA that is not there, and an order of two eqs.
    let error = refused(|value| {
        value["timelines"][0]["tracks"][1]["strip"]["outputs"] = json!([{ "bus": "B9" }]);
    });
    assert_eq!(reason(&error), "unknown_bus");
    let error = refused(|value| {
        value["timelines"][0]["tracks"][1]["strip"]["vcas"] = json!(["VCA1"]);
    });
    assert_eq!(reason(&error), "unknown_vca");
    let error = refused(|value| {
        value["timelines"][0]["tracks"][1]["strip"]["order"] = json!(["eq", "eq", "fx"]);
    });
    assert_eq!(reason(&error), "order");
    // VCAs that are members of each other.
    let error = refused(|value| {
        let vca = |id: &str, of: &str| Vca {
            id: id.into(),
            name: id.into(),
            fader_db: 0.0,
            mute: false,
            solo: false,
            vcas: vec![of.into()],
            lanes: columbina_model::Lanes::default(),
        };
        value["timelines"][0]["vcas"] =
            serde_json::to_value([vca("VCA1", "VCA2"), vca("VCA2", "VCA1")]).unwrap();
    });
    assert_eq!(reason(&error), "vca_cycle");
    // A fader above +10 dB is out of its range.
    let error = refused(|value| {
        value["timelines"][0]["tracks"][1]["strip"]["fader_db"] = 10.5.into();
    });
    assert_eq!(error, OpError::OutOfRange { what: "fader_db" });
}

#[test]
fn a_track_that_a_ducker_names_is_not_taken_away_and_one_that_ducks_is_added_back_whole() {
    let mut project = Project::sample();
    let mut ducked = Track::new("", TrackKind::Audio);
    ducked.audio.format = TrackFormat::Mono;
    ducked.audio.channels = 1;
    ducked.audio.strip.track_fx = Some(Ducker {
        on: true,
        sources: vec![SourceRef::Track("A1".into())],
        ..Ducker::default()
    });
    let inverse = apply(
        &mut project,
        Op::AddTrack {
            timeline: "t1".into(),
            track: ducked,
            index: None,
        },
    )
    .unwrap();
    let error = apply(
        &mut project,
        Op::RemoveTrack {
            timeline: "t1".into(),
            id: "A1".into(),
        },
    )
    .unwrap_err();
    assert_eq!(
        error,
        OpError::Missing {
            what: "track",
            id: "A1".into()
        }
    );
    let added = project.clone();
    let back = apply(&mut project, inverse).unwrap();
    apply(&mut project, back).unwrap();
    assert_eq!(project, added);
    // A track that outputs to a bus that is not there is refused.
    let mut lost = Track::new("", TrackKind::Audio);
    lost.audio.strip.outputs = vec![output("B4")];
    let error = apply(
        &mut project,
        Op::AddTrack {
            timeline: "t1".into(),
            track: lost,
            index: None,
        },
    )
    .unwrap_err();
    assert_eq!(error, OpError::UnknownBus { id: "B4".into() });
}

/// The gain of the first piece of sound of A1 of the sample at a sample.
fn heard_at(project: &Project, sample: i64) -> Option<f32> {
    let timeline = &project.timelines[0];
    audio_between(timeline, project, 0, 480_000)
        .first()
        .map(|piece| piece.gain.at(sample))
}

#[test]
fn a_muted_track_and_one_not_soloed_while_another_is_are_not_heard() {
    let mut project = Project::sample();
    assert_eq!(heard_at(&project, 1000), Some(1.0));
    project.timelines[0].tracks[1].audio.mute = true;
    assert_eq!(heard_at(&project, 1000), None);
    project.timelines[0].tracks[1].audio.mute = false;
    let mut other = Track::new("A2", TrackKind::Audio);
    other.audio.solo = true;
    project.timelines[0].tracks.push(other);
    project.validate().unwrap();
    assert_eq!(heard_at(&project, 1000), None);
    project.timelines[0].tracks[1].audio.solo_safe = true;
    assert_eq!(heard_at(&project, 1000), Some(1.0));
}

#[test]
fn a_gain_of_keyframes_is_heard_at_the_time_of_the_source_that_each_sample_reads() {
    let mut project = Project::sample();
    let key = |seconds: i64, value| AudioKey {
        at: Time {
            ticks: seconds * 48000,
            scale: 48000,
        },
        value,
        interp: KeyInterp::Linear,
    };
    let clip = &mut project.timelines[0].tracks[1].clips[0];
    clip.gain_db = AudioParam::Keys(vec![key(1, -20.0), key(3, 0.0)]);
    project.validate().unwrap();
    let near = |a: f32, b: f64| (f64::from(a) - b).abs() < 1e-6;
    assert!(near(heard_at(&project, 0).unwrap(), 0.1));
    assert!(near(heard_at(&project, 96_000).unwrap(), 10f64.powf(-0.5)));
    assert!(near(heard_at(&project, 200_000).unwrap(), 1.0));
    // Played twice as fast, second 2 of the source is heard at second 1 of the timeline.
    let clip = &mut project.timelines[0].tracks[1].clips[0];
    clip.speed = columbina_model::Speed { num: 2, den: 1 };
    clip.length = 50;
    project.validate().unwrap();
    assert!(near(heard_at(&project, 48_000).unwrap(), 10f64.powf(-0.5)));
    // Pan of keys moves across.
    let clip = &mut project.timelines[0].tracks[1].clips[0];
    clip.pan = AudioParam::Keys(vec![key(0, -1.0), key(4, 1.0)]);
    project.validate().unwrap();
    let pieces = audio_between(&project.timelines[0], &project, 0, 96_000);
    assert!((pieces[0].pan_at(48_000) - 0.0).abs() < 1e-6);
    assert!((pieces[0].pan - -1.0).abs() < 1e-6);
}

#[test]
fn a_fade_of_sound_lasts_its_time_along_its_curve() {
    let mut project = Project::sample();
    let clip = &mut project.timelines[0].tracks[1].clips[0];
    // Half a second (24000 samples), on the curve of the sine.
    clip.audio_fade_in = Some(AudioFade {
        length: Time { ticks: 1, scale: 2 },
        curve: 1.0,
        skew: 0.0,
    });
    project.validate().unwrap();
    let near = |a: f32, b: f64| (f64::from(a) - b).abs() < 1e-6;
    assert!(near(heard_at(&project, 0).unwrap(), 0.0));
    assert!(near(
        heard_at(&project, 12_000).unwrap(),
        std::f64::consts::FRAC_PI_4.sin()
    ));
    assert!(near(heard_at(&project, 24_000).unwrap(), 1.0));
    // A fade longer than the clip is refused.
    let clip = &mut project.timelines[0].tracks[1].clips[0];
    clip.audio_fade_in.as_mut().unwrap().length = Time {
        ticks: 11,
        scale: 1,
    };
    let error = project.validate().unwrap_err();
    assert_eq!(error.error, OpError::OutOfRange { what: "fade" });
}

#[test]
fn a_new_crossfade_is_on_straight_lines_and_one_of_3_db_at_equal_power() {
    let mut project = Project::sample();
    let timeline = &mut project.timelines[0];
    let a1 = &mut timeline.tracks[1];
    a1.clips[0].length = 50;
    let mut right = columbina_model::Clip::of_media(
        "c9",
        "m1",
        Time {
            ticks: 4 * 15360,
            scale: 15360,
        },
        50,
        50,
    );
    right.link = None;
    a1.clips.push(right);
    a1.transitions = vec![columbina_model::Transition {
        id: "x9".into(),
        kind: columbina_model::TransitionKind::Crossfade,
        left: "c2".into(),
        right: "c9".into(),
        length: 10,
        align: columbina_model::Alignment::Center,
        fade_in_level_db: CrossfadeLevel::default(),
        fade_out_level_db: CrossfadeLevel::default(),
    }];
    project.next_id = 10;
    project.validate().unwrap();
    // The crossfade lies over frames [45, 55), samples [86400, 105600); its middle is 96000.
    let at_middle = |project: &Project| {
        let pieces = audio_between(&project.timelines[0], project, 0, 192_000);
        (pieces[0].gain.at(96_000), pieces[1].gain.at(96_000))
    };
    assert_eq!(at_middle(&project), (0.5, 0.5));
    let x = &mut project.timelines[0].tracks[1].transitions[0];
    (x.fade_in_level_db, x.fade_out_level_db) = (CrossfadeLevel::Plus3, CrossfadeLevel::Minus3);
    let (out, into) = at_middle(&project);
    let half = std::f32::consts::FRAC_1_SQRT_2;
    assert!((into - half).abs() < 1e-6, "{into}");
    assert!((out - 0.5f32.powf(1.5)).abs() < 1e-6, "{out}");
    // A crossfade's level is written as its decibels.
    let written = serde_json::to_value(&project.timelines[0].tracks[1].transitions[0]).unwrap();
    assert_eq!(
        (&written["fade_in_level_db"], &written["fade_out_level_db"]),
        (&json!(3), &json!(-3))
    );
}
