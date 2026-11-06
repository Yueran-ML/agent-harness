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
                id: String::new(),
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
            remix: None,
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
            automation_arm: columbina_model::AutomationArm::Unarmed,
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
        OpError::InUse {
            what: "track",
            id: "A1".into(),
            by: "A2".into()
        }
    );
    let added = project.clone();
    let back = apply(&mut project, inverse).unwrap();
    apply(&mut project, back).unwrap();
    assert_eq!(project, added);
    // A member of a group of tracks stays while the group names it.
    let mut grouped = Project::sample();
    grouped.timelines[0].track_groups = vec![columbina_model::TrackGroup {
        id: "TG1".into(),
        name: "Dialogue".into(),
        color: columbina_model::Rgba([0, 0, 0, 255]),
        members: vec!["A1".into()],
        all_tracks: false,
        linked: vec![],
        active: true,
    }];
    grouped.validate().unwrap();
    let remove = Op::RemoveTrack {
        timeline: "t1".into(),
        id: "A1".into(),
    };
    assert_eq!(
        apply(&mut grouped, remove).unwrap_err(),
        OpError::InUse {
            what: "track",
            id: "A1".into(),
            by: "TG1".into()
        }
    );
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
fn a_track_is_heard_through_buses_that_are_not_muted_and_as_buses_are_soloed() {
    // A1 outputs to B2, which outputs to B1, the main output; A2 straight to B1.
    let mut project = Project::sample();
    let t = &mut project.timelines[0];
    let mut b2 = bus(2);
    b2.strip.outputs = vec![output("B1")];
    t.buses.push(b2);
    t.tracks[1].audio.strip.outputs = vec![output("B2")];
    let mut a2 = Track::new("A2", TrackKind::Audio);
    a2.clips = vec![columbina_model::Clip::of_media(
        "c9",
        "m1",
        Time {
            ticks: 0,
            scale: 15360,
        },
        0,
        100,
    )];
    t.tracks.push(a2);
    project.validate().unwrap();
    let heard = |project: &Project| -> Vec<String> {
        audio_between(&project.timelines[0], project, 0, 1000)
            .into_iter()
            .map(|piece| piece.track)
            .collect()
    };
    assert_eq!(heard(&project), ["A1", "A2"]);
    // B2 muted: A1 is not heard, but still drives a sidechain.
    project.timelines[0].buses[1].mute = true;
    assert_eq!(heard(&project), ["A2"]);
    let sources: Vec<String> =
        columbina_model::resolve::sources_between(&project.timelines[0], &project, 0, 1000)
            .into_iter()
            .map(|piece| piece.track)
            .collect();
    assert_eq!(sources, ["A1", "A2"]);
    project.timelines[0].buses[1].mute = false;
    // B2 soloed: what feeds it is heard, and A2 is not, unless it is solo-safe.
    project.timelines[0].buses[1].solo = true;
    assert_eq!(heard(&project), ["A1"]);
    project.timelines[0].tracks[2].audio.solo_safe = true;
    assert_eq!(heard(&project), ["A1", "A2"]);
    project.timelines[0].buses[1].solo = false;
    // A track that is off does not solo: the others are heard.
    let a2 = &mut project.timelines[0].tracks[2];
    (a2.enabled, a2.audio.solo, a2.audio.solo_safe) = (false, true, false);
    assert_eq!(heard(&project), ["A1"]);
    // A track that outputs to no bus reaches no main output.
    project.timelines[0].tracks[2].enabled = true;
    project.timelines[0].tracks[2].audio.solo = false;
    project.timelines[0].tracks[2].audio.strip.outputs = vec![];
    assert_eq!(heard(&project), ["A1"]);
    // A bus is written with its mute and solo, which may be left out.
    let written = serde_json::to_value(&project.timelines[0].buses[1]).unwrap();
    assert_eq!(
        (&written["mute"], &written["solo"], &written["solo_safe"]),
        (&json!(false), &json!(false), &json!(false))
    );
}

