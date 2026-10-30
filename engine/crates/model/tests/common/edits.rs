//! Random timelines and random edits of them.
//!
//! A timeline is made of a recipe: clips of picture and sound, linked or alone, titles and
//! colours, laid one after another on three tracks of picture and two of sound with gaps or
//! none between them, at speeds other than 1 and backward now and then; transitions at some cuts,
//! fades, subtitles on two tracks, and some tracks locked. A recipe is turned into a project by
//! operations, and what an operation refuses is left out.
//!
//! An edit is made of numbers, against the project as it is then: it names clips, cuts, frames
//! and tracks of the project most of the time, and now and then ones that are not there.

use columbina_model::{
    apply, apply_done, Alignment, AudioFacts, Clip, CueItem, Edge, Edit, Facts, Identity, Material,
    Media, Op, Project, Rate, Rgba, Source, Speed, Straddling, TextAlign, Time, Timeline,
    TitleStyle, Track, TrackKind, Transition, TransitionKind, VideoFacts,
};
use proptest::prelude::*;

pub const RATES: [Rate; 3] = [
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

pub const SPEEDS: [Speed; 6] = [
    Speed::ONE,
    Speed::ONE,
    Speed { num: 2, den: 1 },
    Speed { num: 1, den: 2 },
    Speed { num: 3, den: 2 },
    Speed { num: 2, den: 3 },
];

const TRACKS: [&str; 7] = ["V1", "V2", "V3", "A1", "A2", "S1", "S2"];

/// The media of the timelines: picture and sound in ticks of 15360, of 90000 for 7.3 seconds,
/// sound only in ticks of 48000, and picture only in ticks of 1000.
fn media() -> Vec<Media> {
    let make = |id: &str, ticks: i64, scale: u32, video: bool, audio: bool| Media {
        id: id.into(),
        path: format!("C:/media/{id}.mp4"),
        identity: Identity {
            size: 1000,
            modified_us: 0,
            digest: "0".repeat(64),
        },
        facts: Facts {
            duration: Time { ticks, scale },
            video: video.then(|| VideoFacts {
                still: false,
                width: 1920,
                height: 1080,
                rate: Rate { num: 25, den: 1 },
                codec: "h264".into(),
                bits: 8,
                rotation: 0,
            }),
            audio: audio.then(|| AudioFacts {
                rate: 48000,
                channels: 2,
                codec: "aac".into(),
            }),
        },
        proxy: None,
        name: format!("{id}.mp4"),
        bin: None,
        mark_in: None,
        mark_out: None,
    };
    vec![
        make("m1", 20 * 15360, 15360, true, true),
        make("m2", 657_000, 90000, true, true),
        make("m3", 10 * 48000, 48000, false, true),
        make("m4", 12_000, 1000, true, false),
    ]
}

/// A piece of a timeline: what it is, where it goes and how it plays.
#[derive(Clone, Debug)]
pub struct Piece {
    /// 0 to 3 picture and sound linked, 4 picture, 5 sound, 6 a title, 7 a colour.
    kind: u8,
    video: usize,
    audio: usize,
    media: usize,
    gap: i64,
    length: i64,
    /// How far into its media it begins, in hundredths of what is left.
    into: i64,
    speed: usize,
    reverse: bool,
    fades: (i64, i64),
    /// For picture and sound linked, how many frames longer the sound is than the picture.
    split: i64,
    /// Whether each clip of the piece starts at the end of its own track, rather than all at the
    /// end of the one that goes furthest: after a split, a picture that starts under the sound
    /// of the piece before it.
    stagger: bool,
}

/// A recipe of a timeline.
#[derive(Clone, Debug)]
pub struct Recipe {
    rate: usize,
    pieces: Vec<Piece>,
    /// For each cut in order, a transition of this length and alignment, or none if 0.
    transitions: Vec<(i64, u8)>,
    cues: Vec<(i64, i64, usize)>,
    locked: Vec<bool>,
    /// The track of subtitles that is shown: none, S1 or S2. A timeline shows one at a time.
    shown: usize,
}

fn piece() -> impl Strategy<Value = Piece> {
    (
        (
            prop_oneof![2 => 0u8..4, 1 => 4u8..8],
            prop_oneof![3 => Just(0usize), 1 => 1usize..3],
            prop_oneof![3 => Just(0usize), 1 => Just(1usize)],
            0usize..4,
        ),
        prop_oneof![3 => Just(0i64), 2 => 1i64..40],
        1i64..90,
        0i64..100,
        (0usize..SPEEDS.len(), prop::bool::weighted(0.2)),
        prop_oneof![3 => Just((0i64, 0i64)), 1 => (0i64..10, 0i64..10)],
        (
            prop_oneof![3 => Just(0i64), 1 => -30i64..40],
            prop::bool::weighted(0.3),
        ),
    )
        .prop_map(
            |(
                (kind, video, audio, media),
                gap,
                length,
                into,
                (speed, reverse),
                fades,
                (split, stagger),
            )| Piece {
                kind,
                video,
                audio,
                media,
                gap,
                length,
                into,
                speed,
                reverse,
                fades,
                split,
                stagger,
            },
        )
}

pub fn recipe() -> impl Strategy<Value = Recipe> {
    (
        0usize..RATES.len(),
        prop::collection::vec(piece(), 0..24),
        prop::collection::vec(
            prop_oneof![1 => Just((0i64, 0u8)), 1 => (1i64..24, 0u8..3)],
            0..16,
        ),
        prop::collection::vec((0i64..60, 1i64..40, 0usize..2), 0..8),
        prop::collection::vec(prop::bool::weighted(0.12), TRACKS.len()),
        0usize..3,
    )
        .prop_map(|(rate, pieces, transitions, cues, locked, shown)| Recipe {
            rate,
            pieces,
            transitions,
            cues,
            locked,
            shown,
        })
}

/// The project of a recipe: timeline t1 with its tracks, and what of the recipe the operations
/// take.
#[allow(clippy::too_many_lines, reason = "a step for each part of the recipe")]
pub fn project_of(recipe: &Recipe) -> Project {
    let mut timeline = Timeline::new("t1", "Timeline 1", RATES[recipe.rate]);
    timeline.tracks = TRACKS
        .iter()
        .map(|id| {
            let kind = match id.as_bytes()[0] {
                b'V' => TrackKind::Video,
                b'A' => TrackKind::Audio,
                _ => TrackKind::Subtitle,
            };
            let mut track = Track::new(*id, kind);
            if kind == TrackKind::Subtitle {
                track.enabled = ["", "S1", "S2"][recipe.shown] == *id;
            }
            track
        })
        .collect();
    let mut project = Project {
        media: media(),
        timelines: vec![timeline],
        ..Project::new()
    };
    project.next_id = 10;
    let rate = RATES[recipe.rate];
    // Where each track is up to.
    let mut ends = [0i64; 7];
    for piece in &recipe.pieces {
        let v = piece.video;
        let a = 3 + piece.audio;
        let (on, source) = match piece.kind {
            0..=3 => (
                vec![v, a],
                media_source(&project, piece, rate, [0, 1][piece.media % 2]),
            ),
            4 => (
                vec![v],
                media_source(&project, piece, rate, [0, 1, 3][piece.media % 3]),
            ),
            5 => (
                vec![a],
                media_source(&project, piece, rate, [0, 1, 2][piece.media % 3]),
            ),
            6 => (vec![v], title()),
            _ => (
                vec![v],
                Source::Color {
                    color: Rgba([10, 20, 30, 255]),
                },
            ),
        };
        let start = on.iter().map(|&r| ends[r]).max().unwrap_or(0) + piece.gap;
        let mut group = (on.len() > 1).then(String::new);
        for &r in &on {
            let start = if piece.stagger {
                ends[r] + piece.gap
            } else {
                start
            };
            let length = if r >= 3 && on.len() > 1 {
                (piece.length + piece.split).max(1)
            } else {
                piece.length
            };
            let mut clip = Clip::new("", source.clone(), start, length);
            clip.speed = SPEEDS[piece.speed];
            clip.reverse = piece.reverse;
            clip.fade_in = piece.fades.0;
            clip.fade_out = piece.fades.1;
            clip.link.clone_from(&group);
            let op = Op::InsertClip {
                timeline: "t1".into(),
                track: TRACKS[r].into(),
                clip,
            };
            if let Ok((Op::InsertClip { clip, .. }, _)) = apply_done(&mut project, op) {
                ends[r] = clip.end();
                group = clip.link;
            }
        }
    }
    // Transitions at the cuts, track by track, as far as the recipe goes.
    let mut wanted = recipe.transitions.iter();
    let cuts: Vec<(String, String, String, TrackKind)> = project.timelines[0]
        .tracks
        .iter()
        .flat_map(|r| {
            r.clips
                .windows(2)
                .filter(|p| p[0].end() == p[1].start)
                .map(|p| (r.id.clone(), p[0].id.clone(), p[1].id.clone(), r.kind))
        })
        .collect();
    for (track, left, right, kind) in cuts {
        let Some(&(length, align)) = wanted.next() else {
            break;
        };
        if length == 0 {
            continue;
        }
        let kind = if kind == TrackKind::Audio {
            TransitionKind::Crossfade
        } else {
            [TransitionKind::Dissolve, TransitionKind::DipBlack][usize::from(align) % 2]
        };
        let transition = Transition {
            id: String::new(),
            kind,
            left,
            right,
            length,
            align: [Alignment::Center, Alignment::Start, Alignment::End][usize::from(align)],
        };
        // A fade at the cut is taken off first, as an editor would.
        for (id, edge) in [(&transition.left, 1), (&transition.right, 0)] {
            let clip = project.timelines[0]
                .tracks
                .iter()
                .flat_map(|r| &r.clips)
                .find(|c| c.id == *id)
                .unwrap();
            let (fade_in, fade_out) = if edge == 0 {
                (0, clip.fade_out)
            } else {
                (clip.fade_in, 0)
            };
            let _ = apply(&mut project, fades(id, fade_in, fade_out));
        }
        let _ = apply(
            &mut project,
            Op::AddTransition {
                timeline: "t1".into(),
                track,
                transition,
            },
        );
    }
    let mut cue_ends = [0i64; 2];
    for (n, &(gap, length, track)) in recipe.cues.iter().enumerate() {
        let start = cue_ends[track] + gap;
        let op = Op::AddCue {
            timeline: "t1".into(),
            track: TRACKS[5 + track].into(),
            cue: columbina_model::Cue {
                id: String::new(),
                start,
                length,
                text: format!("line {n}"),
            },
        };
        apply(&mut project, op).unwrap();
        cue_ends[track] = start + length;
    }
    for (at, &locked) in recipe.locked.iter().enumerate() {
        if locked {
            let op = Op::SetTrack {
                timeline: "t1".into(),
                id: TRACKS[at].into(),
                name: None,
                enabled: None,
                locked: Some(true),
                language: None,
            };
            apply(&mut project, op).unwrap();
        }
    }
    project
}

fn fades(id: &str, fade_in: i64, fade_out: i64) -> Op {
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
        fade_in: Some(fade_in),
        fade_out: Some(fade_out),
        source: None,
    }
}

