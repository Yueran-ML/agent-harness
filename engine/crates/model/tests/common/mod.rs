//! What the tests of random operations share: the makings of an operation, the operation that
//! they make against a project as it is then, the projects to start from, and the check that a
//! project is still one that the operations could have made.

use std::collections::HashSet;

use columbina_model::span::{source_span, source_tick};
use columbina_model::{
    apply, Alignment, Bin, Clip, Cue, Media, Op, Project, Proxy, Rate, Rgba, Source, Speed,
    TextAlign, Time, Timeline, TitleStyle, Track, TrackKind, Transform, Transition, TransitionKind,
    LARGEST,
};
use proptest::prelude::*;

pub mod colors;
pub mod grades;
pub mod sounds;

/// The levels of a side of a crossfade.
const LEVELS: [columbina_model::CrossfadeLevel; 3] = [
    columbina_model::CrossfadeLevel::Minus3,
    columbina_model::CrossfadeLevel::Zero,
    columbina_model::CrossfadeLevel::Plus3,
];

/// The rates of the timelines that are added.
const RATES: [Rate; 3] = [
    Rate { num: 25, den: 1 },
    Rate {
        num: 30000,
        den: 1001,
    },
    Rate {
        num: 24000,
        den: 1001,
    },
];

/// The kinds of operation, as `op_of` numbers them, each as often as it is wanted: clips are
/// put in, moved, trimmed and set most often, and timelines are taken away least. 27 is an
/// operation on grades or groups, 28 one of colour management, and 29 one of sound.
const KINDS: [u8; 72] = [
    0, 0, 1, 2, 3, 4, 4, 5, 6, 6, 6, 6, 6, 6, 7, 7, 8, 8, 8, 8, 9, 9, 9, 9, 10, 10, 11, 12, 12, 13,
    13, 13, 13, 14, 15, 16, 17, 17, 18, 18, 18, 19, 20, 20, 21, 21, 22, 23, 24, 24, 24, 25, 26, 26,
    26, 27, 27, 27, 27, 27, 27, 28, 28, 28, 29, 29, 29, 29, 29, 29, 29, 29,
];

/// The speeds that a clip is set to, the last of them too slow.
const SPEEDS: [Speed; 7] = [
    Speed::ONE,
    Speed { num: 2, den: 1 },
    Speed { num: 1, den: 2 },
    Speed { num: 3, den: 2 },
    Speed { num: 4, den: 5 },
    Speed { num: 100, den: 1 },
    Speed { num: 1, den: 101 },
];

/// The makings of an operation, which are turned into one against the project as it is then.
#[derive(Clone, Debug)]
pub struct Spec {
    kind: usize,
    a: usize,
    b: usize,
    c: usize,
    start: i64,
    length: i64,
    /// How many ticks past the end of its media a clip that is cut near the end goes.
    slack: i64,
    near_end: bool,
    named: bool,
    /// Which of the settings that an operation may set it sets, a bit for each.
    d: usize,
}

pub fn spec() -> impl Strategy<Value = Spec> {
    (
        (
            0..KINDS.len(),
            any::<usize>(),
            any::<usize>(),
            any::<usize>(),
            any::<usize>(),
        ),
        // Now and then a start whose end JavaScript could not read.
        prop_oneof![39 => 0i64..400, 1 => Just(-1i64), 1 => Just(LARGEST - 10)],
        prop_oneof![19 => 1i64..120, 1 => Just(0i64)],
        -2i64..=2,
        any::<bool>(),
        prop::bool::weighted(0.2),
    )
        .prop_map(
            |((kind, a, b, c, d), start, length, slack, near_end, named)| Spec {
                kind,
                a,
                b,
                c,
                start,
                length,
                slack,
                near_end,
                named,
                d,
            },
        )
}

/// One of `ids`, or one time in sixteen `bogus`, which names nothing.
fn pick(ids: &[String], n: usize, bogus: &str) -> String {
    if n.is_multiple_of(16) || ids.is_empty() {
        bogus.into()
    } else {
        ids[n / 16 % ids.len()].clone()
    }
}

/// An id that the engine is to give, or one of `names`, some of which are held or reserved.
fn id_of(spec: &Spec, names: &[&str]) -> String {
    if spec.named {
        names[spec.c % names.len()].into()
    } else {
        String::new()
    }
}

/// An index into a list of `len`, or none; one time in sixteen past its end.
fn index_of(n: usize, len: usize) -> Option<usize> {
    match n % 16 {
        0..=5 => None,
        6 => Some(len + 1),
        _ => Some(n / 16 % (len + 1)),
    }
}

fn kind_of(n: usize) -> TrackKind {
    if n.is_multiple_of(2) {
        TrackKind::Video
    } else {
        TrackKind::Audio
    }
}

/// A clip of the project: its timeline, its track and itself.
struct Placed<'a> {
    timeline: &'a Timeline,
    track: &'a Track,
    clip: &'a Clip,
}

fn placed(project: &Project) -> Vec<Placed<'_>> {
    let mut all = vec![];
    for timeline in &project.timelines {
        for track in &timeline.tracks {
            for clip in &track.clips {
                all.push(Placed {
                    timeline,
                    track,
                    clip,
                });
            }
        }
    }
    all
}