#[test]
fn a_track_added_without_a_strip_goes_to_bus_1_if_there_is_one_and_else_to_none() {
    use columbina_model::edit::{expand, Edit};
    let added = json!({ "op": "add_track", "timeline": "t1", "track": {
        "id": "", "kind": "audio", "name": "x", "enabled": true, "locked": false,
        "clips": [], "transitions": [],
    } });
    let outputs = |project: &Project| -> Vec<String> {
        let track = project.timelines[0].tracks.last().unwrap();
        track
            .audio
            .strip
            .outputs
            .iter()
            .map(|o| o.bus.clone())
            .collect()
    };
    let mut project = Project::sample();
    let inverse = apply(&mut project, serde_json::from_value(added.clone()).unwrap()).unwrap();
    assert_eq!(outputs(&project), ["B1"]);
    apply(&mut project, inverse).unwrap();
    // Only B2: the track outputs to none, and an explicit B1 is still refused.
    let t = &mut project.timelines[0];
    t.buses = vec![bus(2)];
    t.audio.monitor = Some("B2".into());
    t.tracks[1].audio.strip.outputs = vec![output("B2")];
    project.validate().unwrap();
    let inverse = apply(&mut project, serde_json::from_value(added).unwrap()).unwrap();
    assert_eq!(outputs(&project), Vec::<String>::new());
    // Taken back and done again, it is added as it was: to no bus.
    let added = project.clone();
    let back = apply(&mut project, inverse).unwrap();
    apply(&mut project, back).unwrap();
    assert_eq!(project, added);
    apply(
        &mut project,
        Op::RemoveTrack {
            timeline: "t1".into(),
            id: "A2".into(),
        },
    )
    .unwrap();
    let error = apply(
        &mut project,
        Op::AddTrack {
            timeline: "t1".into(),
            track: Track::new("", TrackKind::Audio),
            index: None,
        },
    )
    .unwrap_err();
    assert_eq!(error, OpError::UnknownBus { id: "B1".into() });
    // The edit that adds a track lets the engine choose.
    expand(
        &mut project,
        Edit::AddTrack {
            timeline: "t1".into(),
            kind: TrackKind::Audio,
            name: None,
            language: None,
            index: None,
        },
    )
    .unwrap();
    assert_eq!(outputs(&project), Vec::<String>::new());
    // Read from a project that leaves the strip out, the track goes to B1.
    let mut value = serde_json::to_value(Project::sample()).unwrap();
    value["timelines"][0]["tracks"][1]
        .as_object_mut()
        .unwrap()
        .remove("strip");
    let read = Project::from_value(value).unwrap();
    assert_eq!(outputs(&read), ["B1"]);
}

