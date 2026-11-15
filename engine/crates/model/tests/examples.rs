//! Examples of every operation, as the engine applies them: the project before, the operation as
//! it was given, the operation as it was done and the one that takes it back, and the project
//! after. They are kept in `tests/data/ops-examples.json`, one to a line, for the copies of the
//! project that the clients keep to be checked against the engine.
//!
//! The file is written anew when `COLUMBINA_WRITE_EXAMPLES=1` is set; otherwise it is checked to
//! be as the engine applies the operations now.

use std::path::Path;

use columbina_model::fusion::{self, Composition, Input};
use columbina_model::grade::{Interp, Key, Kind, Link, Node, Param, Port, Pos, Target};
use columbina_model::{
    apply, apply_done, Alignment, Bin, Clip, Cue, Grade, Group, Op, Placed, Project, Proxy, Rate,
    Rgba, Source, Speed, TextAlign, Time, TitleStyle, Track, TrackKind, Transition, TransitionKind,
};

const FILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/ops-examples.json");

/// A time of m1, which counts ticks of 1/15360 of a second.
fn at(seconds: i64) -> Time {
    Time {
        ticks: seconds * 15360,
        scale: 15360,
    }
}

fn title(text: &str) -> Source {
    Source::Title {
        text: text.into(),
        style: TitleStyle {
            font: String::new(),
            size: 96.0.into(),
            color: Rgba([255, 255, 255, 255]),
            x: 0.5.into(),
            y: 0.5.into(),
            align: TextAlign::Center,
        },
    }
}

fn set_clip(id: &str) -> Op {
    Op::SetClip {
        timeline: "t1".into(),
        id: id.into(),
        enabled: None,
        speed: None,
        reverse: None,
        link: None,
        transform: None,
        crop: None,
        composite: None,
        retime_process: None,
        gain_db: None,
        pan: None,
        fade_in: None,
        fade_out: None,
        audio_fade_in: None,
        audio_fade_out: None,
        eq: None,
        effects: None,
        pitch_correction: None,
        pitch: None,
        channels: None,
        source: None,
    }
}

/// `set_clip` of c2 to a gain that rises from -12 dB to 0 dB, easing in, over a second of m1.
fn keyed_gain() -> Op {
    use columbina_model::{AudioKey, AudioParam, KeyInterp};
    let mut op = set_clip("c2");
    if let Op::SetClip { gain_db, .. } = &mut op {
        *gain_db = Some(AudioParam::Keys(vec![
            AudioKey {
                at: at(1),
                value: -12.0,
                interp: KeyInterp::Linear,
            },
            AudioKey {
                at: at(2),
                value: 0.0,
                interp: KeyInterp::EaseIn,
            },
        ]));
    }
    op
}

/// `add_track` of a 5.1 track of sound, muted and solo-safe, whose strip is at -6 dB with a Gain
/// in an insert and a Ducker that A1 drives, with a clip of m1 whose channels are named.
fn sound_track() -> Op {
    use columbina_model::audio::fx::{Ducker, GainFx};
    use columbina_model::{
        ChannelMap, ChannelSource, ChannelSources, Effect, Insert, SourceRef, TrackFormat,
    };
    let mut clip = Clip::of_media("", "m1", at(1), 0, 30);
    clip.channels = Some(
        (0..6)
            .map(|channel| ChannelMap {
                remix: None,
                // L and R from stream 1, C their mean, and the rest silent.
                source: ChannelSources(match channel {
                    0 | 1 => vec![ChannelSource { stream: 1, channel }],
                    2 => (0..2)
                        .map(|channel| ChannelSource { stream: 1, channel })
                        .collect(),
                    _ => vec![],
                }),
                enabled: true,
                mute: false,
                level_db: 0.0,
            })
            .collect(),
    );
    let mut track = Track::new("", TrackKind::Audio).with_clips(vec![clip]);
    track.audio.format = TrackFormat::FiveOne;
    track.audio.channels = 6;
    track.audio.mute = true;
    track.audio.solo_safe = true;
    track.audio.strip.fader_db = -6.0;
    track.audio.strip.inserts = vec![Insert {
        id: String::new(),
        on: true,
        effect: Effect::Gain(GainFx { gain_db: 3.0 }),
    }];
    track.audio.strip.track_fx = Some(Ducker {
        on: true,
        sources: vec![SourceRef::Track("A1".into())],
        amount_db: 12.0,
        ..Ducker::default()
    });
    Op::AddTrack {
        timeline: "t1".into(),
        track,
        index: None,
    }
}

/// The project that the examples start from: the sample, with a track of subtitles S1 holding
/// q3, bin b4, c5 on V1 right after c1 with dissolve x6 between them, c1 and c2 linked in g7, and
/// a title c8 on V2.
fn base() -> Project {
    let mut project = Project::sample();
    let mut subtitles = Track::new("", TrackKind::Subtitle);
    subtitles.language = "zh".into();
    subtitles.cues = vec![Cue {
        id: String::new(),
        start: 0,
        length: 50,
        text: "你好".into(),
    }];
    let mut link_c1 = set_clip("c1");
    if let Op::SetClip { link, .. } = &mut link_c1 {
        *link = Some(Some(String::new()));
    }
    let mut link_c2 = set_clip("c2");
    if let Op::SetClip { link, .. } = &mut link_c2 {
        *link = Some(Some("g7".into()));
    }
    let ops = vec![
        Op::AddTrack {
            timeline: "t1".into(),
            track: subtitles,
            index: None,
        },
        Op::AddBin {
            bin: Bin {
                id: String::new(),
                name: "Rushes".into(),
                parent: None,
            },
            index: None,
        },
        Op::InsertClip {
            timeline: "t1".into(),
            track: "V1".into(),
            clip: Clip::of_media("", "m1", at(4), 100, 50),
        },
        Op::AddTransition {
            timeline: "t1".into(),
            track: "V1".into(),
            transition: Transition {
                id: String::new(),
                kind: TransitionKind::Dissolve,
                left: "c1".into(),
                right: "c5".into(),
                length: 10,
                align: Alignment::Center,
                fade_in_level_db: columbina_model::CrossfadeLevel::Zero,
                fade_out_level_db: columbina_model::CrossfadeLevel::Zero,
            },
        },
        link_c1,
        link_c2,
        Op::AddTrack {
            timeline: "t1".into(),
            track: Track::new("", TrackKind::Video).with_clips(vec![Clip::new(
                "",
                title("Hello"),
                0,
                50,
            )]),
            index: Some(1),
        },
    ];
    for op in ops {
        apply(&mut project, op).unwrap();
    }
    project
}