/// Media from a place in it that leaves room for the piece, as far as there is room.
fn media_source(project: &Project, piece: &Piece, rate: Rate, media: usize) -> Source {
    let media = &project.media[media];
    let duration = media.facts.duration;
    let span = columbina_model::span::ticks_of_scaled(
        piece.length + 30,
        SPEEDS[piece.speed],
        rate,
        duration.scale,
    );
    let room = (duration.ticks - span).max(0);
    Source::Media {
        media: media.id.clone(),
        source_in: Time {
            ticks: room * piece.into / 100,
            scale: duration.scale,
        },
    }
}

fn title() -> Source {
    Source::Title {
        text: "Title".into(),
        style: style(),
    }
}

fn style() -> TitleStyle {
    TitleStyle {
        font: String::new(),
        size: 96,
        color: Rgba([255, 255, 255, 255]),
        x: 0.5,
        y: 0.5,
        align: TextAlign::Center,
    }
}

/// The numbers that an edit is made of.
#[derive(Clone, Debug)]
pub struct Making {
    pub kind: usize,
    a: usize,
    b: usize,
    c: usize,
    delta: i64,
    flags: u8,
}

/// The kinds of edit, as `edit_of` numbers them.
pub const KINDS: usize = 28;

pub fn making() -> impl Strategy<Value = Making> {
    making_of(0..KINDS)
}