#[test]
#[allow(clippy::too_many_lines, reason = "a path of each kind of setting")]
fn lanes_are_of_the_settings_of_their_strip_by_their_paths_and_kinds() {
    use columbina_model::{Lane, LaneKind, LanePoint};
    let lane = |kind| Lane {
        points: vec![LanePoint {
            at: Time {
                ticks: 0,
                scale: 48000,
            },
            value: 0.0,
        }],
        kind,
        scale: None,
    };
    let mut project = Project::sample();
    let t = &mut project.timelines[0];
    t.buses.push(bus(3));
    let strip = &mut t.tracks[1].audio.strip;
    strip.sends = vec![Send {
        bus: "B3".into(),
        on: true,
        pre: false,
        level_db: 0.0,
        pan: columbina_model::Pan::default(),
    }];
    strip.inserts = vec![
        Insert {
            id: String::new(),
            on: true,
            effect: Effect::Gain(GainFx { gain_db: 0.0 }),
        };
        2
    ];
    for effect in [
        Effect::Limiter(columbina_model::audio::fx::LimiterFx::default()),
        Effect::DeEsser(columbina_model::audio::fx::DeEsserFx::default()),
    ] {
        strip.inserts.push(Insert {
            id: String::new(),
            on: true,
            effect,
        });
    }
    project.validate().unwrap();
    // The inserts are given ids, which their lanes name.
    let ids: Vec<&str> = project.timelines[0].tracks[1]
        .audio
        .strip
        .inserts
        .iter()
        .map(|i| i.id.as_str())
        .collect();
    assert_eq!(ids, ["fx1", "fx2", "fx3", "fx4"]);
    let continuous = [
        "fader_db",
        "trim_db",
        "pan.x",
        "eq.gain_db",
        "eq.2.gain_db",
        "eq.0.freq",
        "dynamics.compressor.threshold_db",
        "dynamics.makeup_db",
        "send.B3.level_db",
        "send.B3.pan.x",
        "insert.fx2.gain_db",
        "insert.fx3.ceiling_db",
    ];
    let switches = [
        "mute",
        "eq.on",
        "eq.5.on",
        "dynamics.compressor.on",
        "send.B3.on",
        "output.B1.on",
        "insert.fx1.on",
        "insert.fx3.sidechain.on",
    ];
    // Each lane at the value that its setting has (T14 holds a lane to the range of its
    // setting: a frequency of 0 Hz is refused), or 0 where there is no setting.
    let fits = |path: &str, kind| {
        use columbina_model::audio::automation::{fixed_value, view, Owner};
        let mut project = project.clone();
        let audio = &project.timelines[0].tracks[1].audio;
        let set = fixed_value(
            &view(Owner::Strip {
                strip: &audio.strip,
                mute: audio.mute,
                bus: false,
            }),
            path,
        );
        let mut lane = lane(kind);
        lane.points[0].value = set.unwrap_or(0.0);
        project.timelines[0].tracks[1]
            .audio
            .lanes
            .insert(path.into(), lane);
        project.validate().is_ok()
    };
    for path in continuous {
        assert!(fits(path, LaneKind::Continuous), "{path}");
        assert!(!fits(path, LaneKind::Switch), "{path} is not a switch");
    }
    for path in switches {
        assert!(fits(path, LaneKind::Switch), "{path}");
    }
    for path in [
        "fader",
        "send.B2.level_db",
        "insert.fx5.on",
        "eq.6.gain_db",
        "dynamics.expander.mode",
        // Listening is never written, and a range is not a number.
        "insert.fx3.sidechain.listen",
        "insert.fx4.listen",
        "dynamics.compressor.sidechain.listen",
        "insert.fx4.range",
        "eq.2.kind",
        "track_fx.amount_db",
        "",
        // What the graph does not move as a lane would, and a whole number (T14's review).
        "inserts_on",
        "pan.spread",
        "send.B3.pre",
        "eq.1.slope",
    ] {
        assert!(!fits(path, LaneKind::Continuous), "{path} is of nothing");
        assert!(!fits(path, LaneKind::Switch), "{path} is of nothing");
    }
    // A VCA has its fader and its mute.
    let mut vca = project.clone();
    vca.timelines[0].vcas = vec![columbina_model::Vca {
        id: "VCA1".into(),
        name: "Music".into(),
        fader_db: 0.0,
        mute: false,
        solo: false,
        vcas: vec![],
        automation_arm: columbina_model::AutomationArm::Unarmed,
        lanes: [("fader_db".to_owned(), lane(LaneKind::Continuous))].into(),
    }];
    vca.validate().unwrap();
    vca.timelines[0].vcas[0]
        .lanes
        .insert("pan.x".into(), lane(LaneKind::Continuous));
    assert!(vca.validate().is_err());
}

#[test]
fn vcas_named_twice_or_with_their_masters_a_bus_named_as_a_track_and_levels_of_a_dissolve_are_refused(
) {
    let vca = |id: &str, of: &[&str]| Vca {
        id: id.into(),
        name: id.into(),
        fader_db: 0.0,
        mute: false,
        solo: false,
        vcas: of.iter().map(|&v| v.into()).collect(),
        automation_arm: columbina_model::AutomationArm::Unarmed,
        lanes: columbina_model::Lanes::default(),
    };
    let with = |vcas: Vec<Vca>, strip: &'static [&'static str]| {
        move |value: &mut serde_json::Value| {
            value["timelines"][0]["vcas"] = serde_json::to_value(&vcas).unwrap();
            value["timelines"][0]["tracks"][1]["strip"]["vcas"] = json!(strip);
        }
    };
    let range = OpError::OutOfRange { what: "vcas" };
    // VCA2 is a member of VCA1: a strip of both would add VCA1 twice.
    let nested = || vec![vca("VCA1", &[]), vca("VCA2", &["VCA1"])];
    let mut value = serde_json::to_value(Project::sample()).unwrap();
    with(nested(), &["VCA2"])(&mut value);
    Project::from_value(value).unwrap();
    assert_eq!(refused(with(nested(), &["VCA2", "VCA1"])), range);
    assert_eq!(refused(with(nested(), &["VCA1", "VCA1"])), range);
    let twice = vec![vca("VCA1", &[]), vca("VCA2", &["VCA1", "VCA1"])];
    assert_eq!(refused(with(twice, &[])), range);
    let deep = vec![
        vca("VCA1", &[]),
        vca("VCA2", &["VCA1"]),
        vca("VCA3", &["VCA2", "VCA1"]),
    ];
    assert_eq!(refused(with(deep, &[])), range);
    // A bus named as a track.
    let error = refused(|value| {
        let mut a1 = bus(1);
        a1.id = "A1".into();
        value["timelines"][0]["buses"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::to_value(a1).unwrap());
    });
    assert_eq!(
        error,
        OpError::Taken {
            what: "track",
            id: "A1".into()
        }
    );
    // A dissolve has no levels of a crossfade.
    let error = refused(|value| {
        let first = value["timelines"][0]["tracks"][0]["clips"][0].clone();
        value["timelines"][0]["tracks"][0]["clips"]
            .as_array_mut()
            .unwrap()
            .push(first);
        let clips = &mut value["timelines"][0]["tracks"][0]["clips"];
        clips[0]["length"] = 50.into();
        clips[1]["id"] = "c9".into();
        clips[1]["start"] = 50.into();
        clips[1]["length"] = 50.into();
        clips[1]["source"]["in"]["ticks"] = (2 * 15360).into();
        value["timelines"][0]["tracks"][0]["transitions"] = json!([{
            "id": "x9", "kind": "dissolve", "left": "c1", "right": "c9", "length": 10,
            "align": "center", "fade_in_level_db": 3, "fade_out_level_db": 0,
        }]);
    });
    assert_eq!(error, OpError::OutOfRange { what: "level" });
    // A new pan sends nothing to the LFE.
    assert_eq!(columbina_model::Pan::default().lfe_db, -100.0);
}

