//! Random operations on the sample project: each one that is applied is taken back by its
//! inverse, and each one that is refused changes nothing.

use std::collections::HashSet;

use columbina_model::{
    apply, Clip, Media, Op, Project, Rate, Time, Timeline, Track, TrackKind, LARGEST,
};
use proptest::prelude::*;

/// The scale of the media of the sample.
const SCALE: u32 = 15360;

/// The makings of an operation, which are turned into one against the project as it is then.
#[derive(Clone, Debug)]
struct Spec {
    kind: usize,
    a: usize,
    b: usize,
    c: usize,
    start: i64,
    length: i64,
    named: bool,
}

fn spec() -> impl Strategy<Value = Spec> {
    (
        0..KINDS.len(),
        any::<usize>(),
        any::<usize>(),
        any::<usize>(),
        // Now and then a start whose end JavaScript could not read.
        prop_oneof![19 => -5i64..400, 1 => Just(LARGEST - 10)],
        -2i64..120,
        any::<bool>(),
    )
        .prop_map(|(kind, a, b, c, start, length, named)| Spec {
            kind,
            a,
            b,
            c,
            start,
            length,
            named,
        })
}

/// The kinds of operation, as `op_of` numbers them, each as often as it is wanted: clips are
/// put in most often, and timelines are taken away least, since little is left to do after.
const KINDS: [u8; 20] = [0, 0, 1, 2, 2, 3, 4, 4, 5, 6, 6, 6, 6, 6, 7, 7, 8, 8, 9, 9];

/// One of `ids`, or one time in eight `bogus`, which names nothing.
fn pick(ids: &[String], n: usize, bogus: &str) -> String {
    if n.is_multiple_of(8) || ids.is_empty() {
        bogus.into()
    } else {
        ids[n / 8 % ids.len()].clone()
    }
}

/// An id that the engine is to give, or one of `names`, some of which are held.
fn id_of(spec: &Spec, names: &[&str]) -> String {
    if spec.named {
        names[spec.c % names.len()].into()
    } else {
        String::new()
    }
}

/// An index into a list of `len`, or none; one time in eight past its end.
fn index_of(n: usize, len: usize) -> Option<usize> {
    match n % 8 {
        0 | 2 => None,
        1 => Some(len + 1),
        _ => Some(n / 8 % (len + 1)),
    }
}

fn media_ids(project: &Project) -> Vec<String> {
    project.media.iter().map(|m| m.id.clone()).collect()
}

fn timeline_ids(project: &Project) -> Vec<String> {
    project.timelines.iter().map(|t| t.id.clone()).collect()
}

fn timeline_named<'a>(project: &'a Project, id: &str) -> Option<&'a Timeline> {
    project.timelines.iter().find(|t| t.id == id)
}

fn track_ids(timeline: Option<&Timeline>) -> Vec<String> {
    timeline
        .map(|t| t.tracks.iter().map(|r| r.id.clone()).collect())
        .unwrap_or_default()
}