/// The makings of edits of the kinds that `kinds` gives.
pub fn making_of(kinds: impl Strategy<Value = usize>) -> impl Strategy<Value = Making> {
    (
        kinds,
        any::<usize>(),
        any::<usize>(),
        any::<usize>(),
        prop_oneof![8 => -40i64..=40, 1 => -400i64..=400, 1 => Just(0i64)],
        any::<u8>(),
    )
        .prop_map(|(kind, a, b, c, delta, flags)| Making {
            kind,
            a,
            b,
            c,
            delta,
            flags,
        })
}

impl Making {
    fn flag(&self, bit: u8) -> bool {
        self.flags & (1 << bit) != 0
    }
}

/// The clips of t1, every track.
pub fn clips_of(project: &Project) -> Vec<&Clip> {
    project.timelines[0]
        .tracks
        .iter()
        .flat_map(|r| &r.clips)
        .collect()
}

/// One of `ids`, or one time in twenty one that is not there.
fn pick(ids: &[String], n: usize, bogus: &str) -> String {
    if n.is_multiple_of(20) || ids.is_empty() {
        bogus.into()
    } else {
        ids[n / 20 % ids.len()].clone()
    }
}

/// A frame of t1: one at or near an edge of a clip or a subtitle, or any.
fn frame_of(project: &Project, n: usize) -> i64 {
    let mut edges: Vec<i64> = vec![0];
    for track in &project.timelines[0].tracks {
        for clip in &track.clips {
            edges.extend([clip.start, clip.end()]);
        }
        for cue in &track.cues {
            edges.extend([cue.start, cue.start + cue.length]);
        }
    }
    let base = edges[n % edges.len()];
    let near = [0, 0, 1, -1, 3, -3, 7, 15][n / 64 % 8];
    if n.is_multiple_of(13) {
        i64::try_from(n / 13 % 600).unwrap() - 10
    } else {
        base + near
    }
}