#[test]
fn times_of_sound_further_than_2_to_the_53_ticks_are_refused_and_never_overflow() {
    let far = Time {
        ticks: 1 << 53,
        scale: u32::MAX,
    };
    // A fade, a key and the end of an active range.
    let error = refused(|value| {
        value["timelines"][0]["tracks"][1]["clips"][0]["audio_fade_in"] =
            json!({ "length": far, "curve": 0.0, "skew": 0.0 });
    });
    assert_eq!(error, OpError::OutOfRange { what: "fade" });
    let error = refused(|value| {
        value["timelines"][0]["tracks"][1]["clips"][0]["gain_db"] =
            json!({ "keys": [{ "at": far, "value": 0.0 }] });
    });
    assert_eq!(error, OpError::OutOfRange { what: "gain_db" });
    let error = refused(|value| {
        value["timelines"][0]["audio"]["record"]["active_range"] = json!({
            "start": { "ticks": 0, "scale": 1 }, "end": far, "on": true,
        });
    });
    assert_eq!(error, OpError::OutOfRange { what: "audio" });
    // The longest fades and clip there are, of the finest scales: compared without overflow.
    let mut project = Project::sample();
    let clip = &mut project.timelines[0].tracks[1].clips[0];
    let longest = Time {
        ticks: (1 << 53) - 1,
        scale: u32::MAX,
    };
    clip.audio_fade_in = Some(AudioFade {
        length: longest,
        curve: 0.0,
        skew: 0.0,
    });
    clip.audio_fade_out = Some(AudioFade {
        length: Time {
            ticks: (1 << 53) - 1,
            scale: u32::MAX - 1,
        },
        curve: 0.0,
        skew: 0.0,
    });
    assert!(project.validate().is_err());
}

#[test]
fn a_track_of_sound_is_muted_and_soloed_by_set_track_and_taken_back() {
    let mut project = Project::sample();
    let set = |id: &str, mute, solo, solo_safe| Op::SetTrack {
        timeline: "t1".into(),
        id: id.into(),
        name: None,
        color: None,
        enabled: None,
        locked: None,
        language: None,
        mute,
        solo,
        solo_safe,
    };
    // The track of a project of schema 3 that was off is muted; it is heard again unmuted.
    project.timelines[0].tracks[1].audio.mute = true;
    let before = project.clone();
    assert_eq!(heard_at(&project, 1000), None);
    let inverse = apply(&mut project, set("A1", Some(false), Some(true), None)).unwrap();
    assert_eq!(heard_at(&project, 1000), Some(1.0));
    assert!(project.timelines[0].tracks[1].audio.solo);
    assert_eq!(inverse, set("A1", Some(true), Some(false), None));
    apply(&mut project, inverse).unwrap();
    assert_eq!(project, before);
    let written = serde_json::to_value(set("A1", Some(true), None, Some(true))).unwrap();
    assert_eq!(
        written,
        json!({ "op": "set_track", "timeline": "t1", "id": "A1", "mute": true, "solo_safe": true })
    );
    // A track of picture has no mute.
    let error = apply(&mut project, set("V1", Some(true), None, None)).unwrap_err();
    assert_eq!(
        error,
        OpError::WrongTrack {
            what: "mute",
            kind: TrackKind::Video
        }
    );
}

