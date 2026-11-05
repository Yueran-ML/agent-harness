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
    apply, apply_done, Alignment, AudioFacts, AudioFade, AudioParam, ChannelMap, ChannelSource,
    ChannelSources, Clip, CueItem, Edge, Edit, Facts, Identity, Material, Media, Op, Project, Rate,
    Rgba, Source, Speed, Straddling, TextAlign, Time, Timeline, TitleStyle, Track, TrackKind,
    Transition, TransitionKind, VideoFacts,
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
                color: None,
            }),
            audio: audio.then(|| AudioFacts {
                rate: 48000,
                channels: 2,
                codec: "aac".into(),
                layout: None,
                streams: None,
            }),
        },
        proxy: None,
        name: format!("{id}.mp4"),
        bin: None,
        mark_in: None,
        mark_out: None,
        color_override: columbina_model::ColorOverride::default(),
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
            // On a track of sound, now and then fades of sound of their own, and its channels
            // named.
            if r >= 3 {
                let sound = |units: i64, curve| {
                    (units % 2 == 1).then_some(AudioFade {
                        length: Time {
                            ticks: units * 7,
                            scale: 100,
                        },
                        curve,
                        skew: -0.25,
                    })
                };
                clip.audio_fade_in = sound(piece.fades.0, 0.5);
                clip.audio_fade_out = sound(piece.fades.1, -1.0);
                if piece.into % 5 == 0 {
                    clip.channels = Some(channels_of(piece.into));
                }
            }
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
            fade_in_level_db: columbina_model::CrossfadeLevel::Zero,
            fade_out_level_db: columbina_model::CrossfadeLevel::Zero,
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
                ((0, None), (clip.fade_out, clip.audio_fade_out))
            } else {
                ((clip.fade_in, clip.audio_fade_in), (0, None))
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
                mute: None,
                solo: None,
                solo_safe: None,
            };
            apply(&mut project, op).unwrap();
        }
    }
    // Now and then the tracks of sound in a group, which links all it may or some of it, a
    // second bus, and the solo exclusive.
    if recipe.pieces.len() % 2 == 1 {
        let linked = if recipe.pieces.len() % 4 == 1 {
            serde_json::json!([
                "editing",
                "fader",
                "solo",
                "mute",
                "sends",
                "panning",
                "automation"
            ])
        } else {
            serde_json::json!(["fader", "solo"])
        };
        let exclusive = recipe.transitions.len() % 2 == 1;
        for op in [
            serde_json::json!({"op": "add_track_group", "timeline": "t1",
                "group": {"id": "", "name": "Sound", "members": ["A1", "A2"], "linked": linked}}),
            serde_json::json!({"op": "set_timeline_audio", "timeline": "t1",
                "patch": {"exclusive_solo": exclusive}}),
            serde_json::json!({"op": "add_bus", "timeline": "t1", "bus": {"id": "", "name": "Verb",
                "color": "#808080ff", "format": "stereo", "strip": {"outputs": [{"bus": "B1"}]}}}),
            serde_json::json!({"op": "set_track_audio", "timeline": "t1", "track": "A2",
                "patch": {"strip": {"fader_db": -6, "sends": [{"bus": "B2", "level_db": -12}]}}}),
            // Lanes of automation: of a send, of a fader, and of an insert (T14).
            serde_json::json!({"op": "add_insert", "timeline": "t1", "of": {"track": "A1"},
                "insert": {"effect": {"kind": "gain", "gain_db": -2}}}),
            serde_json::json!({"op": "add_insert", "timeline": "t1", "of": {"bus": "B2"},
                "insert": {"effect": {"kind": "soft_clipper"}}}),
            serde_json::json!({"op": "set_lane", "timeline": "t1", "of": {"track": "A2"},
                "path": "send.B2.level_db", "lane": {"points": [
                    {"at": {"ticks": 0, "scale": 48000}, "value": -12},
                    {"at": {"ticks": 96000, "scale": 48000}, "value": -3}]}}),
            serde_json::json!({"op": "set_lane", "timeline": "t1", "of": {"track": "A1"},
                "path": "fader_db", "lane": {"points": [
                    {"at": {"ticks": 24000, "scale": 48000}, "value": -10},
                    {"at": {"ticks": 48000, "scale": 48000}, "value": 0}]}}),
            serde_json::json!({"op": "set_lane", "timeline": "t1", "of": {"track": "A1"},
                "path": "insert.fx1.gain_db", "lane": {"points": [
                    {"at": {"ticks": 0, "scale": 48000}, "value": 3}]}}),
        ] {
            apply(&mut project, serde_json::from_value(op).unwrap()).unwrap();
        }
    }
    project
}