/// Where a clip cut from `media` for a timeline of `rate` begins, and how long it lasts: from
/// the start of the media or a fifth or two of the way along, or ending `slack` ticks after the
/// end of the media; now and then counted in a scale that is not the media's.
fn source_of(project: &Project, media: &str, rate: Rate, spec: &Spec) -> (Time, i64) {
    let duration = project
        .media
        .iter()
        .find(|m| m.id == media)
        .map_or(Project::sample().media[0].facts.duration, |m| {
            m.facts.duration
        });
    let scale = match spec.b % 24 {
        0 => 90000,
        1 => 0,
        _ => duration.scale,
    };
    let ticks = if spec.near_end {
        duration.ticks - rate.ticks_of(spec.length, duration.scale) + spec.slack
    } else {
        i64::try_from(spec.a % 3).unwrap() * (duration.ticks / 5)
    };
    (Time { ticks, scale }, spec.length)
}

fn clip_of(project: &Project, rate: Rate, spec: &Spec, names: &[&str]) -> Clip {
    let ids: Vec<String> = project.media.iter().map(|m| m.id.clone()).collect();
    let media = pick(&ids, spec.b, "m99");
    let (source_in, length) = source_of(project, &media, rate, spec);
    Clip::of_media(id_of(spec, names), media, source_in, spec.start, length)
}

/// Media of the sample, or of another length or scale, with picture, sound or both.
fn media_of(spec: &Spec) -> Media {
    let mut media = Project::sample().media[0].clone();
    media.id = id_of(spec, &["m1", "m2", "m50", "x"]);
    match spec.a % 4 {
        0 => media.facts.video = None,
        1 => media.facts.audio = None,
        _ => {}
    }
    media.facts.duration = match spec.b % 3 {
        0 => Time {
            ticks: 10_000,
            scale: 1000,
        },
        // 7.3 seconds, in ticks that the frames of no rate here last a whole number of.
        1 => Time {
            ticks: 657_000,
            scale: 90000,
        },
        _ => media.facts.duration,
    };
    media
}

/// An operation made of `spec`, which names things of `project` most of the time.
pub fn op_of(project: &Project, spec: &Spec) -> Op {
    let timeline_ids: Vec<String> = project.timelines.iter().map(|t| t.id.clone()).collect();
    let clips = placed(project);
    let no_tracks = project.timelines.iter().all(|t| t.tracks.is_empty());
    let kind = match KINDS[spec.kind] {
        // A proxy, a bin, media and colours are set whatever else there is.
        kind @ (10 | 14..=17 | 28) => kind,
        // With no timeline left, most operations could only be refused.
        kind if kind >= 3 && project.timelines.is_empty() => 2,
        // With no track left, there is nowhere to put a clip.
        kind if no_tracks && (matches!(kind, 6..=9 | 12 | 13) || kind >= 18) => 4,
        // With no clip left, there is nothing to take away, move, trim, set or put next to.
        7 | 8 | 9 | 13 | 18 | 24 if clips.is_empty() => 6,
        kind => kind,
    };
    let timeline_id = pick(&timeline_ids, spec.a, "t99");
    let timeline = project.timelines.iter().find(|t| t.id == timeline_id);
    let rate = timeline.map_or(RATES[0], |t| t.rate);
    let track_ids: Vec<String> = timeline
        .map(|t| t.tracks.iter().map(|r| r.id.clone()).collect())
        .unwrap_or_default();
    // A clip, and now and then the id of none.
    let chosen = clips.get(spec.c % clips.len().max(1));
    let clip_id = match chosen {
        Some(placed) if !spec.b.is_multiple_of(16) => placed.clip.id.clone(),
        _ => "c99".into(),
    };
    let clip_timeline = chosen.map_or_else(|| timeline_id.clone(), |p| p.timeline.id.clone());
    match kind {
        0 => Op::AddMedia {
            media: media_of(spec),
            index: index_of(spec.c, project.media.len()),
        },
        1 => {
            let ids: Vec<String> = project.media.iter().map(|m| m.id.clone()).collect();
            Op::RemoveMedia {
                id: pick(&ids, spec.b, "m99"),
            }
        }
        2 => timeline_of(project, spec),
        3 => Op::RemoveTimeline { id: timeline_id },
        4 => Op::AddTrack {
            track: track_of(project, rate, spec),
            index: index_of(spec.c, track_ids.len()),
            timeline: timeline_id,
        },
        5 => Op::RemoveTrack {
            timeline: timeline_id,
            id: pick(&track_ids, spec.b, "V99"),
        },
        6 => insert_of(project, spec, timeline_id),
        7 => Op::RemoveClip {
            timeline: clip_timeline,
            id: clip_id,
        },
        10 => proxy_op_of(project, spec),
        8 => Op::MoveClip {
            timeline: clip_timeline,
            id: clip_id,
            track: move_target(chosen, spec),
            start: spec.start,
        },
        9 => trim_of(project, spec, chosen, clip_timeline, clip_id),
        11 => set_timeline_of(spec, timeline_id),
        12 => set_track_of(project, spec, timeline_id),
        13 => set_clip_of(project, spec, chosen, clip_timeline, clip_id),
        14..=17 => bin_or_media_of(project, spec, kind),
        18 => add_transition_of(project, spec, chosen, clip_timeline),
        19 | 20 => transition_op_of(project, spec, kind),
        21 => add_cue_of(project, spec),
        22 | 23 => cue_op_of(project, spec, kind),
        24 => insert_after_of(project, spec, chosen),
        26 => shift_of(spec, chosen, clip_timeline, clip_id),
        27 => grades::grade_op_of(project, [spec.a, spec.b, spec.c, spec.d]),
        28 => colors::color_op_of(project, [spec.a, spec.b, spec.c, spec.d]),
        29 => sounds::sound_op_of(project, [spec.a, spec.b, spec.c, spec.d]),
        _ => {
            let Op::InsertClip {
                timeline,
                track,
                mut clip,
            } = insert_of(project, spec, timeline_id)
            else {
                unreachable!()
            };
            clip.source = if spec.d.is_multiple_of(2) {
                title_of(spec)
            } else {
                colour_of(spec)
            };
            Op::InsertClip {
                timeline,
                track,
                clip,
            }
        }
    }
}