/// The examples, each a name, the operations that make the project before it of the base, and
/// the operation.
#[allow(clippy::too_many_lines)]
fn examples() -> Vec<(&'static str, Vec<Op>, Op)> {
    let mut media = Project::sample().media[0].clone();
    media.id = String::new();
    media.name = "b-roll.mov".into();
    media.path = "C:/media/b-roll.mov".into();
    let mut still = media.clone();
    still.name = "picture.png".into();
    still.path = "C:/media/picture.png".into();
    still.facts.audio = None;
    let mut fast = set_clip("c2");
    if let Op::SetClip {
        enabled,
        speed,
        reverse,
        transform,
        gain_db,
        pan,
        fade_in,
        fade_out,
        ..
    } = &mut fast
    {
        *enabled = Some(false);
        *speed = Some(Speed { num: 3, den: 2 });
        *reverse = Some(true);
        *transform = Some(
            Placed {
                x: 0.25,
                y: -0.125,
                scale: 1.5,
                rotation: 90.0,
                opacity: 0.5,
            }
            .into(),
        );
        *gain_db = Some((-6.0).into());
        *pan = Some((-0.5).into());
        (*fade_in, *fade_out) = (Some(10), Some(5));
    }
    let mut unlinked = set_clip("c1");
    if let Op::SetClip { link, .. } = &mut unlinked {
        *link = Some(None);
    }
    let mut new_group = set_clip("c5");
    if let Op::SetClip { link, .. } = &mut new_group {
        *link = Some(Some(String::new()));
    }
    let mut retitled = set_clip("c8");
    if let Op::SetClip { source, .. } = &mut retitled {
        *source = Some(title("Goodbye"));
    }
    let mut colour = Clip::new(
        "",
        Source::Color {
            color: Rgba([0, 0, 0, 255]),
        },
        60,
        20,
    );
    colour.link = Some(String::new());
    let mut timeline = columbina_model::Timeline::new("", "Second", Rate { num: 30, den: 1 });
    timeline.tracks = vec![
        Track::new("", TrackKind::Audio).with_clips(vec![Clip::of_media("", "m1", at(1), 0, 30)]),
    ];
    // A timeline shows one track of subtitles at a time: S2 is added hidden while S1 is shown,
    // and S1 is hidden before S2 is shown.
    let mut english = Track::new("", TrackKind::Subtitle);
    english.name = "English".into();
    english.language = "en".into();
    english.enabled = false;
    let add_english = Op::AddTrack {
        timeline: "t1".into(),
        track: english,
        index: None,
    };
    let show = |id: &str, enabled: bool| Op::SetTrack {
        timeline: "t1".into(),
        id: id.into(),
        name: None,
        color: None,
        enabled: Some(enabled),
        locked: None,
        language: None,
        mute: None,
        solo: None,
        solo_safe: None,
    };
    let add_bin = |parent: Option<&str>| Op::AddBin {
        bin: Bin {
            id: String::new(),
            name: "Selects".into(),
            parent: parent.map(Into::into),
        },
        index: None,
    };
    // Speed points of c2, the sound on A1: 40 frames forward, a picture held for 20, and 60
    // frames backward at half the speed.
    let held = || {
        written_op(serde_json::json!({
            "op": "set_retime", "timeline": "t1", "id": "c2",
            "retime": { "points": [
                { "frame": 0, "speed": { "num": 1, "den": 1 } },
                { "frame": 40, "speed": { "num": 0, "den": 1 } },
                { "frame": 60, "speed": { "num": -1, "den": 2 } },
            ] },
            "length": 120,
        }))
    };
    let mut all = vec![
        (
            "add_media",
            vec![],
            Op::AddMedia {
                media: media.clone(),
                index: None,
            },
        ),
        (
            "remove_media",
            vec![Op::AddMedia {
                media,
                index: Some(0),
            }],
            Op::RemoveMedia { id: "m9".into() },
        ),
        (
            "add_timeline",
            vec![],
            Op::AddTimeline {
                timeline,
                index: Some(0),
            },
        ),
        (
            "remove_timeline",
            vec![],
            Op::RemoveTimeline { id: "t1".into() },
        ),
        (
            "add_track",
            vec![],
            Op::AddTrack {
                timeline: "t1".into(),
                track: Track::new("", TrackKind::Audio).with_clips(vec![Clip::of_media(
                    "",
                    "m1",
                    at(2),
                    10,
                    20,
                )]),
                index: None,
            },
        ),
        (
            "add_track of subtitles, hidden while S1 is shown",
            vec![],
            add_english.clone(),
        ),
        (
            "move_track",
            vec![],
            Op::MoveTrack {
                timeline: "t1".into(),
                id: "A1".into(),
                index: 0,
            },
        ),
        (
            "remove_track",
            vec![],
            Op::RemoveTrack {
                timeline: "t1".into(),
                id: "V1".into(),
            },
        ),
        (
            "insert_clip of media",
            vec![],
            Op::InsertClip {
                timeline: "t1".into(),
                track: "A1".into(),
                clip: Clip::of_media("", "m1", at(5), 120, 25),
            },
        ),
        (
            "insert_clip of a title",
            vec![],
            Op::InsertClip {
                timeline: "t1".into(),
                track: "V2".into(),
                clip: Clip::new("", title("Chapter 2"), 100, 40),
            },
        ),
        (
            "insert_clip of a colour in a new group",
            vec![],
            Op::InsertClip {
                timeline: "t1".into(),
                track: "V2".into(),
                clip: colour,
            },
        ),
        (
            "remove_clip",
            vec![],
            Op::RemoveClip {
                timeline: "t1".into(),
                id: "c8".into(),
            },
        ),
        (
            "move_clip",
            vec![],
            Op::MoveClip {
                timeline: "t1".into(),
                id: "c8".into(),
                track: "V2".into(),
                start: 200,
            },
        ),
        (
            "shift_clips with an internal transition",
            vec![],
            Op::ShiftClips {
                timeline: "t1".into(),
                track: "V1".into(),
                clips: vec!["c1".into(), "c5".into()],
                by: 20,
            },
        ),
        (
            "trim_clip of media",
            vec![],
            Op::TrimClip {
                timeline: "t1".into(),
                id: "c2".into(),
                source_in: Some(Time {
                    ticks: 614,
                    scale: 15360,
                }),
                start: 1,
                length: 90,
            },
        ),
        (
            "trim_clip of a title",
            vec![],
            Op::TrimClip {
                timeline: "t1".into(),
                id: "c8".into(),
                source_in: None,
                start: 10,
                length: 30,
            },
        ),
        (
            "set_proxy",
            vec![],
            Op::SetProxy {
                media: "m1".into(),
                proxy: Some(Proxy {
                    path: "C:/work/.columbina/studio/cache/proxies/m1-0123456789ab.mov".into(),
                    width: 1920,
                    height: 1080,
                    codec: "hevc".into(),
                    digest_of_source: "0".repeat(64),
                }),
            },
        ),
        (
            "set_timeline",
            vec![],
            Op::SetTimeline {
                id: "t1".into(),
                name: Some("Final".into()),
                rate: Some(Rate { num: 50, den: 1 }),
                width: Some(3840),
                height: Some(2160),
                sample_rate: Some(96000),
            },
        ),
        (
            "set_track",
            vec![],
            Op::SetTrack {
                timeline: "t1".into(),
                id: "S1".into(),
                name: Some("Chinese".into()),
                color: Some(Some(Rgba([90, 160, 224, 255]))),
                enabled: Some(false),
                locked: Some(true),
                language: Some("zh-Hans".into()),
                mute: None,
                solo: None,
                solo_safe: None,
            },
        ),
        (
            "set_track shows S2 once S1 is hidden",
            vec![add_english, show("S1", false)],
            show("S2", true),
        ),
        ("set_clip", vec![], fast),
        ("set_clip out of its group", vec![], unlinked),
        ("set_clip in a new group", vec![], new_group),
        ("set_clip of a title", vec![], retitled),
        ("set_clip of a gain of keyframes", vec![], keyed_gain()),
        (
            "add_track of sound, 5.1, muted, with a strip",
            vec![],
            sound_track(),
        ),
        ("add_bin", vec![], add_bin(Some("b4"))),
        (
            "remove_bin",
            vec![add_bin(None)],
            Op::RemoveBin { id: "b9".into() },
        ),
        (
            "set_bin",
            vec![add_bin(None)],
            Op::SetBin {
                id: "b9".into(),
                name: Some("Day 1".into()),
                parent: Some(Some("b4".into())),
            },
        ),
        (
            "set_media",
            vec![],
            Op::SetMedia {
                still: None,
                id: "m1".into(),
                name: Some("Talk".into()),
                bin: Some(Some("b4".into())),
                mark_in: Some(Some(at(1))),
                mark_out: Some(Some(at(8))),
            },
        ),
        (
            "set_color_settings to Resolve Color Management, Custom",
            vec![],
            Op::SetColorSettings {
                settings: serde_json::json!({
                    "science": "yrgb_managed",
                    "preset": "custom",
                    "timeline": "DaVinci Wide Gamut/DaVinci Intermediate",
                    "working_luminance": "custom",
                    "custom_luminance": 600,
                    "output_drt": "davinci",
                }),
            },
        ),
        (
            "set_color_settings of a LUT of the project",
            vec![],
            Op::SetColorSettings {
                settings: serde_json::json!({
                    "lut_interpolation": "tetrahedral",
                    "output_lut": { "3d": "Film/Kodak 2383.cube" },
                }),
            },
        ),
        (
            "set_color_settings to ACEScct",
            vec![],
            Op::SetColorSettings {
                settings: serde_json::json!({
                    "science": "acescct",
                    "aces_version": "1.3",
                    "idt": "Apple Log",
                    "odt": "P3-D65 ST2084 (1000 nits)",
                }),
            },
        ),
        (
            "set_media_audio",
            vec![],
            serde_json::from_value(serde_json::json!({
                "op": "set_media_audio", "id": "m1", "config": [
                    {"name":"Left", "format":"mono", "channels":[{"source":{"stream":1,"channel":0}}]},
                    {"name":"Right", "format":"mono", "channels":[{"source":{"stream":1,"channel":1},"mute":true,"level_db":-3}]}
                ]
            })).unwrap(),
        ),
        (
            "set_media_color",
            vec![],
            Op::SetMediaColor {
                id: "m1".into(),
                input: Some(Some("DJI D-Gamut/D-Log".into())),
                levels: Some(columbina_model::Levels::Full),
                lut: None,
                inverse_drt: None,
            },
        ),
        (
            "set_media_color back to the project's default",
            vec![Op::SetMediaColor {
                id: "m1".into(),
                input: Some(Some("bypass".into())),
                levels: None,
                lut: None,
                inverse_drt: None,
            }],
            Op::SetMediaColor {
                id: "m1".into(),
                input: Some(None),
                levels: None,
                lut: None,
                inverse_drt: None,
            },
        ),
        (
            "set_media_color of a source clip's LUTs and Inverse DRT",
            vec![],
            Op::SetMediaColor {
                id: "m1".into(),
                input: None,
                levels: None,
                lut: Some(columbina_model::color_management::LutPair {
                    one: Some("Shapers/log.cube".into()),
                    three: Some("Film/Look.cube".into()),
                }),
                inverse_drt: Some(false),
            },
        ),
        (
            "set_media_streams, as the engine reads them of media of an older project",
            vec![],
            Op::SetMediaStreams {
                id: "m1".into(),
                layout: Some("5.1(side)".into()),
                streams: Some(vec![
                    columbina_model::AudioStream {
                        index: 1,
                        channels: 6,
                        layout: Some("5.1(side)".into()),
                    },
                    columbina_model::AudioStream {
                        index: 2,
                        channels: 1,
                        layout: None,
                    },
                ]),
            },
        ),
        (
            "set_media still classification",
            vec![Op::AddMedia {
                media: still,
                index: None,
            }],
            Op::SetMedia {
                still: Some(true),
                id: "m9".into(),
                name: None,
                bin: None,
                mark_in: None,
                mark_out: None,
            },
        ),
        (
            "add_transition",
            vec![Op::InsertClip {
                timeline: "t1".into(),
                track: "V2".into(),
                clip: Clip::new("", title("Next"), 50, 30),
            }],
            Op::AddTransition {
                timeline: "t1".into(),
                track: "V2".into(),
                transition: Transition {
                    id: String::new(),
                    kind: TransitionKind::DipBlack,
                    left: "c8".into(),
                    right: "c9".into(),
                    length: 8,
                    align: Alignment::End,
                    fade_in_level_db: columbina_model::CrossfadeLevel::Zero,
                    fade_out_level_db: columbina_model::CrossfadeLevel::Zero,
                },
            },
        ),
        (
            "remove_transition",
            vec![],
            Op::RemoveTransition {
                timeline: "t1".into(),
                id: "x6".into(),
            },
        ),
        (
            "set_transition",
            vec![],
            Op::SetTransition {
                timeline: "t1".into(),
                id: "x6".into(),
                kind: Some(TransitionKind::DipWhite),
                length: Some(20),
                align: Some(Alignment::Start),
                fade_in_level_db: None,
                fade_out_level_db: None,
            },
        ),
        (
            "add_cue",
            vec![],
            Op::AddCue {
                timeline: "t1".into(),
                track: "S1".into(),
                cue: Cue {
                    id: String::new(),
                    start: 60,
                    length: 40,
                    text: "再见\nGoodbye".into(),
                },
            },
        ),
        (
            "remove_cue",
            vec![],
            Op::RemoveCue {
                timeline: "t1".into(),
                id: "q3".into(),
            },
        ),
        (
            "shift_cues",
            vec![Op::AddCue {
                timeline: "t1".into(),
                track: "S1".into(),
                cue: Cue {
                    id: "late".into(),
                    start: 120,
                    length: 20,
                    text: "later".into(),
                },
            }],
            Op::ShiftCues {
                timeline: "t1".into(),
                track: "S1".into(),
                cues: vec!["late".into(), "q3".into()],
                by: 30,
            },
        ),
        (
            "set_cue",
            vec![],
            Op::SetCue {
                timeline: "t1".into(),
                id: "q3".into(),
                start: Some(10),
                length: Some(30),
                text: Some("你好！".into()),
            },
        ),
        (
            "set_retime of speed points, with the length that the clip then has",
            vec![],
            held(),
        ),
        (
            "set_retime of one speed, backward, kept as the speed of the clip",
            vec![],
            written_op(serde_json::json!({
                "op": "set_retime", "timeline": "t1", "id": "c2",
                "retime": { "points": [{ "frame": 0, "speed": { "num": -2, "den": 1 } }] },
            })),
        ),
        (
            "set_retime of none, back to the speed of the source",
            vec![held()],
            written_op(serde_json::json!({
                "op": "set_retime", "timeline": "t1", "id": "c2", "retime": null,
            })),
        ),
        (
            "set_clip of a retime process that mixes two pictures, at half the speed",
            vec![],
            blended(),
        ),
        (
            "set_clip of the retime process of one picture, which a clip is written without",
            vec![blended()],
            written_op(serde_json::json!({
                "op": "set_clip", "timeline": "t1", "id": "c1", "retime_process": "nearest",
            })),
        ),
    ];
    all.extend(grade_examples());
    all.extend(sound_examples());
    all.extend(fusion_examples());
    all
}