/// Set the fades of a clip: of its frames, and of its sound.
fn fades(
    id: &str,
    (fade_in, sound_in): (i64, Option<AudioFade>),
    (fade_out, sound_out): (i64, Option<AudioFade>),
) -> Op {
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
        audio_fade_in: Some(sound_in),
        audio_fade_out: Some(sound_out),
        eq: None,
        effects: None,
        pitch_correction: None,
        pitch: None,
        channels: None,
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
pub const KINDS: usize = 42;

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
            linked: m.flag(3),
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
            fade_in_level_db: m.flag(4).then_some(columbina_model::CrossfadeLevel::Plus3),
            fade_out_level_db: m.flag(5).then_some(columbina_model::CrossfadeLevel::Minus3),
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
        27 => Edit::RemoveTrack {
            timeline,
            track: track_of(m.c),
        },
        _ => sound_edit_of(project, m, &timeline, &clip, &clips),
    }
}

/// A media time of a clip of t1: one of its keys, a place within what it plays of its media,
/// or now and then a second or a fraction of one, which may be outside it.
fn media_time_of(project: &Project, clip: &str, n: usize) -> Time {
    let found = clips_of(project).into_iter().find(|c| c.id == clip);
    let keys = found
        .and_then(|c| match &c.gain_db {
            AudioParam::Keys(keys) => Some(keys.clone()),
            AudioParam::Fixed(_) => None,
        })
        .unwrap_or_default();
    let within = found.and_then(|c| {
        let start = c.source_in()?;
        let span = columbina_model::span::source_span(c, project.timelines[0].rate, start.scale);
        let share = i64::try_from(n % 5).unwrap();
        Some(Time {
            ticks: start.ticks + span * share / 4,
            scale: start.scale,
        })
    });
    match (n % 6, keys.is_empty(), within) {
        (0, false, _) => keys[n / 6 % keys.len()].at,
        (1, _, _) => Time {
            ticks: i64::try_from(n % 9).unwrap(),
            scale: [1, 2, 4][n % 3],
        },
        (_, _, Some(within)) => within,
        _ => Time {
            ticks: i64::try_from(n % 97).unwrap() * 500,
            scale: 48000,
        },
    }
}

/// A clip of sound of t1, most of the time, or `other`.
fn sound_clip_of(project: &Project, n: usize, other: &str) -> String {
    let sound: Vec<String> = project.timelines[0]
        .tracks
        .iter()
        .filter(|r| r.kind == TrackKind::Audio)
        .flat_map(|r| &r.clips)
        .map(|c| c.id.clone())
        .collect();
    if sound.is_empty() || n.is_multiple_of(7) {
        other.into()
    } else {
        sound[n / 7 % sound.len()].clone()
    }
}