/// A clip trimmed at its start or moved too, from where it began or another place.
fn trim_of(
    project: &Project,
    spec: &Spec,
    chosen: Option<&Placed>,
    timeline: String,
    id: String,
) -> Op {
    let (media, rate, start) = chosen.map_or_else(
        || (Some("m1".to_string()), RATES[0], spec.start),
        |p| {
            let start = if spec.c.is_multiple_of(3) {
                spec.start
            } else {
                p.clip.start
            };
            (p.clip.media().cloned(), p.timeline.rate, start)
        },
    );
    // A title or a colour begins nowhere in media, but now and then is told to.
    let (source_in, length) = source_of(project, media.as_deref().unwrap_or("m1"), rate, spec);
    let source_in = (media.is_some() || spec.d.is_multiple_of(8)).then_some(source_in);
    Op::TrimClip {
        timeline,
        id,
        source_in,
        start,
        length,
    }
}

/// A title, of some text or now and then too much.
fn title_of(spec: &Spec) -> Source {
    let text = if spec.c % 16 == 1 {
        "x".repeat(1001)
    } else {
        format!("Title {}", spec.a % 10)
    };
    Source::Title {
        text,
        style: TitleStyle {
            font: String::new(),
            size: if spec.c % 16 == 2 { 0 } else { 96 },
            color: Rgba([255, 255, 255, 255]),
            x: 0.5,
            y: 0.875,
            align: TextAlign::Center,
        },
    }
}

fn colour_of(spec: &Spec) -> Source {
    let grey = u8::try_from(spec.a % 256).unwrap();
    Source::Color {
        color: Rgba([grey, grey, grey, 255]),
    }
}

/// Whether the settings bit `bit` of `spec` is set.
fn bit(spec: &Spec, bit: u32) -> bool {
    spec.d >> bit & 1 == 1
}

fn set_timeline_of(spec: &Spec, id: String) -> Op {
    Op::SetTimeline {
        id,
        name: bit(spec, 0).then(|| "Renamed".into()),
        rate: bit(spec, 1)
            .then(|| [RATES[0], RATES[1], RATES[2], Rate { num: 10, den: 1 }][spec.a % 4]),
        width: bit(spec, 2).then(|| [1280, 3840, 1919][spec.b % 3]),
        height: bit(spec, 3).then(|| [720, 2160, 8][spec.b % 3]),
        sample_rate: bit(spec, 4).then(|| [44100, 96000, 22050][spec.c % 3]),
    }
}

fn set_track_of(project: &Project, spec: &Spec, timeline_id: String) -> Op {
    let track_ids: Vec<String> = project
        .timelines
        .iter()
        .find(|t| t.id == timeline_id)
        .map(|t| t.tracks.iter().map(|r| r.id.clone()).collect())
        .unwrap_or_default();
    Op::SetTrack {
        color: None,
        id: pick(&track_ids, spec.b, "V99"),
        timeline: timeline_id,
        name: bit(spec, 0).then(|| "Track".into()),
        enabled: bit(spec, 1).then(|| bit(spec, 2)),
        locked: bit(spec, 3).then(|| bit(spec, 4)),
        // Only a track of subtitles has a language, and only one that is a tag.
        language: bit(spec, 5).then(|| ["zh", "en-GB", "", "english"][spec.c % 4].into()),
        // Only a track of sound is muted and soloed.
        mute: bit(spec, 6).then(|| bit(spec, 7)),
        solo: bit(spec, 8).then(|| bit(spec, 9)),
        solo_safe: bit(spec, 10).then(|| bit(spec, 2)),
    }
}