/// An operation written as a client writes it.
fn written_op(value: serde_json::Value) -> Op {
    serde_json::from_value(value).unwrap()
}

/// `set_clip` of c1, the picture on V1, to half its speed with Frame Blend: every second frame
/// of it then lies halfway between two pictures of m1.
fn blended() -> Op {
    written_op(serde_json::json!({
        "op": "set_clip", "timeline": "t1", "id": "c1",
        "speed": { "num": 1, "den": 2 }, "retime_process": "frame_blend",
    }))
}

/// The examples of the operations on sound (T2), over a second track of sound A2 and, where they
/// need them, a bus B2, a VCA, a group, inserts and a crossfade on A1.
#[allow(clippy::too_many_lines)]
fn sound_examples() -> Vec<(&'static str, Vec<Op>, Op)> {
    use serde_json::json;
    let a2 = Op::AddTrack {
        timeline: "t1".into(),
        track: Track::new("", TrackKind::Audio),
        index: None,
    };
    let b2 = written_op(json!({"op": "add_bus", "timeline": "t1", "bus": {"id": "",
        "name": "Music", "color": "#40c080ff", "format": "stereo",
        "strip": {"outputs": [{"bus": "B1"}]}}}));
    let vca = written_op(json!({"op": "add_vca", "timeline": "t1",
        "vca": {"id": "", "name": "Dialogue"}}));
    let group = written_op(json!({"op": "add_track_group", "timeline": "t1",
        "group": {"id": "", "name": "Dialogue", "members": ["A1", "A2"]}}));
    let gains = |of: serde_json::Value| -> Vec<Op> {
        [-3, -6]
            .into_iter()
            .map(|db| {
                written_op(json!({"op": "add_insert", "timeline": "t1", "of": of,
                "insert": {"effect": {"kind": "gain", "gain_db": db}}}))
            })
            .collect()
    };
    let mut tail = Clip::of_media("tail", "m1", at(4), 100, 50);
    tail.link = None;
    let crossfade = vec![
        Op::InsertClip {
            timeline: "t1".into(),
            track: "A1".into(),
            clip: tail,
        },
        written_op(
            json!({"op": "add_transition", "timeline": "t1", "track": "A1",
            "transition": {"id": "fade", "kind": "crossfade", "left": "c2", "right": "tail",
                "length": 10, "align": "center"}}),
        ),
    ];
    vec![
        (
            "set_track_audio of a fader, a pan and a Ducker that A2 drives",
            vec![a2.clone()],
            written_op(
                json!({"op": "set_track_audio", "timeline": "t1", "track": "A1",
                "patch": {"mute": true, "strip": {"fader_db": -6, "pan": {"x": 0.25},
                    "track_fx": {"sources": [{"track": "A2"}], "amount_db": 12}}}}),
            ),
        ),
        (
            "set_track_audio of a format, which sets the channels",
            vec![a2.clone()],
            written_op(
                json!({"op": "set_track_audio", "timeline": "t1", "track": "A2",
                "patch": {"format": "mono"}}),
            ),
        ),
        (
            "add_bus",
            vec![],
            written_op(json!({"op": "add_bus", "timeline": "t1", "bus": {"id": "",
                "name": "Effects", "color": "#c04080ff", "format": "5.1",
                "strip": {"outputs": [{"bus": "B1"}],
                    "inserts": [{"effect": {"kind": "limiter", "ceiling_db": -1}}]}}})),
        ),
        (
            "remove_bus",
            vec![b2.clone()],
            written_op(json!({"op": "remove_bus", "timeline": "t1", "id": "B2"})),
        ),
        (
            "set_bus",
            vec![b2.clone()],
            written_op(json!({"op": "set_bus", "timeline": "t1", "id": "B2",
                "patch": {"name": "Score", "mute": true, "strip": {"fader_db": -3}}})),
        ),
        (
            "add_vca",
            vec![],
            written_op(json!({"op": "add_vca", "timeline": "t1",
                "vca": {"id": "", "name": "All", "fader_db": -2}})),
        ),
        (
            "remove_vca",
            vec![vca.clone()],
            written_op(json!({"op": "remove_vca", "timeline": "t1", "id": "VCA1"})),
        ),
        (
            "set_vca",
            vec![vca],
            written_op(json!({"op": "set_vca", "timeline": "t1", "id": "VCA1",
                "patch": {"fader_db": -4.5, "solo": true}})),
        ),
        (
            "add_track_group",
            vec![a2.clone()],
            written_op(json!({"op": "add_track_group", "timeline": "t1",
                "group": {"id": "", "name": "Dialogue", "members": ["A1", "A2"],
                    "linked": ["fader", "mute"]}})),
        ),
        (
            "remove_track_group",
            vec![a2.clone(), group.clone()],
            written_op(json!({"op": "remove_track_group", "timeline": "t1", "id": "TG1"})),
        ),
        (
            "set_track_group",
            vec![a2, group],
            written_op(
                json!({"op": "set_track_group", "timeline": "t1", "id": "TG1",
                "patch": {"active": false, "linked": ["fader"]}}),
            ),
        ),
        (
            "add_insert of a strip, given an id",
            gains(json!({"track": "A1"})),
            written_op(
                json!({"op": "add_insert", "timeline": "t1", "of": {"track": "A1"},
                "insert": {"effect": {"kind": "de_esser"}}, "index": 1}),
            ),
        ),
        (
            "add_insert of a clip",
            vec![],
            written_op(
                json!({"op": "add_insert", "timeline": "t1", "of": {"clip": "c2"},
                "insert": {"effect": {"kind": "pitch", "semitones": 3}}}),
            ),
        ),
        (
            "set_insert in place of another",
            gains(json!({"bus": "B1"})),
            written_op(
                json!({"op": "set_insert", "timeline": "t1", "of": {"bus": "B1"},
                "slot": 0, "insert": {"on": false, "effect": {"kind": "soft_clipper"}}}),
            ),
        ),
        (
            "set_insert to take one away",
            gains(json!({"track": "A1"})),
            written_op(
                json!({"op": "set_insert", "timeline": "t1", "of": {"track": "A1"},
                "slot": 0, "insert": null}),
            ),
        ),
        (
            "move_insert",
            gains(json!({"track": "A1"})),
            written_op(
                json!({"op": "move_insert", "timeline": "t1", "of": {"track": "A1"},
                "from": 1, "to": 0}),
            ),
        ),
        (
            "set_clip of the sound of schema 4",
            vec![],
            written_op(json!({"op": "set_clip", "timeline": "t1", "id": "c2",
                "eq": {"on": true, "bands": [
                    {"on": true, "kind": "high_pass", "freq": 80, "gain_db": 0, "q": 0.7},
                    {"kind": "bell", "freq": 400, "gain_db": -3, "q": 1},
                    {"kind": "bell", "freq": 3000, "gain_db": 2, "q": 1},
                    {"kind": "high_shelf", "freq": 10000, "gain_db": 0, "q": 0.7}]},
                "effects": [{"effect": {"kind": "gain", "gain_db": 2}}],
                "pitch_correction": true, "pitch": {"semitones": -2, "cents": 25},
                "channels": [{"source": {"stream": 1, "channel": 1}},
                    {"source": {"stream": 1, "channel": 0}, "level_db": -3}]})),
        ),
        (
            "set_transition of the levels of a crossfade",
            crossfade,
            written_op(
                json!({"op": "set_transition", "timeline": "t1", "id": "fade",
                "fade_in_level_db": 3, "fade_out_level_db": -3}),
            ),
        ),
        (
            "set_timeline_audio",
            vec![b2.clone()],
            written_op(json!({"op": "set_timeline_audio", "timeline": "t1",
                "patch": {"monitor": "B2", "exclusive_solo": true,
                    "loudness": {"standard": "youtube", "target": -14}}})),
        ),
        (
            "set_lane of a fader",
            vec![],
            written_op(
                json!({"op": "set_lane", "timeline": "t1", "of": {"track": "A1"},
                "path": "fader_db", "lane": {"points": [
                    {"at": {"ticks": 0, "scale": 48000}, "value": -10},
                    {"at": {"ticks": 48000, "scale": 48000}, "value": 0}]}}),
            ),
        ),
        (
            "set_lane of null, of the mute of a bus",
            vec![
                b2,
                written_op(
                    json!({"op": "set_lane", "timeline": "t1", "of": {"bus": "B2"},
                    "path": "mute", "lane": {"points": [
                        {"at": {"ticks": 24000, "scale": 48000}, "value": 1}], "kind": "switch"}}),
                ),
            ],
            written_op(
                json!({"op": "set_lane", "timeline": "t1", "of": {"bus": "B2"},
                "path": "mute", "lane": null}),
            ),
        ),
        (
            "splice_lane",
            vec![written_op(
                json!({"op": "set_lane", "timeline": "t1", "of": {"track": "A1"},
                "path": "pan.x", "lane": {"points": [
                    {"at": {"ticks": 0, "scale": 48000}, "value": -1},
                    {"at": {"ticks": 4800, "scale": 48000}, "value": 0},
                    {"at": {"ticks": 9600, "scale": 48000}, "value": 1}]}}),
            )],
            written_op(
                json!({"op": "splice_lane", "timeline": "t1", "of": {"track": "A1"},
                "path": "pan.x", "at": 1, "remove": 1, "points": [
                    {"at": {"ticks": 2400, "scale": 48000}, "value": 0.5},
                    {"at": {"ticks": 7200, "scale": 48000}, "value": -0.5}]}),
            ),
        ),
    ]
}