/// An edit of sound made of `m`, kinds 28 to 41 (36 to 41 of inserts and lanes, T14).
#[allow(clippy::too_many_lines, reason = "an arm for each edit of sound")]
fn sound_edit_of(
    project: &Project,
    m: &Making,
    timeline: &str,
    clip: &str,
    clips: &[String],
) -> Edit {
    let delta_db = [3.0, -1.0, -6.0, 40.0, 0.0][m.c % 5];
    let edit = |value: serde_json::Value| -> Edit { serde_json::from_value(value).unwrap() };
    let sound = sound_clip_of(project, m.a, clip);
    let clip = sound.as_str();
    let tracks = ["A1", "A2", "V1", "A9"];
    match m.kind {
        28 => {
            let patch = match m.b % 6 {
                0 => serde_json::json!({"strip": {"fader_db": m.delta.clamp(-30, 12)}}),
                1 => serde_json::json!({"mute": m.flag(1), "solo": m.flag(2)}),
                2 => serde_json::json!({"strip": {"pan": {"x": 0.5, "divergence": 40}}}),
                3 => {
                    serde_json::json!({"strip": {"sends": [{"bus": "B2", "level_db": m.delta.clamp(-30, 12), "on": m.flag(2)}]}})
                }
                4 => serde_json::json!({"automation_arm": m.flag(1), "strip": {"fader_db": -3}}),
                _ => serde_json::json!({"format": "mono"}),
            };
            edit(
                serde_json::json!({"edit": "set_track_audio", "timeline": timeline,
                "track": tracks[m.c % 4], "patch": patch, "follow_groups": !m.flag(4)}),
            )
        }
        29 => {
            let action = ["add", "remove", "move", "set_interp"][m.b % 4];
            let at = media_time_of(project, clip, m.c);
            let to = media_time_of(project, clip, m.c / 3 + 1);
            edit(
                serde_json::json!({"edit": "clip_gain_keys", "timeline": timeline, "clip": clip,
                "param": (["gain_db", "pan"][usize::from(m.flag(1))]), "action": action, "at": at,
                "to": to, "value": (m.flag(2).then_some(-3.0)), "interp": (["ease_in", "linear"][usize::from(m.flag(3))])}),
            )
        }
        30 => {
            let from = media_time_of(project, clip, m.c);
            let to = media_time_of(project, clip, m.c + 2);
            edit(
                serde_json::json!({"edit": "clip_gain_range", "timeline": timeline, "clip": clip,
                "from": from, "to": to, "delta_db": delta_db}),
            )
        }
        31 => edit(
            serde_json::json!({"edit": "nudge_clip_gain", "timeline": timeline,
            "clips": clips, "delta_db": delta_db, "linked": !m.flag(0)}),
        ),
        32 => edit(
            serde_json::json!({"edit": "paste_attributes", "timeline": timeline,
            "from": clip, "to": clips, "items": (["volume", "plugins", "equalizer"][..=(m.b % 3)])}),
        ),
        33 => edit(
            serde_json::json!({"edit": "remove_attributes", "timeline": timeline,
            "clips": clips, "items": (["volume", "plugins", "equalizer"][(m.b % 3)..])}),
        ),
        34 => {
            // Most of the time inside the clip, a quarter to three quarters of the way along.
            let inside = clips_of(project)
                .into_iter()
                .find(|c| c.id == clip)
                .map(|c| c.start + (c.length * i64::try_from(m.c % 3 + 1).unwrap() / 4).max(1));
            let at = match inside {
                Some(at) if !m.c.is_multiple_of(5) => at,
                _ => frame_of(project, m.c),
            };
            let chosen = if m.flag(2) {
                clips.to_vec()
            } else {
                vec![clip.to_string()]
            };
            edit(
                serde_json::json!({"edit": "fade_to_playhead", "timeline": timeline,
                "clips": chosen, "side": (["in", "out"][usize::from(m.flag(1))]), "at": at}),
            )
        }
        35 => {
            let of = |n: usize| match n % 3 {
                0 => serde_json::json!({"track": tracks[n / 3 % 4]}),
                1 => serde_json::json!({"bus": (["B1", "B2", "B9"][n / 3 % 3])}),
                _ => serde_json::json!({"clip": clip}),
            };
            edit(
                serde_json::json!({"edit": "copy_effects", "timeline": timeline,
                "from": of(m.b), "to": [of(m.c), of(m.c / 7)]}),
            )
        }
        36 | 37 => {
            // A1 has a Gain with a lane, B2 a Soft Clipper; the rest none.
            let of = |n: usize| {
                [
                    serde_json::json!({"track": "A1"}),
                    serde_json::json!({"track": "A1"}),
                    serde_json::json!({"bus": "B2"}),
                    serde_json::json!({"clip": clip}),
                    serde_json::json!({"track": "V1"}),
                ][n % 5]
                    .clone()
            };
            let slot = usize::from(m.c.is_multiple_of(7));
            if m.kind == 36 {
                let insert = (!m.flag(1)).then(
                    || serde_json::json!({"id": "fx1", "effect": {"kind": "gain", "gain_db": -4}}),
                );
                edit(
                    serde_json::json!({"edit": "set_insert", "timeline": timeline,
                    "of": of(m.b), "slot": slot, "insert": insert}),
                )
            } else {
                let to_of = m.flag(2).then(|| of(m.c));
                edit(
                    serde_json::json!({"edit": "move_insert", "timeline": timeline,
                    "of": of(m.b), "from": slot, "to": m.a % 2, "to_of": to_of}),
                )
            }
        }
        _ => lane_edit_of(m, timeline),
    }
}