fn set_clip_of(
    project: &Project,
    spec: &Spec,
    chosen: Option<&Placed>,
    timeline: String,
    id: String,
) -> Op {
    let groups: Vec<String> = placed(project)
        .iter()
        .filter_map(|p| p.clip.link.clone())
        .collect();
    let link = match spec.c % 5 {
        0 => None,
        1 => Some(String::new()),
        2 => Some(pick(&groups, spec.a, "g1")),
        // One that the engine may make later.
        3 => Some("g90000".into()),
        _ => Some(chosen.and_then(|p| p.clip.link.clone()).unwrap_or_default()),
    };
    let transform = if spec.a.is_multiple_of(8) {
        Transform {
            scale: 0.0.into(),
            ..Transform::NONE
        }
    } else {
        columbina_model::Placed {
            x: 0.125,
            y: -0.25,
            scale: 1.5,
            rotation: 45.0,
            opacity: 0.75,
        }
        .into()
    };
    // A title or a colour is set anew; a clip of media is not given one, but now and then is
    // told to.
    let source = match chosen.map(|p| &p.clip.source) {
        Some(Source::Color { .. }) => Some(colour_of(spec)),
        Some(Source::Title { .. }) => Some(title_of(spec)),
        _ => (spec.d >> 12).is_multiple_of(8).then(|| title_of(spec)),
    };
    // Now and then a gain or a pan out of its range.
    let gain_db = if spec.a % 8 == 1 { 30.0 } else { -6.0 };
    let pan = if spec.b % 8 == 1 { 2.0 } else { -0.5 };
    Op::SetClip {
        timeline,
        id,
        enabled: bit(spec, 0).then(|| bit(spec, 1)),
        speed: bit(spec, 2).then(|| SPEEDS[spec.b % SPEEDS.len()]),
        reverse: bit(spec, 3).then(|| bit(spec, 4)),
        link: bit(spec, 5).then_some(link),
        transform: bit(spec, 6).then_some(transform),
        crop: None,
        composite: None,
        gain_db: bit(spec, 7).then(|| keyed(spec, gain_db)),
        pan: bit(spec, 8).then_some(pan.into()),
        fade_in: bit(spec, 9).then_some(spec.length % 12),
        fade_out: bit(spec, 10).then_some(spec.start % 12),
        audio_fade_in: None,
        audio_fade_out: None,
        eq: None,
        effects: None,
        pitch_correction: None,
        pitch: None,
        channels: None,
        source: source.filter(|_| bit(spec, 11)),
    }
}

/// An operation on a bin or on media: kind 14 adds a bin, 15 takes one away, 16 sets one, and
/// 17 sets media.
fn bin_or_media_of(project: &Project, spec: &Spec, kind: u8) -> Op {
    let bins: Vec<String> = project.bins.iter().map(|b| b.id.clone()).collect();
    let some_bin = |n: usize| match n % 8 {
        0 | 1 => None,
        2 => Some("b99".to_string()),
        _ => Some(pick(&bins, n / 8, "b99")),
    };
    match kind {
        14 => Op::AddBin {
            bin: Bin {
                id: id_of(spec, &["b1", "b4", "b60", "y"]),
                name: "Bin".into(),
                parent: some_bin(spec.b),
            },
            index: index_of(spec.c, project.bins.len()),
        },
        15 => Op::RemoveBin {
            id: pick(&bins, spec.b, "b99"),
        },
        16 => Op::SetBin {
            id: pick(&bins, spec.b, "b99"),
            name: bit(spec, 0).then(|| "Renamed".into()),
            parent: bit(spec, 1).then(|| some_bin(spec.c)),
        },
        _ => {
            let ids: Vec<String> = project.media.iter().map(|m| m.id.clone()).collect();
            let id = pick(&ids, spec.b, "m99");
            let duration = project
                .media
                .iter()
                .find(|m| m.id == id)
                .map_or(Project::sample().media[0].facts.duration, |m| {
                    m.facts.duration
                });
            // A mark a fifth or more of the way along, now and then past the end or in another
            // scale.
            let mark = |n: usize| match n % 8 {
                0 => None,
                1 => Some(Time {
                    ticks: duration.ticks + 1,
                    scale: duration.scale,
                }),
                2 => Some(Time {
                    ticks: 0,
                    scale: duration.scale + 1,
                }),
                n => Some(Time {
                    ticks: duration.ticks / 5 * i64::try_from(n - 3).unwrap(),
                    scale: duration.scale,
                }),
            };
            Op::SetMedia {
                still: None,
                id,
                name: bit(spec, 0).then(|| "Renamed".into()),
                bin: bit(spec, 1).then(|| some_bin(spec.c)),
                mark_in: bit(spec, 2).then(|| mark(spec.a)),
                mark_out: bit(spec, 3).then(|| mark(spec.a / 8 + 4)),
            }
        }
    }
}

/// A transition at the end of a clip, to the clip that starts there if there is one; now and
/// then of a kind that its track does not take.
fn add_transition_of(
    project: &Project,
    spec: &Spec,
    chosen: Option<&Placed>,
    timeline: String,
) -> Op {
    // Two clips that meet: the chosen one and the next, or now and then any two.
    let all = placed(project);
    let meeting: Vec<(&Placed, &Clip)> = chosen
        .into_iter()
        .chain(all.iter().filter(|_| !spec.c.is_multiple_of(4)))
        .filter_map(|p| {
            let right = p.track.clips.iter().find(|c| c.start == p.clip.end())?;
            Some((p, right))
        })
        // Most of the time at a cut that has no transition yet.
        .filter(|(p, _)| spec.d % 8 == 7 || p.track.transitions.iter().all(|x| x.left != p.clip.id))
        .collect();
    let at = if spec.b.is_multiple_of(2) {
        0
    } else {
        spec.b / 2 % meeting.len().max(1)
    };
    let (timeline, track, left, right, kind) = match meeting.get(at) {
        Some((p, right)) => {
            let picture = [
                TransitionKind::Dissolve,
                TransitionKind::DipBlack,
                TransitionKind::DipWhite,
            ][spec.a / 8 % 3];
            // One time in eight, of the kind of the other kind of track.
            let sound = (p.track.kind == TrackKind::Audio) != spec.a.is_multiple_of(8);
            let kind = if sound {
                TransitionKind::Crossfade
            } else {
                picture
            };
            (
                p.timeline.id.clone(),
                p.track.id.clone(),
                p.clip.id.clone(),
                right.id.clone(),
                kind,
            )
        }
        None => (
            timeline,
            "V1".into(),
            chosen.map_or_else(|| "c99".into(), |p| p.clip.id.clone()),
            "c98".into(),
            TransitionKind::Dissolve,
        ),
    };
    Op::AddTransition {
        timeline,
        track,
        transition: Transition {
            id: id_of(spec, &["x1", "x6", "x70", "c"]),
            kind,
            left,
            right,
            length: spec.length % 40,
            align: [Alignment::Center, Alignment::Start, Alignment::End][spec.b % 3],
            fade_in_level_db: LEVELS[spec.c % 3],
            fade_out_level_db: LEVELS[spec.d % 3],
        },
    }
}