/// The grade of c1.
fn c1() -> Target {
    Target::Clip {
        timeline: "t1".into(),
        clip: "c1".into(),
    }
}

/// Put a corrector, n9 in the base, between the source and the output of the grade of c1.
fn first_node() -> Op {
    Op::AddNode {
        grade: c1(),
        node: Node::corrector(""),
        index: None,
        links: vec![Link::rgb("source", ""), Link::rgb("", "output")],
    }
}

/// Add group k9 (k10 after `first_node`), with no clip in it.
fn group(name: &str) -> Op {
    Op::AddGroup {
        group: Group::new("", name),
        index: None,
    }
}

/// A shot match, whole.
fn shot_match() -> serde_json::Value {
    serde_json::json!({
        "gamma": {"r": 0.95, "g": 1, "b": 1.08},
        "gain": {"r": 1.1, "g": 1, "b": 0.92},
        "offset": {"r": 0.01, "g": 0, "b": -0.015},
        "contrast": 1.1,
        "pivot": 0.435,
    })
}

/// Set the shot match of n9, the node of `first_node`.
fn set_shot_match(value: serde_json::Value) -> Op {
    let mut params = serde_json::Map::new();
    params.insert("shot_match".into(), value);
    Op::SetNode {
        grade: c1(),
        id: "n9".into(),
        label: None,
        enabled: None,
        locked: None,
        pos: None,
        params: Some(params.into()),
    }
}