/// A fade of sound of half a second.
fn half_second() -> AudioFade {
    AudioFade {
        length: Time { ticks: 1, scale: 2 },
        curve: 0.0,
        skew: 0.0,
    }
}

/// The audio fades of the clips of A1 of a project, by clip.
fn fades_of(project: &Project) -> Vec<(i64, i64, Option<Time>, Option<Time>)> {
    project.timelines[0].tracks[1]
        .clips
        .iter()
        .map(|c| {
            (
                c.start,
                c.length,
                c.audio_fade_in.map(|f| f.length),
                c.audio_fade_out.map(|f| f.length),
            )
        })
        .collect()
}

#[test]
fn a_blade_leaves_no_fade_at_the_cut_and_cuts_a_fade_too_long_for_its_half() {
    use columbina_model::edit::{expand, Edit};
    let blade = |project: &mut Project, at| {
        expand(
            project,
            Edit::Blade {
                timeline: "t1".into(),
                at,
                tracks: Some(vec!["A1".into()]),
                linked: false,
                grouped: false,
            },
        )
    };
    // c2 lasts 100 frames, 4 s: cut in the middle, each half keeps the fade at its far end.
    let mut project = Project::sample();
    let clip = &mut project.timelines[0].tracks[1].clips[0];
    (clip.audio_fade_in, clip.audio_fade_out) = (Some(half_second()), Some(half_second()));
    project.validate().unwrap();
    blade(&mut project, 50).unwrap();
    let half = Time { ticks: 1, scale: 2 };
    assert_eq!(
        fades_of(&project),
        [(0, 50, Some(half), None), (50, 50, None, Some(half))]
    );
    // Heard across the cut at full gain: no dip.
    let pieces = audio_between(&project.timelines[0], &project, 0, 192_000);
    assert_eq!(pieces[0].gain.at(95_999), 1.0);
    assert_eq!(pieces[1].gain.at(96_000), 1.0);
    // Cut 2 frames in, the first half is too short for the fade in: it is cut to the half.
    let mut project = Project::sample();
    let clip = &mut project.timelines[0].tracks[1].clips[0];
    (clip.audio_fade_in, clip.audio_fade_out) = (Some(half_second()), Some(half_second()));
    project.validate().unwrap();
    blade(&mut project, 2).unwrap();
    let fades = fades_of(&project);
    assert_eq!(fades[0].3, None);
    let first = fades[0].2.unwrap();
    assert_eq!(
        i128::from(first.ticks) * 25,
        i128::from(first.scale) * 2,
        "2 frames"
    );
    assert_eq!(fades[1], (2, 98, None, Some(half)));
}

#[test]
fn a_trim_shortens_a_fade_of_sound_that_the_clip_no_longer_holds() {
    use columbina_model::edit::{expand, Edit};
    use columbina_model::Edge;
    let mut project = Project::sample();
    let clip = &mut project.timelines[0].tracks[1].clips[0];
    // Fades of 1.5 s and 2 s on a clip of 4 s: trimmed to 3 s from its start, the fade in goes
    // first, to 1 s.
    clip.audio_fade_in = Some(AudioFade {
        length: Time { ticks: 3, scale: 2 },
        curve: 0.5,
        skew: 0.0,
    });
    clip.audio_fade_out = Some(AudioFade {
        length: Time { ticks: 2, scale: 1 },
        curve: 0.0,
        skew: 0.0,
    });
    project.validate().unwrap();
    expand(
        &mut project,
        Edit::Trim {
            timeline: "t1".into(),
            clip: "c2".into(),
            edge: Edge::In,
            delta: 25,
            ripple: false,
            linked: false,
        },
    )
    .unwrap();
    let clip = &project.timelines[0].tracks[1].clips[0];
    assert_eq!(clip.length, 75);
    let seconds = |t: Time| f64::from(i32::try_from(t.ticks).unwrap()) / f64::from(t.scale);
    assert!((seconds(clip.audio_fade_in.unwrap().length) - 1.0).abs() < 1e-9);
    assert_eq!(clip.audio_fade_in.unwrap().curve, 0.5);
    assert!((seconds(clip.audio_fade_out.unwrap().length) - 2.0).abs() < 1e-9);
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