/// Kind 19 takes a transition away, and 20 sets one.
fn transition_op_of(project: &Project, spec: &Spec, kind: u8) -> Op {
    let all: Vec<(String, String)> = project
        .timelines
        .iter()
        .flat_map(|t| {
            t.tracks
                .iter()
                .flat_map(|r| &r.transitions)
                .map(|x| (t.id.clone(), x.id.clone()))
        })
        .collect();
    let (timeline, id) = match all.get(spec.a % all.len().max(1)) {
        Some(found) if !spec.b.is_multiple_of(16) => found.clone(),
        _ => ("t1".into(), "x99".into()),
    };
    if kind == 19 {
        return Op::RemoveTransition { timeline, id };
    }
    Op::SetTransition {
        timeline,
        id,
        kind: bit(spec, 0).then(|| {
            [
                TransitionKind::Dissolve,
                TransitionKind::DipBlack,
                TransitionKind::DipWhite,
                TransitionKind::Crossfade,
            ][spec.c % 4]
        }),
        length: bit(spec, 1).then_some(spec.length % 60),
        align: bit(spec, 2)
            .then(|| [Alignment::Center, Alignment::Start, Alignment::End][spec.b % 3]),
        // Now and then a level on a transition of picture, which is refused.
        fade_in_level_db: bit(spec, 3).then(|| LEVELS[spec.c % 3]),
        fade_out_level_db: bit(spec, 4).then(|| LEVELS[spec.a % 3]),
    }
}

/// A subtitle, on a track of subtitles most of the time.
fn add_cue_of(project: &Project, spec: &Spec) -> Op {
    let tracks: Vec<(&Timeline, &Track)> = project
        .timelines
        .iter()
        .flat_map(|t| t.tracks.iter().map(move |r| (t, r)))
        .collect();
    let subtitles: Vec<&(&Timeline, &Track)> = tracks
        .iter()
        .filter(|(_, r)| r.kind == TrackKind::Subtitle)
        .collect();
    let any = tracks.get(spec.a % tracks.len().max(1));
    let found = match subtitles.get(spec.a % subtitles.len().max(1)) {
        Some(&found) if !spec.c.is_multiple_of(8) => Some(found),
        _ => any,
    };
    let (timeline, track) = found.map_or(("t99".into(), "S99".into()), |(t, r)| {
        (t.id.clone(), r.id.clone())
    });
    Op::AddCue {
        timeline,
        track,
        cue: Cue {
            id: id_of(spec, &["q1", "q3", "q80", "x"]),
            start: spec.start,
            length: spec.length % 60,
            text: if spec.b.is_multiple_of(10) {
                String::new()
            } else {
                "字幕".into()
            },
        },
    }
}

/// Kind 22 takes a subtitle away, and 23 sets one.
fn cue_op_of(project: &Project, spec: &Spec, kind: u8) -> Op {
    let all: Vec<(String, String)> = project
        .timelines
        .iter()
        .flat_map(|t| {
            t.tracks
                .iter()
                .flat_map(|r| &r.cues)
                .map(|q| (t.id.clone(), q.id.clone()))
        })
        .collect();
    let (timeline, id) = match all.get(spec.a % all.len().max(1)) {
        Some(found) if !spec.b.is_multiple_of(16) => found.clone(),
        _ => ("t1".into(), "q99".into()),
    };
    if kind == 22 {
        return Op::RemoveCue { timeline, id };
    }
    Op::SetCue {
        timeline,
        id,
        start: bit(spec, 0).then_some(spec.start),
        length: bit(spec, 1).then_some(spec.length % 60),
        text: bit(spec, 2).then(|| ["再见", ""][spec.c % 2].into()),
    }
}

/// A clip right after another, on its track, of a few frames: of the same media from a fifth or
/// two of the way along it, or now and then a title; so that transitions have clips that meet.
fn insert_after_of(project: &Project, spec: &Spec, chosen: Option<&Placed>) -> Op {
    let Some(p) = chosen else {
        return insert_of(project, spec, "t1".into());
    };
    let found = p
        .clip
        .media()
        .and_then(|id| project.media.iter().find(|m| m.id == *id));
    let source = match found {
        Some(media) if !spec.d.is_multiple_of(5) => {
            let duration = media.facts.duration;
            let fifths = i64::try_from(1 + spec.a % 2).unwrap();
            Source::Media {
                media: media.id.clone(),
                source_in: Time {
                    ticks: duration.ticks / 5 * fifths,
                    scale: duration.scale,
                },
            }
        }
        _ => title_of(spec),
    };
    let clip = Clip::new(
        id_of(spec, &["c1", "c60", "y"]),
        source,
        p.clip.end(),
        1 + spec.length % 20,
    );
    Op::InsertClip {
        timeline: p.timeline.id.clone(),
        track: p.track.id.clone(),
        clip,
    }
}