/// Three keys on the saturation of n9, the node of `first_node`: the second eased out and in.
fn saturation_keys() -> Op {
    let third = 1.0 / 3.0;
    let key = |frame: i64, value: f64, interp: Interp| Key {
        frame,
        value,
        interp,
    };
    Op::SetKeys {
        grade: c1(),
        id: "n9".into(),
        path: "primaries.saturation".into(),
        param: Param::Keys(vec![
            key(0, 50.0, Interp::Linear),
            key(
                24,
                80.0,
                Interp::Bezier {
                    out: [third, 0.0],
                    into: [third, 10.0],
                },
            ),
            key(48, 50.0, Interp::Linear),
        ]),
    }
}

/// Keys of how far c1 is moved right: it eases from the left edge to where it is fitted.
fn position_keys() -> Op {
    let third = 1.0 / 3.0;
    Op::SetClipKeys {
        timeline: "t1".into(),
        id: "c1".into(),
        path: "transform.x".into(),
        param: Param::Keys(vec![
            Key {
                frame: 0,
                value: -0.5,
                interp: Interp::Bezier {
                    out: [third, 0.0],
                    into: [third, 0.0],
                },
            },
            Key {
                frame: 24,
                value: 0.0,
                interp: Interp::Linear,
            },
        ]),
    }
}

/// Set n9, the node of `first_node`, by `params`.
/// Keys of the size of the letters of the title c8: they grow over its first second.
fn size_keys() -> Op {
    Op::SetClipKeys {
        timeline: "t1".into(),
        id: "c8".into(),
        path: "title.size".into(),
        param: Param::Keys(vec![
            Key {
                frame: 0,
                value: 48.0,
                interp: Interp::Linear,
            },
            Key {
                frame: 24,
                value: 96.0,
                interp: Interp::Hold,
            },
        ]),
    }
}

/// Keys of how far down the anchor of the title c8 is: it rises from below the picture.
fn anchor_keys() -> Op {
    Op::SetClipKeys {
        timeline: "t1".into(),
        id: "c8".into(),
        path: "title.y".into(),
        param: Param::Keys(vec![
            Key {
                frame: 0,
                value: 1.25,
                interp: Interp::Linear,
            },
            Key {
                frame: 40,
                value: 0.5,
                interp: Interp::Linear,
            },
        ]),
    }
}

fn set_n9(params: serde_json::Value) -> Op {
    Op::SetNode {
        grade: c1(),
        id: "n9".into(),
        label: None,
        enabled: None,
        locked: None,
        pos: None,
        params: Some(params),
    }
}

/// A qualifier, whole: the oranges of a picture, its edge blurred.
fn qualifier() -> serde_json::Value {
    let mut whole = serde_json::to_value(columbina_model::Qualifier::default()).unwrap();
    whole["hue"]["center"] = 8.0.into();
    whole["hue"]["width"] = 10.0.into();
    whole["hue"]["soft"] = 4.0.into();
    whole["sat"]["low"] = 30.0.into();
    whole["sat"]["low_soft"] = 10.0.into();
    whole["finesse"]["clean_black"] = 20.0.into();
    whole["finesse"]["blur_radius"] = 30.0.into();
    whole
}

/// A window list, containing all five shapes and keeping an unfinished Curve draft.
fn windows() -> serde_json::Value {
    use columbina_model::grade::{Window, WindowShape};
    serde_json::to_value([
        Window::new("w1", WindowShape::linear()),
        Window::new("w2", WindowShape::circular()),
        Window::new("w3", WindowShape::polygon()),
        Window::new("w4", WindowShape::curve()),
        Window::new("w5", WindowShape::gradient()),
    ])
    .unwrap()
}

/// Lock n9, the node of `first_node`, where it lies at (120, 30).
fn lock_n9() -> Op {
    Op::SetNode {
        grade: c1(),
        id: "n9".into(),
        label: None,
        enabled: None,
        locked: Some(true),
        pos: Some(Some(Pos { x: 120.0, y: 30.0 })),
        params: None,
    }
}