fn clip_ids(timeline: Option<&Timeline>) -> Vec<String> {
    timeline
        .map(|t| {
            t.tracks
                .iter()
                .flat_map(|r| &r.clips)
                .map(|c| c.id.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// A time in the media, mostly of its scale, now and then of another or of none.
fn source_in(n: usize) -> Time {
    let scale = match n % 10 {
        0 => 0,
        1 => 90000,
        _ => SCALE,
    };
    let ticks = if n % 10 == 2 {
        -100
    } else {
        i64::try_from(n % 5).unwrap() * 3072
    };
    Time { ticks, scale }
}

fn clip_of(project: &Project, spec: &Spec, names: &[&str]) -> Clip {
    Clip {
        id: id_of(spec, names),
        media: pick(&media_ids(project), spec.b, "m9"),
        source_in: source_in(spec.a / 3),
        start: spec.start,
        length: spec.length,
    }
}

fn media_of(spec: &Spec) -> Media {
    let mut media = Project::sample().media[0].clone();
    media.id = id_of(spec, &["m1", "m2", "m5"]);
    match spec.a % 3 {
        0 => media.facts.video = None,
        1 => media.facts.audio = None,
        _ => {}
    }
    if spec.b.is_multiple_of(3) {
        media.facts.duration = Time {
            ticks: 10_000,
            scale: 1000,
        };
    }
    media
}

fn kind_of(n: usize) -> TrackKind {
    if n.is_multiple_of(2) {
        TrackKind::Video
    } else {
        TrackKind::Audio
    }
}

/// An operation made of `spec`, which names things of `project` most of the time.
fn op_of(project: &Project, spec: &Spec) -> Op {
    let timeline = pick(&timeline_ids(project), spec.a, "t9");
    let named = timeline_named(project, &timeline);
    // With no timeline left, most operations could only be refused: a timeline is added instead.
    let kind = match KINDS[spec.kind] {
        kind if kind >= 3 && project.timelines.is_empty() => 2,
        kind => kind,
    };
    match kind {
        0 => Op::AddMedia {
            media: media_of(spec),
            index: index_of(spec.c, project.media.len()),
        },
        1 => Op::RemoveMedia {
            id: pick(&media_ids(project), spec.b, "m9"),
        },
        2 => {
            let tracks = match spec.b % 3 {
                0 => vec![],
                1 => vec![Track {
                    id: String::new(),
                    kind: kind_of(spec.a),
                    clips: vec![clip_of(project, spec, &["c1", "x1"])],
                }],
                _ => vec![
                    Track {
                        id: "V1".into(),
                        kind: TrackKind::Video,
                        clips: vec![
                            clip_of(project, spec, &["c2", "x2"]),
                            Clip {
                                id: String::new(),
                                start: spec.start + 50,
                                ..clip_of(project, spec, &["c1"])
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
                    id: id_of(spec, &["t1", "t2", "t7"]),
                    name: "Timeline".into(),
                    rate: Rate { num: 25, den: 1 },
                    tracks,
                },
                index: index_of(spec.c, project.timelines.len()),
            }
        }
        3 => Op::RemoveTimeline { id: timeline },
        4 => Op::AddTrack {
            timeline,
            track: Track {
                id: id_of(spec, &["V1", "V2", "A1", "A4"]),
                kind: kind_of(spec.b),
                clips: if spec.b.is_multiple_of(3) {
                    vec![]
                } else {
                    vec![clip_of(project, spec, &["c3", "c40"])]
                },
            },
            index: index_of(spec.c, track_ids(named).len()),
        },
        5 => Op::RemoveTrack {
            timeline,
            id: pick(&track_ids(named), spec.b, "V9"),
        },
        6 => Op::InsertClip {
            timeline,
            track: pick(&track_ids(named), spec.b, "V9"),
            clip: clip_of(project, spec, &["c1", "c2", "c50", "y"]),
        },
        7 => Op::RemoveClip {
            timeline,
            id: pick(&clip_ids(named), spec.b, "c99"),
        },
        8 => Op::MoveClip {
            timeline,
            id: pick(&clip_ids(named), spec.b, "c99"),
            track: pick(&track_ids(named), spec.c, "V9"),
            start: spec.start,
        },
        _ => Op::TrimClip {
            timeline,
            id: pick(&clip_ids(named), spec.b, "c99"),
            source_in: source_in(spec.c),
            start: spec.start,
            length: spec.length,
        },
    }
}

/// Check what must hold of a project after every operation that is applied.
fn check(project: &Project) {
    let mut media = HashSet::new();
    for m in &project.media {
        assert_ne!(m.id, "");
        assert!(media.insert(&m.id), "media {} twice", m.id);
    }
    let mut timelines = HashSet::new();
    let mut clips = HashSet::new();
    for timeline in &project.timelines {
        assert_ne!(timeline.id, "");
        assert!(timelines.insert(&timeline.id), "timeline {}", timeline.id);
        let mut tracks = HashSet::new();
        for track in &timeline.tracks {
            assert_ne!(track.id, "");
            assert!(tracks.insert(&track.id), "track {}", track.id);
            for clip in &track.clips {
                assert_ne!(clip.id, "");
                assert!(clips.insert(&clip.id), "clip {}", clip.id);
                assert!(media.contains(&clip.media), "clip {} of no media", clip.id);
                assert!(clip.length > 0 && clip.start >= 0, "{clip:?}");
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

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn every_operation_is_taken_back_or_changes_nothing(
        specs in prop::collection::vec(spec(), 1..30)
    ) {
        let mut project = Project::sample();
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

            // Doing it again with the inverse of the inverse makes the same things,
            // with the same ids.
            apply(&mut undone, again.clone())
                .unwrap_or_else(|e| panic!("{again:?} of {op:?}: {e}"));
            prop_assert_eq!(&undone, &after, "{:?} redone as {:?}", op, again);
        }
    }
}