fn track_of(n: usize) -> String {
    if n.is_multiple_of(25) {
        "V9".into()
    } else {
        TRACKS[n / 25 % TRACKS.len()].into()
    }
}

/// A track of subtitles most of the time.
fn subtitle_track_of(n: usize) -> String {
    if n.is_multiple_of(6) {
        track_of(n / 6)
    } else {
        TRACKS[5 + n / 6 % 2].into()
    }
}

/// A track of t1 and a frame of a gap on it, or a track and a frame of `frame_of`.
fn gap_of(project: &Project, n: usize) -> (String, i64) {
    let mut gaps = vec![];
    for track in &project.timelines[0].tracks {
        let clips = track.clips.iter().map(|c| (c.start, c.end()));
        let cues = track.cues.iter().map(|q| (q.start, q.start + q.length));
        let mut spans: Vec<(i64, i64)> = clips.chain(cues).collect();
        spans.sort_unstable();
        let mut end = 0;
        for (start, next_end) in spans {
            if start > end {
                gaps.push((
                    track.id.clone(),
                    end + i64::try_from(n % 7).unwrap() % (start - end),
                ));
            }
            end = end.max(next_end);
        }
    }
    if gaps.is_empty() || n.is_multiple_of(9) {
        (track_of(n), frame_of(project, n / 9))
    } else {
        gaps[n / 9 % gaps.len()].clone()
    }
}

fn tracks_of(n: usize) -> Option<Vec<String>> {
    match n % 4 {
        0 => Some(vec![track_of(n / 4)]),
        1 => Some(vec![track_of(n / 4), track_of(n / 4 / 7)]),
        _ => None,
    }
}