/// The examples of the operations on grades and groups.
#[allow(clippy::too_many_lines)]
fn grade_examples() -> Vec<(&'static str, Vec<Op>, Op)> {
    let after_n9 = Op::AddNode {
        grade: c1(),
        node: Node::corrector(""),
        index: None,
        links: vec![Link::rgb("n9", ""), Link::rgb("", "output")],
    };
    let keys = vec![
        Key {
            frame: 0,
            value: 1.0,
            interp: Interp::Linear,
        },
        Key {
            frame: 50,
            value: 1.4,
            interp: Interp::Bezier {
                out: [0.33, 0.0],
                into: [0.33, 0.0],
            },
        },
        Key {
            frame: 100,
            value: 1.2,
            interp: Interp::Hold,
        },
    ];
    let mut graded = Grade {
        nodes: vec![Node::corrector("n3"), Node::new("n4", Kind::LayerMixer)],
        links: vec![
            Link::rgb("source", "n3"),
            Link::rgb("n3", "n4"),
            Link {
                from: "source".into(),
                to: "n4".into(),
                kind: Port::Rgb,
                input: 1,
            },
            Link::rgb("n4", "output"),
        ],
    };
    graded.nodes[0].label = "Look".into();
    vec![
        (
            "add_node between the source and the output",
            vec![],
            first_node(),
        ),
        (
            "add_node of a parallel mixer, unlinked, at the start",
            vec![first_node()],
            Op::AddNode {
                grade: Target::Timeline {
                    timeline: "t1".into(),
                },
                node: Node::new("", Kind::ParallelMixer),
                index: Some(0),
                links: vec![],
            },
        ),
        (
            "remove_node, bridged",
            vec![first_node(), after_n9.clone()],
            Op::RemoveNode {
                grade: c1(),
                id: "n9".into(),
                links: None,
            },
        ),
        (
            "remove_node with the links to put in its place",
            vec![first_node(), after_n9.clone()],
            Op::RemoveNode {
                grade: c1(),
                id: "n10".into(),
                links: Some(vec![]),
            },
        ),
        (
            "connect a key",
            vec![first_node(), after_n9.clone()],
            Op::Connect {
                grade: c1(),
                link: Link {
                    from: "n9".into(),
                    to: "n10".into(),
                    kind: Port::Key,
                    input: 0,
                },
            },
        ),
        (
            "connect in place of a link",
            vec![first_node()],
            Op::Connect {
                grade: c1(),
                link: Link::rgb("source", "output"),
            },
        ),
        (
            "disconnect",
            vec![first_node()],
            Op::Disconnect {
                grade: c1(),
                to: "output".into(),
                kind: Port::Rgb,
                input: 0,
            },
        ),
        (
            "set_node",
            vec![first_node()],
            Op::SetNode {
                grade: c1(),
                id: "n9".into(),
                label: Some("Skin".into()),
                enabled: Some(false),
                locked: None,
                pos: None,
                params: Some(serde_json::json!({
                    "primaries": {"saturation": 60, "lift": {"r": -0.05}},
                    "lut": "LUTs/film.cube",
                })),
            },
        ),
        ("set_keys", vec![first_node()], saturation_keys()),
        (
            "set_keys of a number in place of keys",
            vec![first_node(), saturation_keys()],
            Op::SetKeys {
                grade: c1(),
                id: "n9".into(),
                path: "primaries.saturation".into(),
                param: Param::Fixed(62.5),
            },
        ),
        (
            "splice_keys",
            vec![first_node(), saturation_keys()],
            Op::SpliceKeys {
                grade: c1(),
                id: "n9".into(),
                path: "primaries.saturation".into(),
                at: 1,
                remove: 1,
                keys: vec![
                    Key {
                        frame: 12,
                        value: 70.0,
                        interp: Interp::Hold,
                    },
                    Key {
                        frame: 36,
                        value: 55.5,
                        interp: Interp::Linear,
                    },
                ],
            },
        ),
        (
            "splice_keys of the first keys of a setting, done as set_keys",
            vec![first_node()],
            Op::SpliceKeys {
                grade: c1(),
                id: "n9".into(),
                path: "primaries.lift.r".into(),
                at: 0,
                remove: 0,
                keys: vec![Key {
                    frame: -4,
                    value: 0.125,
                    interp: Interp::Linear,
                }],
            },
        ),
        (
            "set_clip of a crop and a composite mode",
            vec![],
            serde_json::from_value(serde_json::json!({
                "op": "set_clip", "timeline": "t1", "id": "c1", "composite": "multiply",
                "crop": {"left": 0.125, "right": 0.25, "top": 0, "bottom": 0.5, "softness": 0.0625},
            }))
            .unwrap(),
        ),
        (
            "set_clip_keys of a side of a crop",
            vec![],
            Op::SetClipKeys {
                timeline: "t1".into(),
                id: "c1".into(),
                path: "crop.left".into(),
                param: Param::Keys(vec![
                    Key {
                        frame: 0,
                        value: 1.0,
                        interp: Interp::Linear,
                    },
                    Key {
                        frame: 20,
                        value: 0.0,
                        interp: Interp::Hold,
                    },
                ]),
            },
        ),
        ("set_clip_keys", vec![], position_keys()),
        (
            "set_clip_keys of a number in place of keys",
            vec![position_keys()],
            Op::SetClipKeys {
                timeline: "t1".into(),
                id: "c1".into(),
                path: "transform.x".into(),
                param: Param::Fixed(0.25),
            },
        ),
        (
            "splice_clip_keys",
            vec![position_keys()],
            Op::SpliceClipKeys {
                timeline: "t1".into(),
                id: "c1".into(),
                path: "transform.x".into(),
                at: 1,
                remove: 1,
                keys: vec![
                    Key {
                        frame: 12,
                        value: -0.125,
                        interp: Interp::Hold,
                    },
                    Key {
                        frame: 36,
                        value: 0.0,
                        interp: Interp::Linear,
                    },
                ],
            },
        ),
        (
            "splice_clip_keys of the first keys of a setting, done as set_clip_keys",
            vec![],
            Op::SpliceClipKeys {
                timeline: "t1".into(),
                id: "c1".into(),
                path: "transform.opacity".into(),
                at: 0,
                remove: 0,
                keys: vec![Key {
                    frame: 10,
                    value: 0.5,
                    interp: Interp::Linear,
                }],
            },
        ),
        ("set_clip_keys of the size of a title", vec![], size_keys()),
        (
            "set_clip_keys of a number in place of the keys of a title",
            vec![size_keys()],
            Op::SetClipKeys {
                timeline: "t1".into(),
                id: "c8".into(),
                path: "title.size".into(),
                param: Param::Fixed(120.0),
            },
        ),
        (
            "splice_clip_keys of where a title is",
            vec![anchor_keys()],
            Op::SpliceClipKeys {
                timeline: "t1".into(),
                id: "c8".into(),
                path: "title.y".into(),
                at: 1,
                remove: 0,
                keys: vec![Key {
                    frame: 20,
                    value: 0.625,
                    interp: Interp::Linear,
                }],
            },
        ),
        (
            "set_clip of a title with keys in its style",
            vec![],
            serde_json::from_value(serde_json::json!({
                "op": "set_clip", "timeline": "t1", "id": "c8",
                "source": {"kind": "title", "text": "Hello", "style": {
                    "font": "", "size": 96, "color": "#ffffffff", "y": 0.5, "align": "left",
                    "x": {"keys": [
                        {"frame": 0, "value": -0.25, "interp": "linear"},
                        {"frame": 49, "value": 1.0, "interp": "linear"},
                    ]},
                }},
            }))
            .unwrap(),
        ),
        (
            "set_node with keyframes",
            vec![first_node()],
            Op::SetNode {
                grade: c1(),
                id: "n9".into(),
                label: None,
                enabled: None,
                locked: None,
                pos: None,
                params: Some(serde_json::json!({"primaries": {"contrast": {"keys": keys}}})),
            },
        ),
        (
            "set_node with a shot match, given whole",
            vec![first_node()],
            set_shot_match(shot_match()),
        ),
        (
            "set_node of a part of a shot match",
            vec![first_node(), set_shot_match(shot_match())],
            set_shot_match(serde_json::json!({"gain": {"g": 1.05}, "contrast": 1.2})),
        ),
        (
            "set_node taking a shot match away",
            vec![first_node(), set_shot_match(shot_match())],
            set_shot_match(serde_json::Value::Null),
        ),
        (
            "set_node with a qualifier, given whole",
            vec![first_node()],
            set_n9(serde_json::json!({ "qualifier": qualifier() })),
        ),
        (
            "set_node of a part of a qualifier, a range keyframed",
            vec![
                first_node(),
                set_n9(serde_json::json!({ "qualifier": qualifier() })),
            ],
            set_n9(serde_json::json!({"qualifier": {
                "mode": "lum",
                "lum": {"high": {"keys": [
                    {"frame": 0, "value": 60, "interp": "linear"},
                    {"frame": 12, "value": 80, "interp": "linear"},
                ]}},
                "finesse": {"morph": "grow", "morph_radius": 2},
            }})),
        ),
        (
            "set_node taking a qualifier away",
            vec![
                first_node(),
                set_n9(serde_json::json!({ "qualifier": qualifier() })),
            ],
            set_n9(serde_json::json!({ "qualifier": null })),
        ),
        (
            "set_node with all five Power Window shapes, including a Curve draft",
            vec![first_node()],
            set_n9(serde_json::json!({ "windows": windows() })),
        ),
        (
            "set_node replacing the window list with an animated subtractive window",
            vec![
                first_node(),
                set_n9(serde_json::json!({ "windows": windows() })),
            ],
            set_n9({
                let mut list = windows();
                let window = &mut list[1];
                window["combine"] = "subtract".into();
                window["opacity"] = 75.0.into();
                window["transform"]["pan"] = serde_json::json!({ "keys": [
                    {"frame": 100, "value": 25, "interp": "linear"},
                    {"frame": 120, "value": 75, "interp": "linear"},
                ] });
                serde_json::json!({ "windows": [window] })
            }),
        ),
        (
            "set_node removing all Power Windows",
            vec![
                first_node(),
                set_n9(serde_json::json!({ "windows": windows() })),
            ],
            set_n9(serde_json::json!({ "windows": [] })),
        ),
        (
            "add_node of an Outside node, as add_outside adds it after n9",
            vec![first_node()],
            Op::AddNode {
                grade: c1(),
                node: {
                    let mut outside = Node::corrector("");
                    if let columbina_model::NodeKind::Corrector(corrector) = &mut outside.kind {
                        corrector.key.input.invert = true;
                    }
                    outside
                },
                index: None,
                links: vec![
                    Link::rgb("n9", ""),
                    Link {
                        from: "n9".into(),
                        to: String::new(),
                        kind: columbina_model::Port::Key,
                        input: 0,
                    },
                    Link::rgb("", "output"),
                ],
            },
        ),
        (
            "set_node of the key of a qualifier and the blur of the key coming in",
            vec![first_node()],
            set_n9(serde_json::json!({"key": {
                "qualifier": {"invert": true, "gain": 0.8},
                "input_blur": {"radius": 50, "ratio": 0.7},
            }})),
        ),
        (
            "set_node moving a node and locking it",
            vec![first_node()],
            Op::SetNode {
                grade: c1(),
                id: "n9".into(),
                label: None,
                enabled: None,
                locked: Some(true),
                pos: Some(Some(Pos { x: 250.0, y: 40.5 })),
                params: None,
            },
        ),
        (
            "set_node unlocking a node and setting it",
            vec![first_node(), lock_n9()],
            Op::SetNode {
                grade: c1(),
                id: "n9".into(),
                label: None,
                enabled: None,
                locked: Some(false),
                pos: None,
                params: Some(serde_json::json!({"primaries": {"saturation": 40}})),
            },
        ),
        (
            "set_node putting a node back where the editor lays it out",
            vec![first_node(), lock_n9()],
            Op::SetNode {
                grade: c1(),
                id: "n9".into(),
                label: None,
                enabled: None,
                locked: None,
                pos: Some(None),
                params: None,
            },
        ),
        (
            "set_grade of a timeline",
            vec![],
            Op::SetGrade {
                grade: Target::Timeline {
                    timeline: "t1".into(),
                },
                value: graded,
            },
        ),
        ("add_group", vec![], group("Interview")),
        (
            "remove_group",
            vec![group("Interview")],
            Op::RemoveGroup { id: "k9".into() },
        ),
        (
            "set_group",
            vec![group("Interview")],
            Op::SetGroup {
                id: "k9".into(),
                name: Some("Exteriors".into()),
            },
        ),
        (
            "set_clip_group",
            vec![group("Interview")],
            Op::SetClipGroup {
                timeline: "t1".into(),
                clip: "c1".into(),
                group: Some("k9".into()),
            },
        ),
        (
            "add_node to the pre-clip grade of a group",
            vec![group("Interview")],
            Op::AddNode {
                grade: Target::PreClip { group: "k9".into() },
                node: Node::corrector(""),
                index: None,
                links: vec![Link::rgb("source", ""), Link::rgb("", "output")],
            },
        ),
    ]
}

