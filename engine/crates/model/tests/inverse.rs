//! Random operations on a project: each one that is applied is taken back by its inverse and
//! done again by the inverse of that, and each one that is refused changes nothing. After every
//! operation the project is still one that `Project::validate` leaves as it is.

use std::collections::HashSet;

use columbina_model::{
    apply, Clip, Media, Op, Project, Rate, Time, Timeline, Track, TrackKind, LARGEST,
};
use proptest::prelude::*;

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
/// put in, moved and trimmed most often, and timelines are taken away least.
const KINDS: [u8; 24] = [
    0, 0, 1, 2, 3, 4, 4, 5, 6, 6, 6, 6, 6, 6, 7, 7, 8, 8, 8, 8, 9, 9, 9, 9,
];

/// The makings of an operation, which are turned into one against the project as it is then.
#[derive(Clone, Debug)]
struct Spec {
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
}

fn spec() -> impl Strategy<Value = Spec> {
    (
        (
            0..KINDS.len(),
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
            |((kind, a, b, c), start, length, slack, near_end, named)| Spec {
                kind,
                a,
                b,
                c,
                start,
                length,
                slack,
                near_end,
                named,
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
    Clip {
        id: id_of(spec, names),
        media,
        source_in,
        start: spec.start,
        length,
    }
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
fn op_of(project: &Project, spec: &Spec) -> Op {
    let timeline_ids: Vec<String> = project.timelines.iter().map(|t| t.id.clone()).collect();
    let clips = placed(project);
    let no_tracks = project.timelines.iter().all(|t| t.tracks.is_empty());
    let kind = match KINDS[spec.kind] {
        // With no timeline left, most operations could only be refused.
        kind if kind >= 3 && project.timelines.is_empty() => 2,
        // With no track left, there is nowhere to put a clip.
        kind if kind >= 6 && no_tracks => 4,
        // With no clip left, there is nothing to take away, move or trim.
        kind if kind >= 7 && clips.is_empty() => 6,
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
            track: Track {
                id: id_of(spec, &["V1", "V2", "A1", "A4"]),
                kind: kind_of(spec.b),
                clips: if spec.b.is_multiple_of(3) {
                    vec![]
                } else {
                    vec![clip_of(project, rate, spec, &["c3", "c40", "y"])]
                },
            },
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
        8 => Op::MoveClip {
            timeline: clip_timeline,
            id: clip_id,
            track: move_target(chosen, spec),
            start: spec.start,
        },
        _ => {
            let (media, rate, start) = chosen.map_or_else(
                || ("m1".to_string(), RATES[0], spec.start),
                |p| {
                    let start = if spec.c.is_multiple_of(3) {
                        spec.start
                    } else {
                        p.clip.start
                    };
                    (p.clip.media.clone(), p.timeline.rate, start)
                },
            );
            let (source_in, length) = source_of(project, &media, rate, spec);
            Op::TrimClip {
                timeline: clip_timeline,
                id: clip_id,
                source_in,
                start,
                length,
            }
        }
    }
}

/// A timeline to add, at one of the rates, with no tracks, one, or two.
fn timeline_of(project: &Project, spec: &Spec) -> Op {
    let rate = RATES[spec.a % RATES.len()];
    let tracks = match spec.b % 3 {
        0 => vec![],
        1 => vec![Track {
            id: String::new(),
            kind: kind_of(spec.a),
            clips: vec![clip_of(project, rate, spec, &["c1", "x1"])],
        }],
        _ => vec![
            Track {
                id: "V1".into(),
                kind: TrackKind::Video,
                clips: vec![
                    clip_of(project, rate, spec, &["c2", "x2"]),
                    Clip {
                        id: String::new(),
                        start: spec.start + 200,
                        ..clip_of(project, rate, spec, &["x3"])
                    },
                ],
            },
            Track {
                id: String::new(),
                kind: kind_of(spec.b / 3),
                clips: vec![],
            },
        ],
    };
    Op::AddTimeline {
        timeline: Timeline {
            id: id_of(spec, &["t1", "t2", "t70", "x"]),
            name: "Timeline".into(),
            rate,
            tracks,
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

/// The sample, with a second track of each kind for clips to move to.
fn tidy() -> Project {
    let mut project = Project::sample();
    for (id, kind) in [("V2", TrackKind::Video), ("A2", TrackKind::Audio)] {
        let op = Op::AddTrack {
            timeline: "t1".into(),
            track: Track {
                id: id.into(),
                kind,
                clips: vec![],
            },
            index: None,
        };
        apply(&mut project, op).unwrap();
    }
    project
}

/// A project as it may be written elsewhere, which is put right as it is read: the clips of a
/// track out of order, `next_id` below the numbers of its ids, and a second timeline at
/// 30000/1001.
fn messy() -> Project {
    let mut value = serde_json::to_value(tidy()).unwrap();
    value["next_id"] = 1.into();
    let first = value["timelines"][0]["tracks"][0]["clips"][0].clone();
    let mut later = first.clone();
    later["id"] = "c8".into();
    later["start"] = 260.into();
    later["length"] = 40.into();
    let mut middle = first.clone();
    middle["id"] = "c5".into();
    middle["start"] = 130.into();
    middle["length"] = 50.into();
    middle["source_in"]["ticks"] = 30720.into();
    value["timelines"][0]["tracks"][0]["clips"] = serde_json::json!([later, first, middle]);
    let mut other = value["timelines"][0].clone();
    other["id"] = "t6".into();
    other["rate"] = serde_json::json!({ "num": 30000, "den": 1001 });
    let mut number = 11;
    for track in other["tracks"].as_array_mut().unwrap() {
        for clip in track["clips"].as_array_mut().unwrap() {
            clip["id"] = format!("c{number}").into();
            number += 1;
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
fn check(project: &Project) {
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
                let media = project.media.iter().find(|m| m.id == clip.media).unwrap();
                let has = match track.kind {
                    TrackKind::Video => media.facts.video.is_some(),
                    TrackKind::Audio => media.facts.audio.is_some(),
                };
                assert!(has, "{clip:?} on {}", track.id);
                let duration = media.facts.duration;
                assert_eq!(clip.source_in.scale, duration.scale);
                assert!(clip.length > 0 && clip.start >= 0 && clip.source_in.ticks >= 0);
                assert!(clip.start + clip.length <= LARGEST);
                let rate = timeline.rate;
                let ticks =
                    i128::from(clip.length) * i128::from(rate.den) * i128::from(duration.scale)
                        / i128::from(rate.num);
                assert!(
                    i128::from(clip.source_in.ticks) + ticks <= i128::from(duration.ticks),
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
        }
    }
}

/// The number of an id of the shape of those that the engine makes.
fn engine_number(id: &str) -> Option<u64> {
    let digits = id.strip_prefix(['m', 't', 'c'])?;
    let plain = !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit())
        && (digits == "0" || !digits.starts_with('0'));
    let number: u64 = digits.parse().ok().filter(|_| plain)?;
    (number < LARGEST.unsigned_abs()).then_some(number)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    #[test]
    fn every_operation_is_taken_back_or_changes_nothing(
        start_messy in any::<bool>(),
        specs in prop::collection::vec(spec(), 1..30)
    ) {
        let mut project = if start_messy { messy() } else { tidy() };
        check(&project);
        for spec in &specs {
            let op = op_of(&project, spec);
            let before = project.clone();
            let Ok(inverse) = apply(&mut project, op.clone()) else {
                // A refused operation changes nothing, not even next_id.
                prop_assert_eq!(&project, &before, "{:?}", op);
                continue;
            };
            check(&project);
            prop_assert!(project.next_id >= before.next_id);
            let after = project.clone();

            let mut undone = after.clone();
            let again = apply(&mut undone, inverse.clone())
                .unwrap_or_else(|e| panic!("{inverse:?} of {op:?}: {e}"));
            prop_assert_eq!(undone.next_id, after.next_id);
            let mut expected = before.clone();
            expected.next_id = after.next_id;
            prop_assert_eq!(&undone, &expected, "{:?} then {:?}", op, inverse);

            // Doing it again with the inverse of the inverse makes the same things, with the
            // same ids.
            apply(&mut undone, again.clone())
                .unwrap_or_else(|e| panic!("{again:?} of {op:?}: {e}"));
            prop_assert_eq!(&undone, &after, "{:?} redone as {:?}", op, again);
        }
    }
}