fn material_of(project: &Project, n: usize) -> Material {
    match n % 8 {
        0 => Material::Title {
            text: "New".into(),
            style: style(),
            length: i64::try_from(n / 8 % 60).unwrap(),
        },
        1 => Material::Color {
            color: Rgba([0, 0, 0, 255]),
            length: i64::try_from(n / 8 % 60).unwrap() + 1,
        },
        _ => {
            let media = &project.media[n / 8 % project.media.len()];
            let duration = media.facts.duration;
            let at = |share: usize| Time {
                ticks: duration.ticks * i64::try_from(share % 101).unwrap() / 100,
                scale: duration.scale,
            };
            let length = (n / 1000)
                .is_multiple_of(4)
                .then(|| i64::try_from(n / 4000 % 120).unwrap());
            Material::Media {
                media: media.id.clone(),
                from: (n / 50).is_multiple_of(2).then(|| at(n / 100)),
                to: (n / 50)
                    .is_multiple_of(3)
                    .then(|| at(n / 100 + 20 + n / 7 % 60)),
                length,
            }
        }
    }
}

/// Where the picture and the sound go: left out, none, or a track.
#[allow(clippy::option_option)]
fn targets_of(n: usize) -> [Option<Option<String>>; 2] {
    let one = |n: usize, kind: &str| match n % 5 {
        0 | 1 => None,
        2 => Some(None),
        3 => Some(Some(format!("{kind}2"))),
        _ => Some(Some(track_of(n / 5))),
    };
    [one(n, "V"), one(n / 5, "A")]
}

/// The pairs of clips of t1 that meet on a track.
fn meeting(project: &Project) -> Vec<(String, String)> {
    project.timelines[0]
        .tracks
        .iter()
        .flat_map(|r| {
            r.clips
                .windows(2)
                .filter(|p| p[0].end() == p[1].start)
                .map(|p| (p[0].id.clone(), p[1].id.clone()))
        })
        .collect()
}