/// A proxy set on media, or taken away; now and then the streams of the sound of media set,
/// or taken away, as the engine sets them of a project written before they were (T3c).
fn proxy_op_of(project: &Project, spec: &Spec) -> Op {
    let ids: Vec<String> = project.media.iter().map(|m| m.id.clone()).collect();
    if spec.c % 8 == 7 {
        return Op::SetMediaStreams {
            id: pick(&ids, spec.b, "m99"),
            layout: bit(spec, 0).then(|| "stereo".into()),
            streams: bit(spec, 1).then(|| {
                vec![columbina_model::AudioStream {
                    index: u32::try_from(spec.a % 3).unwrap(),
                    channels: 2,
                    layout: bit(spec, 2).then(|| "stereo".into()),
                }]
            }),
        };
    }
    Op::SetProxy {
        media: pick(&ids, spec.b, "m99"),
        proxy: match spec.c % 4 {
            0 => None,
            // Now and then one that is refused.
            1 => Some(proxy_of("cache/m.mov", spec)),
            _ => Some(proxy_of("/cache/m.mov", spec)),
        },
    }
}

/// A proxy at `path`, of a size of `spec`.
fn proxy_of(path: &str, spec: &Spec) -> Proxy {
    Proxy {
        path: path.into(),
        width: u32::try_from(spec.a % 4000).unwrap_or(1) + 1,
        height: 1080,
        codec: "hevc".into(),
        digest_of_source: "0".repeat(64),
    }
}

/// A track to add to a timeline of `rate`: of picture or sound, empty or with a clip; or now and
/// then one of subtitles, shown or hidden. One added shown while another is shown is refused, as
/// a timeline shows one at a time.
fn track_of(project: &Project, rate: Rate, spec: &Spec) -> Track {
    if spec.b % 5 == 4 {
        return Track {
            enabled: bit(spec, 0),
            ..Track::new(id_of(spec, &["S1", "S2"]), TrackKind::Subtitle)
        };
    }
    let mut track = Track::new(id_of(spec, &["V1", "V2", "A1", "A4"]), kind_of(spec.b)).with_clips(
        if spec.b.is_multiple_of(3) {
            vec![]
        } else {
            vec![clip_of(project, rate, spec, &["c3", "c40", "y"])]
        },
    );
    // A track of sound muted, soloed, mono or at another level, now and then.
    if track.kind == TrackKind::Audio {
        let sound = &mut track.audio;
        (sound.mute, sound.solo) = (bit(spec, 1), bit(spec, 2));
        if bit(spec, 3) {
            (sound.format, sound.channels) = (columbina_model::TrackFormat::Mono, 1);
        }
        if bit(spec, 4) {
            sound.strip.fader_db = -6.0;
        }
    }
    track
}

/// A timeline to add, at one of the rates, with no tracks, one, or two; and now and then two of
/// subtitles.
fn timeline_of(project: &Project, spec: &Spec) -> Op {
    let rate = RATES[spec.a % RATES.len()];
    let mut tracks = match spec.b % 3 {
        0 => vec![],
        1 => vec![Track::new("", kind_of(spec.a)).with_clips(vec![clip_of(
            project,
            rate,
            spec,
            &["c1", "x1"],
        )])],
        _ => vec![
            Track::new("V1", TrackKind::Video).with_clips(vec![
                clip_of(project, rate, spec, &["c2", "x2"]),
                Clip {
                    id: String::new(),
                    start: spec.start + 200,
                    ..clip_of(project, rate, spec, &["x3"])
                },
            ]),
            Track::new("", kind_of(spec.b / 3)),
        ],
    };
    // Now and then two tracks of subtitles, each shown or hidden: both shown is refused.
    if bit(spec, 5) {
        for shown in [bit(spec, 6), bit(spec, 7)] {
            tracks.push(Track {
                enabled: shown,
                ..Track::new("", TrackKind::Subtitle)
            });
        }
    }
    Op::AddTimeline {
        timeline: Timeline {
            tracks,
            ..Timeline::new(id_of(spec, &["t1", "t2", "t70", "x"]), "Timeline", rate)
        },
        index: index_of(spec.c, project.timelines.len()),
    }
}

/// A clip to put onto a track of any timeline, and now and then onto one that is not there.
fn insert_of(project: &Project, spec: &Spec, timeline_id: String) -> Op {
    let tracks: Vec<(&Timeline, &Track)> = project
        .timelines
        .iter()
        .flat_map(|t| t.tracks.iter().map(move |r| (t, r)))
        .collect();
    let (timeline, track, rate) = match tracks.get(spec.a % tracks.len().max(1)) {
        Some((t, r)) if !spec.c.is_multiple_of(16) => (t.id.clone(), r.id.clone(), t.rate),
        _ => (timeline_id, "V99".into(), RATES[0]),
    };
    Op::InsertClip {
        timeline,
        track,
        clip: clip_of(project, rate, spec, &["c1", "c2", "c50", "y"]),
    }
}

/// A suffix of one track moved together; now and then a missing clip or invalid offset.
fn shift_of(spec: &Spec, chosen: Option<&Placed>, timeline: String, clip: String) -> Op {
    Op::ShiftClips {
        timeline,
        track: chosen.map_or_else(|| "V99".into(), |p| p.track.id.clone()),
        clips: chosen.map_or_else(
            || vec![clip],
            |p| {
                p.track
                    .clips
                    .iter()
                    .skip(spec.b % p.track.clips.len())
                    .map(|c| c.id.clone())
                    .collect()
            },
        ),
        by: spec.start,
    }
}