/// Give c1, the picture on V1, the composition that a clip has at first.
fn first_composition() -> Op {
    Op::SetFusion {
        timeline: "t1".into(),
        clip: "c1".into(),
        value: Some(Composition::default()),
    }
}

/// Put a Merge, n9 in the base, before the `MediaOut` of c1, the picture of c1 its foreground.
fn merge_node() -> Op {
    Op::AddFusionNode {
        timeline: "t1".into(),
        clip: "c1".into(),
        node: fusion::Node::new("", fusion::Kind::Merge),
        index: None,
        links: vec![
            fusion::Link::new(fusion::MEDIA_IN, "", Input::Foreground),
            fusion::Link::new("", fusion::MEDIA_OUT, Input::Input),
        ],
    }
}

/// Add a Background, n10 after `merge_node`, as the background of the Merge n9.
fn background_node() -> Op {
    Op::AddFusionNode {
        timeline: "t1".into(),
        clip: "c1".into(),
        node: fusion::Node::new("", fusion::Kind::Background),
        index: None,
        links: vec![fusion::Link::new("", "n9", Input::Background)],
    }
}

/// Keys of the Blend of the Merge n9: the foreground comes in over a second.
fn blend_keys() -> Op {
    Op::SetFusionKeys {
        timeline: "t1".into(),
        clip: "c1".into(),
        path: "n9.blend".into(),
        param: Param::Keys(vec![
            Key {
                frame: 0,
                value: 0.0,
                interp: Interp::Linear,
            },
            Key {
                frame: 24,
                value: 1.0,
                interp: Interp::Hold,
            },
        ]),
    }
}

