//! Examples of every operation, as the engine applies them: the project before, the operation as
//! it was given, the operation as it was done and the one that takes it back, and the project
//! after. They are kept in `tests/data/ops-examples.json`, one to a line, for the copies of the
//! project that the clients keep to be checked against the engine.
//!
//! The file is written anew when `COLUMBINA_WRITE_EXAMPLES=1` is set; otherwise it is checked to
//! be as the engine applies the operations now.

use std::path::Path;

use columbina_model::{
    apply, apply_done, Alignment, Bin, Clip, Cue, Op, Project, Proxy, Rate, Rgba, Source, Speed,
    TextAlign, Time, TitleStyle, Track, TrackKind, Transform, Transition, TransitionKind,
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
            size: 96,
            color: Rgba([255, 255, 255, 255]),
            x: 0.5,
            y: 0.5,
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
        gain_db: None,
        pan: None,
        fade_in: None,
        fade_out: None,
        source: None,
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
        *transform = Some(Transform {
            x: 0.25,
            y: -0.125,
            scale: 1.5,
            rotation: 90.0,
            opacity: 0.5,
        });
        *gain_db = Some(-6.0);
        *pan = Some(-0.5);
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
    let add_bin = |parent: Option<&str>| Op::AddBin {
        bin: Bin {
            id: String::new(),
            name: "Selects".into(),
            parent: parent.map(Into::into),
        },
        index: None,
    };
    vec![
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
                enabled: Some(false),
                locked: Some(true),
                language: Some("zh-Hans".into()),
            },
        ),
        ("set_clip", vec![], fast),
        ("set_clip out of its group", vec![], unlinked),
        ("set_clip in a new group", vec![], new_group),
        ("set_clip of a title", vec![], retitled),
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
                id: "m1".into(),
                name: Some("Talk".into()),
                bin: Some(Some("b4".into())),
                mark_in: Some(Some(at(1))),
                mark_out: Some(Some(at(8))),
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