/// An edit made of `making`, against `project` as it is.
#[allow(clippy::too_many_lines)]
pub fn edit_of(project: &Project, making: &Making) -> Edit {
    let m = making;
    let clip_ids: Vec<String> = clips_of(project).iter().map(|c| c.id.clone()).collect();
    let clip = pick(&clip_ids, m.a, "c999");
    let clips: Vec<String> = (0..=m.b % 3)
        .map(|k| pick(&clip_ids, m.a.wrapping_add(k * 37), "c999"))
        .collect();
    let frame = frame_of(project, m.b);
    let linked = !m.flag(0);
    let delta = m.delta;
    let timeline = if m.c.is_multiple_of(50) { "t9" } else { "t1" }.to_string();
    let pairs = meeting(project);
    let pair = if pairs.is_empty() || m.c.is_multiple_of(10) {
        (clip.clone(), pick(&clip_ids, m.c, "c999"))
    } else {
        pairs[m.c / 10 % pairs.len()].clone()
    };
    let transitions: Vec<String> = project.timelines[0]
        .tracks
        .iter()
        .flat_map(|r| &r.transitions)
        .map(|x| x.id.clone())
        .collect();
    let cues: Vec<String> = project.timelines[0]
        .tracks
        .iter()
        .flat_map(|r| &r.cues)
        .map(|q| q.id.clone())
        .collect();
    let [video_track, audio_track] = targets_of(m.c);
    let source = material_of(project, m.c);
    match m.kind {
        0 => Edit::Insert {
            timeline,
            source,
            at: frame,
            video_track,
            audio_track,
        },
        1 => Edit::Overwrite {
            timeline,
            source,
            at: frame,
            video_track,
            audio_track,
        },
        2 => Edit::Replace {
            timeline,
            source,
            at: frame,
            video_track,
            audio_track,
        },
        3 => Edit::PlaceOnTop {
            timeline,
            source,
            at: frame,
        },
        4 => Edit::Append {
            timeline,
            source,
            video_track,
            audio_track,
        },
        5 => Edit::SmartInsert {
            timeline,
            source,
            at: frame,
            video_track,
            audio_track,
        },
        6 => Edit::RippleOverwrite {
            timeline,
            source,
            at: frame,
            video_track,
            audio_track,
        },
        7 => Edit::Lift {
            timeline,
            clips,
            linked,
        },
        8 => Edit::RippleDelete {
            timeline,
            clips,
            linked,
        },
        9 => Edit::RippleDeleteRange {
            timeline,
            from: frame,
            to: frame + delta.abs(),
            tracks: tracks_of(m.c),
        },
        10 => {
            let (track, at) = gap_of(project, m.b);
            Edit::CloseGap {
                timeline,
                track,
                at,
                straddling: if m.flag(1) {
                    Straddling::Stay
                } else {
                    Straddling::Refuse
                },
            }
        }
        11 => Edit::Blade {
            timeline,
            at: frame,
            tracks: tracks_of(m.c),
            linked,
        },
        12 => Edit::Move {
            timeline,
            clips,
            delta,
            to_track: m.flag(1).then(|| track_of(m.c)),
            overwrite: m.flag(2),
            linked,
        },
        13 => Edit::InsertMove {
            timeline,
            clips,
            at: frame,
            linked,
        },
        14 => Edit::Trim {
            timeline,
            clip,
            edge: if m.flag(1) { Edge::Out } else { Edge::In },
            delta,
            ripple: m.flag(2),
            linked,
        },
        15 => Edit::Roll {
            timeline,
            left: pair.0,
            right: pair.1,
            delta,
            linked,
        },
        16 => Edit::Slip {
            timeline,
            clip,
            delta,
            linked,
        },
        17 => Edit::Slide {
            timeline,
            clip,
            delta,
            linked,
        },
        18 => Edit::AddTransition {
            timeline,
            left: pair.0,
            right: pair.1,
            kind: None,
            length: delta.abs() / 2,
            align: [Alignment::Center, Alignment::Start, Alignment::End][m.c % 3],
        },
        19 => Edit::RemoveTransition {
            timeline,
            id: pick(&transitions, m.c, "x999"),
        },
        20 => Edit::SetTransition {
            timeline,
            id: pick(&transitions, m.c, "x999"),
            kind: None,
            length: m.flag(1).then_some(delta.abs() / 2),
            align: m
                .flag(2)
                .then_some([Alignment::Center, Alignment::Start, Alignment::End][m.b % 3]),
        },
        21 => Edit::SetSpeed {
            timeline,
            clip,
            speed: SPEEDS[m.c % SPEEDS.len()],
            reverse: m.flag(1).then_some(m.flag(3)),
            ripple: m.flag(2),
            linked,
        },
        22 => Edit::AddCue {
            timeline,
            track: subtitle_track_of(m.c),
            start: frame,
            length: delta.abs(),
            text: "added".into(),
        },
        23 => Edit::SetCue {
            timeline,
            id: pick(&cues, m.c, "q999"),
            start: m.flag(1).then_some(frame),
            length: m.flag(2).then_some(delta.abs()),
            text: None,
        },
        24 => Edit::RemoveCue {
            timeline,
            id: pick(&cues, m.c, "q999"),
        },
        25 => Edit::CuesFrom {
            timeline,
            track: subtitle_track_of(m.c),
            items: (0..m.b % 4)
                .map(|k| CueItem {
                    start: frame + i64::try_from(k).unwrap() * 30,
                    length: delta.abs() % 40 + 1,
                    text: format!("item {k}"),
                })
                .collect(),
        },
        26 => Edit::AddTrack {
            timeline,
            kind: [TrackKind::Video, TrackKind::Audio, TrackKind::Subtitle][m.c % 3],
            index: m.flag(1).then_some(m.b % 4),
            name: None,
            language: None,
        },
        _ => Edit::RemoveTrack {
            timeline,
            track: track_of(m.c),
        },
    }
}

/// Check that a project is one that the operations could have made: `validate` neither refuses
/// it nor changes it, and no two clips of a track cover a frame in common.
pub fn check(project: &Project) {
    let mut copy = project.clone();
    copy.validate()
        .unwrap_or_else(|e| panic!("{e}: {project:#?}"));
    assert_eq!(&copy, project);
    for track in &project.timelines[0].tracks {
        for pair in track.clips.windows(2) {
            assert!(pair[0].end() <= pair[1].start, "{pair:?}");
        }
    }
}