/// The track that a clip is moved to: most of the time another track of its kind if there is
/// one, else its own; now and then any track of its timeline, or one that is not there.
fn move_target(chosen: Option<&Placed>, spec: &Spec) -> String {
    match chosen {
        Some(placed) if !spec.a.is_multiple_of(8) => {
            let others: Vec<String> = placed
                .timeline
                .tracks
                .iter()
                .filter(|r| r.kind == placed.track.kind && r.id != placed.track.id)
                .map(|r| r.id.clone())
                .collect();
            if others.is_empty() || spec.a % 8 == 1 {
                placed.track.id.clone()
            } else {
                others[spec.a / 8 % others.len()].clone()
            }
        }
        Some(placed) => {
            let all: Vec<String> = placed
                .timeline
                .tracks
                .iter()
                .map(|r| r.id.clone())
                .collect();
            pick(&all, spec.b, "V99")
        }
        None => "V99".into(),
    }
}

/// The sample, with a second track of each kind for clips to move to; a track of subtitles with
/// q3 on it; bin b4; and c5 on V1 right after c1, cut from 4 seconds into m1, with a dissolve x6
/// between them.
pub fn tidy() -> Project {
    let mut project = Project::sample();
    for (id, kind) in [("V2", TrackKind::Video), ("A2", TrackKind::Audio)] {
        let op = Op::AddTrack {
            timeline: "t1".into(),
            track: Track::new(id, kind),
            index: None,
        };
        apply(&mut project, op).unwrap();
    }
    let mut subtitles = Track::new("", TrackKind::Subtitle);
    subtitles.language = "zh".into();
    subtitles.cues = vec![Cue {
        id: String::new(),
        start: 0,
        length: 50,
        text: "你好".into(),
    }];
    let c5 = Clip::of_media(
        "",
        "m1",
        Time {
            ticks: 4 * 15360,
            scale: 15360,
        },
        100,
        50,
    );
    let ops = [
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
            clip: c5,
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
    ];
    for op in ops {
        apply(&mut project, op).unwrap();
    }
    assert_eq!(project.timelines[0].tracks[0].transitions[0].id, "x6");
    // A bus beside B1 that feeds it, a VCA and a group of the tracks of sound, for the
    // operations on sound to name.
    for op in [
        serde_json::json!({"op": "add_bus", "timeline": "t1", "bus": {"id": "", "name": "Music",
            "color": "#40c080ff", "format": "stereo", "strip": {"outputs": [{"bus": "B1"}]}}}),
        serde_json::json!({"op": "add_vca", "timeline": "t1", "vca": {"id": "", "name": "All"}}),
        serde_json::json!({"op": "add_track_group", "timeline": "t1",
            "group": {"id": "", "name": "Sound", "members": ["A1", "A2"]}}),
    ] {
        apply(&mut project, serde_json::from_value(op).unwrap()).unwrap();
    }
    project
}

/// A project as it may be written elsewhere, which is put right as it is read: the clips of a
/// track out of order, `next_id` below the numbers of its ids, and a second timeline at
/// 30000/1001.
pub fn messy() -> Project {
    let mut value = serde_json::to_value(tidy()).unwrap();
    value["next_id"] = 1.into();
    let clips = value["timelines"][0]["tracks"][0]["clips"].clone();
    let first = clips[0].clone();
    let mut later = first.clone();
    later["id"] = "c8".into();
    later["start"] = 260.into();
    later["length"] = 40.into();
    let mut middle = first.clone();
    middle["id"] = "c9".into();
    middle["start"] = 160.into();
    middle["length"] = 50.into();
    middle["source"]["in"]["ticks"] = 30720.into();
    value["timelines"][0]["tracks"][0]["clips"] =
        serde_json::json!([later, clips[1], first, middle]);
    let mut other = value["timelines"][0].clone();
    other["id"] = "t16".into();
    other["rate"] = serde_json::json!({ "num": 30000, "den": 1001 });
    let mut number = 11;
    let mut fresh = || {
        number += 1;
        number
    };
    for track in other["tracks"].as_array_mut().unwrap() {
        let track = track.as_object_mut().unwrap();
        let mut renamed = std::collections::HashMap::new();
        for clip in track
            .get_mut("clips")
            .into_iter()
            .flat_map(|c| c.as_array_mut().unwrap())
        {
            let id = format!("c{}", fresh());
            renamed.insert(clip["id"].as_str().unwrap().to_string(), id.clone());
            clip["id"] = id.into();
        }
        for transition in track
            .get_mut("transitions")
            .into_iter()
            .flat_map(|x| x.as_array_mut().unwrap())
        {
            transition["id"] = format!("x{}", fresh()).into();
            for side in ["left", "right"] {
                transition[side] = renamed[transition[side].as_str().unwrap()].clone().into();
            }
        }
        for cue in track
            .get_mut("cues")
            .into_iter()
            .flat_map(|q| q.as_array_mut().unwrap())
        {
            cue["id"] = format!("q{}", fresh()).into();
        }
    }
    value["timelines"].as_array_mut().unwrap().push(other);
    let project = Project::from_value(value).unwrap();
    assert!(project.next_id > 1);
    project
}