/// An edit of a lane of automation (T14): of A1, A2, B2, a VCA that is not there, or V1; of a
/// fader, a send, an insert, a mute or a pan.
fn lane_edit_of(m: &Making, timeline: &str) -> Edit {
    let of = [
        serde_json::json!({"track": "A1"}),
        serde_json::json!({"track": "A2"}),
        serde_json::json!({"bus": "B2"}),
        serde_json::json!({"vca": "VCA9"}),
        serde_json::json!({"track": "V1"}),
    ][m.a % 5]
        .clone();
    let path = [
        "fader_db",
        "send.B2.level_db",
        "insert.fx1.gain_db",
        "mute",
        "pan.x",
    ][m.b % 5];
    let ms = |n: i64| serde_json::json!({"ticks": n * 48, "scale": 48000});
    #[allow(clippy::cast_possible_wrap, reason = "a small number")]
    let (c, d) = ((m.c % 3000) as i64, m.delta);
    let value = [-6.0, 0.5, 1.0, 20.0, -0.75][m.c % 5];
    let edit = |value: serde_json::Value| -> Edit { serde_json::from_value(value).unwrap() };
    match m.kind {
        38 => {
            let action = ["add", "remove", "move"][m.c % 3];
            edit(
                serde_json::json!({"edit": "lane_points", "timeline": timeline, "of": of,
                "path": path, "action": action,
                "points": [{"at": ms(c)}, {"at": ms(c + d.abs()), "value": value}],
                "at": [ms([0, 500, 1000, 2000, c][m.a / 5 % 5])], "by": ms(d * 10),
                "delta": (m.flag(1).then_some(value))}),
            )
        }
        39 => {
            let action = ["delete", "cut", "paste", "shift_value", "thin"][m.c % 5];
            edit(
                serde_json::json!({"edit": "lane_range", "timeline": timeline, "of": of,
                "path": path, "action": action, "from": ms(c), "to": ms(c + d * 20),
                "points": [{"at": ms(0), "value": value}, {"at": ms(d.abs() * 5), "value": 0}],
                "delta": value, "tolerance": (m.flag(3).then_some(0.5))}),
            )
        }
        40 => {
            let points: Vec<serde_json::Value> = (0..20)
                .map(|k| serde_json::json!({"at": ms(c + k * d.abs()), "value": value * f64::from(i32::try_from(k % 3).unwrap())}))
                .collect();
            edit(
                serde_json::json!({"edit": "lane_draw", "timeline": timeline, "of": of,
                "path": path, "from": ms(c), "to": ms(c + 20 * d.abs()), "points": points}),
            )
        }
        _ => {
            let paths = m.flag(1).then(|| vec![path]);
            edit(
                serde_json::json!({"edit": "clear_lanes", "timeline": timeline, "of": of,
                "paths": paths, "then": (["keep", "reset", "hold"][m.c % 3]),
                "at": (m.flag(2).then(|| ms(c)))}),
            )
        }
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

/// The channels of a stereo clip of sound that names them: left and right as they are, the other
/// way round, or both the mean of the two.
fn channels_of(n: i64) -> Vec<ChannelMap> {
    let pick = |channels: &[u32]| ChannelMap {
        source: ChannelSources(
            channels
                .iter()
                .map(|&channel| ChannelSource { stream: 0, channel })
                .collect(),
        ),
        enabled: true,
        mute: false,
        level_db: 0.0,
    };
    match n % 3 {
        0 => vec![pick(&[0]), pick(&[1])],
        1 => vec![pick(&[1]), pick(&[0])],
        _ => vec![pick(&[0, 1]), pick(&[0, 1])],
    }
}