/// The examples of the operations on the composition of a clip.
#[allow(clippy::too_many_lines)]
fn fusion_examples() -> Vec<(&'static str, Vec<Op>, Op)> {
    let merged = || vec![first_composition(), merge_node(), background_node()];
    let mut moved = Composition {
        nodes: vec![
            fusion::Node::new("in", fusion::Kind::MediaIn),
            fusion::Node::new("move", fusion::Kind::Transform),
            fusion::Node::new("out", fusion::Kind::MediaOut),
        ],
        // Given out of their order: they are done in it.
        links: vec![
            fusion::Link::new("move", "out", Input::Input),
            fusion::Link::new("in", "move", Input::Input),
        ],
    };
    moved.nodes[1].label = "Slide".into();
    moved.nodes[1].pos = Some(Pos { x: 150.0, y: 0.0 });
    vec![
        (
            "set_fusion of the composition that a clip has at first",
            vec![],
            first_composition(),
        ),
        (
            "set_fusion of a composition in place of another",
            vec![first_composition()],
            Op::SetFusion {
                timeline: "t1".into(),
                clip: "c1".into(),
                value: Some(moved),
            },
        ),
        (
            "set_fusion on a title, whose composition is of the size of the timeline",
            vec![],
            Op::SetFusion {
                timeline: "t1".into(),
                clip: "c8".into(),
                value: Some(Composition::default()),
            },
        ),
        (
            "set_fusion of none, which takes the composition away",
            vec![first_composition()],
            written_op(serde_json::json!({
                "op": "set_fusion", "timeline": "t1", "clip": "c1", "value": null,
            })),
        ),
        (
            "add_fusion_node of a merge, in place of the link into the media_out",
            vec![first_composition()],
            merge_node(),
        ),
        (
            "add_fusion_node of a background, linked to the merge",
            vec![first_composition(), merge_node()],
            background_node(),
        ),
        (
            "add_fusion_node of a transform with an id of its own, unlinked, at the start",
            vec![first_composition()],
            written_op(serde_json::json!({
                "op": "add_fusion_node", "timeline": "t1", "clip": "c1", "index": 0,
                "node": {
                    "id": "shake", "kind": "transform", "label": "Shake", "params": {
                        "center": {"x": 0.5, "y": 0.5}, "size": 1.25, "angle": 0,
                        "flip_horizontal": true, "flip_vertical": false,
                    },
                },
            })),
        ),
        (
            "remove_fusion_node, with its links",
            merged(),
            Op::RemoveFusionNode {
                timeline: "t1".into(),
                clip: "c1".into(),
                id: "n9".into(),
                links: None,
            },
        ),
        (
            "remove_fusion_node with the links to put in its place",
            merged(),
            Op::RemoveFusionNode {
                timeline: "t1".into(),
                clip: "c1".into(),
                id: "n9".into(),
                links: Some(vec![fusion::Link::new(
                    fusion::MEDIA_IN,
                    fusion::MEDIA_OUT,
                    Input::Input,
                )]),
            },
        ),
        (
            "connect_fusion in place of a link",
            merged(),
            Op::ConnectFusion {
                timeline: "t1".into(),
                clip: "c1".into(),
                link: fusion::Link::new(fusion::MEDIA_IN, "n9", Input::Background),
            },
        ),
        (
            "connect_fusion into an input that had no link",
            vec![first_composition(), merge_node()],
            Op::ConnectFusion {
                timeline: "t1".into(),
                clip: "c1".into(),
                link: fusion::Link::new(fusion::MEDIA_IN, "n9", Input::Background),
            },
        ),
        (
            "disconnect_fusion",
            merged(),
            Op::DisconnectFusion {
                timeline: "t1".into(),
                clip: "c1".into(),
                to: fusion::MEDIA_OUT.into(),
                input: Input::Input,
            },
        ),
        (
            "set_fusion_node",
            merged(),
            Op::SetFusionNode {
                timeline: "t1".into(),
                clip: "c1".into(),
                id: "n9".into(),
                label: Some("Over the sky".into()),
                pass_through: Some(true),
                pos: Some(Some(Pos { x: 120.0, y: 30.0 })),
                params: Some(serde_json::json!({
                    "apply": "screen", "blend": 0.5, "center": {"x": 0.25}, "angle": 15,
                })),
            },
        ),
        (
            "set_fusion_node of the colour of a background",
            merged(),
            Op::SetFusionNode {
                timeline: "t1".into(),
                clip: "c1".into(),
                id: "n10".into(),
                label: None,
                pass_through: None,
                pos: None,
                params: Some(serde_json::json!({"color": {"r": 0.1, "g": 0.2, "b": 0.6}})),
            },
        ),
        ("set_fusion_keys", merged(), blend_keys()),
        (
            "set_fusion_keys of a number in place of keys",
            {
                let mut setup = merged();
                setup.push(blend_keys());
                setup
            },
            Op::SetFusionKeys {
                timeline: "t1".into(),
                clip: "c1".into(),
                path: "n9.blend".into(),
                param: Param::Fixed(0.75),
            },
        ),
        (
            "splice_fusion_keys",
            {
                let mut setup = merged();
                setup.push(blend_keys());
                setup
            },
            Op::SpliceFusionKeys {
                timeline: "t1".into(),
                clip: "c1".into(),
                path: "n9.blend".into(),
                at: 1,
                remove: 1,
                keys: vec![
                    Key {
                        frame: 12,
                        value: 0.75,
                        interp: Interp::Bezier {
                            out: [0.33, 0.0],
                            into: [0.33, 0.0],
                        },
                    },
                    Key {
                        frame: 36,
                        value: 1.0,
                        interp: Interp::Linear,
                    },
                ],
            },
        ),
        (
            "splice_fusion_keys of the first keys of a setting, done as set_fusion_keys",
            merged(),
            Op::SpliceFusionKeys {
                timeline: "t1".into(),
                clip: "c1".into(),
                path: "n10.color.a".into(),
                at: 0,
                remove: 0,
                keys: vec![Key {
                    frame: -4,
                    value: 0.5,
                    interp: Interp::Linear,
                }],
            },
        ),
    ]
}

/// The examples as they are written: one object to a line, in a list.
fn written() -> String {
    let base = base();
    let lines: Vec<String> = examples()
        .into_iter()
        .map(|(name, setup, op)| {
            let mut before = base.clone();
            for step in setup {
                apply(&mut before, step).unwrap_or_else(|e| panic!("{name}: {e}"));
            }
            let mut after = before.clone();
            let (done, inverse) =
                apply_done(&mut after, op.clone()).unwrap_or_else(|e| panic!("{name}: {e}"));
            let example = serde_json::json!({
                "name": name,
                "before": before,
                "op": op,
                "done": done,
                "inverse": inverse,
                "after": after,
            });
            serde_json::to_string(&example).unwrap()
        })
        .collect();
    format!("[\n{}\n]\n", lines.join(",\n"))
}

#[test]
fn the_examples_of_the_operations_are_as_the_engine_applies_them() {
    let text = written();
    let names: Vec<String> = serde_json::from_str::<serde_json::Value>(&text)
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|example| example["op"]["op"].as_str().unwrap().to_owned())
        .collect();
    // Every operation has an example.
    for op in [
        "add_media",
        "remove_media",
        "add_timeline",
        "remove_timeline",
        "add_track",
        "remove_track",
        "move_track",
        "insert_clip",
        "remove_clip",
        "move_clip",
        "shift_clips",
        "trim_clip",
        "set_proxy",
        "set_timeline",
        "set_track",
        "set_clip",
        "add_bin",
        "remove_bin",
        "set_bin",
        "set_media",
        "add_transition",
        "remove_transition",
        "set_transition",
        "add_cue",
        "remove_cue",
        "set_cue",
        "shift_cues",
        "add_node",
        "remove_node",
        "connect",
        "disconnect",
        "set_node",
        "set_keys",
        "splice_keys",
        "set_clip_keys",
        "splice_clip_keys",
        "set_retime",
        "set_grade",
        "add_group",
        "remove_group",
        "set_group",
        "set_clip_group",
        "set_color_settings",
        "set_media_color",
        "set_media_audio",
        "set_track_audio",
        "add_bus",
        "remove_bus",
        "set_bus",
        "add_vca",
        "remove_vca",
        "set_vca",
        "add_track_group",
        "remove_track_group",
        "set_track_group",
        "add_insert",
        "set_insert",
        "move_insert",
        "set_timeline_audio",
        "set_lane",
        "splice_lane",
        "set_fusion",
        "add_fusion_node",
        "remove_fusion_node",
        "connect_fusion",
        "disconnect_fusion",
        "set_fusion_node",
        "set_fusion_keys",
        "splice_fusion_keys",
    ] {
        assert!(names.iter().any(|name| name == op), "no example of {op}");
    }
    if std::env::var("COLUMBINA_WRITE_EXAMPLES").as_deref() == Ok("1") {
        std::fs::write(FILE, &text).unwrap();
        return;
    }
    let kept = std::fs::read_to_string(Path::new(FILE)).unwrap_or_default();
    let read = |text: &str| serde_json::from_str::<serde_json::Value>(text).ok();
    assert!(
        read(&kept).is_some() && read(&kept) == read(&text),
        "{FILE} is not as the engine applies the operations now: run the tests of \
         columbina-model with COLUMBINA_WRITE_EXAMPLES=1 to write it anew"
    );
}