/// Check that a project is still one that the operations could have made: `validate` would
/// neither refuse it nor change it. Since `validate` checks clips as the operations do, the
/// main invariants are checked here a second way too.
pub fn check(project: &Project) {
    let mut copy = project.clone();
    copy.validate()
        .unwrap_or_else(|e| panic!("{e}: {project:#?}"));
    assert_eq!(&copy, project);

    let mut ids = HashSet::new();
    let note = |id: &str| {
        assert_ne!(id, "");
        if let Some(number) = engine_number(id) {
            assert!(
                number < project.next_id,
                "{id} at next_id {}",
                project.next_id
            );
        }
    };
    for media in &project.media {
        note(&media.id);
    }
    for timeline in &project.timelines {
        note(&timeline.id);
        for track in &timeline.tracks {
            note(&track.id);
            for clip in &track.clips {
                note(&clip.id);
                assert!(ids.insert(clip.id.clone()), "clip {} twice", clip.id);
                if let Some(group) = &clip.link {
                    note(group);
                }
                let (Some(media), Some(source_in)) = (clip.media(), clip.source_in()) else {
                    assert_eq!(track.kind, TrackKind::Video, "{clip:?}");
                    continue;
                };
                let media = project.media.iter().find(|m| m.id == *media).unwrap();
                let has = match track.kind {
                    TrackKind::Video => media.facts.video.is_some(),
                    TrackKind::Audio => media.facts.audio.is_some(),
                    TrackKind::Subtitle => false,
                };
                assert!(has, "{clip:?} on {}", track.id);
                let duration = media.facts.duration;
                assert_eq!(source_in.scale, duration.scale);
                assert!(clip.length > 0 && clip.start >= 0 && source_in.ticks >= 0);
                assert!(clip.start + clip.length <= LARGEST);
                let ticks = source_span(clip, timeline.rate, duration.scale);
                for k in [0, clip.length - 1] {
                    let at = source_tick(clip, k, timeline.rate, duration.scale);
                    assert!((0..duration.ticks).contains(&at), "{clip:?} reads at {at}");
                }
                assert!(
                    i128::from(source_in.ticks) + i128::from(ticks) <= i128::from(duration.ticks),
                    "{clip:?} past the end of {}",
                    media.id
                );
            }
            for pair in track.clips.windows(2) {
                assert!(
                    pair[0].start + pair[0].length <= pair[1].start,
                    "out of order or overlapping: {pair:?}"
                );
            }
            for transition in &track.transitions {
                note(&transition.id);
                let end_of = |id: &str| track.clips.iter().find(|c| c.id == id).map(Clip::end);
                let start_of = |id: &str| track.clips.iter().find(|c| c.id == id).map(|c| c.start);
                assert_eq!(end_of(&transition.left), start_of(&transition.right));
            }
            for cue in &track.cues {
                note(&cue.id);
            }
        }
    }
    for bin in &project.bins {
        note(&bin.id);
    }
    for group in &project.groups {
        note(&group.id);
    }
    for target in grades::targets(project) {
        let grade = grades::grade_of(project, &target).unwrap();
        let mut nodes = HashSet::new();
        for node in &grade.nodes {
            note(&node.id);
            assert!(
                nodes.insert(node.id.as_str()),
                "node {} twice in {target:?}",
                node.id
            );
        }
        for pair in grade.links.windows(2) {
            assert!(
                pair[0].target() < pair[1].target(),
                "links out of order: {pair:?}"
            );
        }
        assert!(
            goes_forward(grade),
            "the links of {target:?} go round: {grade:?}"
        );
    }
}

/// Whether the links of a grade go round in no loop: the nodes can be taken one by one, each
/// once nothing that goes into it is left.
fn goes_forward(grade: &columbina_model::Grade) -> bool {
    let mut left: Vec<&columbina_model::Link> = grade
        .links
        .iter()
        .filter(|l| l.from != "source" && l.to != "output")
        .collect();
    let mut nodes: Vec<&str> = grade.nodes.iter().map(|n| n.id.as_str()).collect();
    while let Some(at) = nodes
        .iter()
        .position(|node| !left.iter().any(|l| l.to == *node))
    {
        let node = nodes.remove(at);
        left.retain(|l| l.from != node);
    }
    nodes.is_empty()
}

/// The number of an id of the shape of those that the engine makes.
fn engine_number(id: &str) -> Option<u64> {
    let digits = id.strip_prefix(['b', 'm', 't', 'c', 'x', 'q', 'g', 'n', 'k'])?;
    let plain = !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit())
        && (digits == "0" || !digits.starts_with('0'));
    let number: u64 = digits.parse().ok().filter(|_| plain)?;
    (number < LARGEST.unsigned_abs()).then_some(number)
}

/// A gain to set: now and then keyframes, from it to 0 dB over a second of the source, in order
/// or (to be refused) not.
fn keyed(spec: &Spec, gain_db: f64) -> columbina_model::AudioParam {
    use columbina_model::{AudioKey, AudioParam, KeyInterp};
    let key = |seconds: i64, value| AudioKey {
        at: Time {
            ticks: seconds * 48000,
            scale: 48000,
        },
        value,
        interp: KeyInterp::EaseOut,
    };
    match spec.d % 6 {
        0 => AudioParam::Keys(vec![key(1, gain_db), key(2, 0.0)]),
        1 => AudioParam::Keys(vec![key(2, gain_db), key(1, 0.0)]),
        _ => gain_db.into(),
    }
}
