//! What is seen and heard on random timelines that the operations could have made: at most two
//! layers of a track at a frame, and those only at a transition; every opacity and gain from 0 to
//! 1 times the gain of the clip; the pieces of sound of a track one after another, overlapping
//! only at a crossfade, and covering the samples that the clips of the timeline sound on.

use columbina_model::resolve::{
    audio_between, cues_at, duration, video_at, video_of_track_at, Content,
};
use columbina_model::{
    apply, Alignment, Clip, Cue, Op, Project, Rate, Rgba, Source, Speed, Time, Timeline, Track,
    TrackKind, Transition, TransitionKind,
};
use proptest::prelude::*;

/// The makings of a clip: the frames before it, its length, what it shows, how fast, backward or
/// not, its fades, whether it is on, and how loud.
#[derive(Clone, Debug)]
struct Making {
    gap: i64,
    length: i64,
    kind: u8,
    fifths: i64,
    speed: Speed,
    reverse: bool,
    fades: (i64, i64),
    enabled: bool,
    gain_db: f64,
}

fn making() -> impl Strategy<Value = Making> {
    (
        prop_oneof![3 => Just(0i64), 1 => 1i64..20],
        1i64..60,
        0u8..8,
        0i64..3,
        prop::sample::select(vec![
            Speed::ONE,
            Speed { num: 2, den: 1 },
            Speed { num: 1, den: 2 },
            Speed { num: 3, den: 4 },
        ]),
        any::<bool>(),
        (0i64..30, 0i64..30),
        prop::bool::weighted(0.9),
        prop::sample::select(vec![0.0, -6.0, 6.0]),
    )
        .prop_map(
            |(gap, length, kind, fifths, speed, reverse, fades, enabled, gain_db)| Making {
                gap,
                length,
                kind,
                fifths,
                speed,
                reverse,
                fades,
                enabled,
                gain_db,
            },
        )
}

/// The makings of a timeline: its rate, the clips of V1, V2, A1 and A2, the transitions at the
/// cuts, the subtitles, and which tracks are off.
#[derive(Clone, Debug)]
struct Plan {
    rate: Rate,
    tracks: Vec<Vec<Making>>,
    transitions: Vec<(i64, u8, u8)>,
    cues: Vec<(i64, i64)>,
    off: Vec<bool>,
}

fn plan() -> impl Strategy<Value = Plan> {
    (
        prop::sample::select(vec![
            Rate { num: 25, den: 1 },
            Rate {
                num: 30000,
                den: 1001,
            },
        ]),
        prop::collection::vec(prop::collection::vec(making(), 0..6), 4),
        prop::collection::vec((1i64..24, 0u8..4, 0u8..3), 0..12),
        prop::collection::vec((0i64..10, 1i64..30), 0..6),
        prop::collection::vec(prop::bool::weighted(0.15), 5),
    )
        .prop_map(|(rate, tracks, transitions, cues, off)| Plan {
            rate,
            tracks,
            transitions,
            cues,
            off,
        })
}

/// A project of the sample's media and a timeline made of `plan`, of the clips and transitions
/// that the operations take.
#[allow(
    clippy::too_many_lines,
    reason = "a track of each kind, and what is on them"
)]
fn project_of(plan: &Plan) -> Project {
    let mut project = Project::sample();
    let mut timeline = Timeline::new("t2", "Random", plan.rate);
    let kinds = [
        TrackKind::Video,
        TrackKind::Video,
        TrackKind::Audio,
        TrackKind::Audio,
        TrackKind::Subtitle,
    ];
    for (at, kind) in kinds.into_iter().enumerate() {
        let id = ["V1", "V2", "A1", "A2", "S1"][at];
        let mut track = Track::new(id, kind);
        track.enabled = !plan.off[at];
        timeline.tracks.push(track);
    }
    project.timelines = vec![timeline];
    project.next_id = 10;
    for (r, makings) in plan.tracks.iter().enumerate() {
        let track = ["V1", "V2", "A1", "A2"][r];
        let mut start = 0;
        for making in makings {
            start += making.gap;
            let source = match making.kind {
                6 if r < 2 => Source::Color {
                    color: Rgba([10, 20, 30, 255]),
                },
                7 if r < 2 => Source::Title {
                    text: "Title".into(),
                    style: columbina_model::TitleStyle {
                        font: String::new(),
                        size: 96.0.into(),
                        color: Rgba([255, 255, 255, 255]),
                        x: 0.5.into(),
                        y: 0.5.into(),
                        align: columbina_model::TextAlign::Center,
                        stroke: None,
                        shadow: None,
                        background: None,
                        spans: Vec::new(),
                    },
                },
                _ => Source::Media {
                    media: "m1".into(),
                    source_in: Time {
                        ticks: 15360 * 2 * making.fifths,
                        scale: 15360,
                    },
                },
            };
            let mut clip = Clip::new("", source, start, making.length);
            clip.speed = making.speed;
            clip.reverse = making.reverse;
            clip.fade_in = making.fades.0.min(making.length);
            clip.fade_out = making.fades.1.min(making.length - clip.fade_in);
            clip.enabled = making.enabled;
            clip.gain_db = making.gain_db.into();
            let op = Op::InsertClip {
                timeline: "t2".into(),
                track: track.into(),
                clip,
            };
            if apply(&mut project, op).is_ok() {
                start += making.length;
            }
        }
    }
    let cuts: Vec<(String, String, String, TrackKind)> = project.timelines[0]
        .tracks
        .iter()
        .flat_map(|track| {
            track
                .clips
                .windows(2)
                .filter(|pair| pair[0].end() == pair[1].start)
                .map(|pair| {
                    (
                        track.id.clone(),
                        pair[0].id.clone(),
                        pair[1].id.clone(),
                        track.kind,
                    )
                })
        })
        .collect();
    for (at, &(length, which, align)) in plan.transitions.iter().enumerate() {
        let Some((track, left, right, kind)) = cuts.get(at % cuts.len().max(1)).cloned() else {
            break;
        };
        let kind = if kind == TrackKind::Audio {
            TransitionKind::Crossfade
        } else {
            [
                TransitionKind::Dissolve,
                TransitionKind::DipBlack,
                TransitionKind::DipWhite,
                TransitionKind::Dissolve,
            ][usize::from(which)]
        };
        for (id, fade_in, fade_out) in [(&left, None, Some(0)), (&right, Some(0), None)] {
            let op = Op::SetClip {
                timeline: "t2".into(),
                id: id.clone(),
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
                fade_in,
                fade_out,
                audio_fade_in: None,
                audio_fade_out: None,
                eq: None,
                effects: None,
                pitch_correction: None,
                pitch: None,
                channels: None,
                source: None,
            };
            apply(&mut project, op).unwrap();
        }
        let op = Op::AddTransition {
            timeline: "t2".into(),
            track,
            transition: Transition {
                id: String::new(),
                kind,
                left,
                right,
                length,
                align: [Alignment::Center, Alignment::Start, Alignment::End][usize::from(align)],
                fade_in_level_db: columbina_model::CrossfadeLevel::Zero,
                fade_out_level_db: columbina_model::CrossfadeLevel::Zero,
            },
        };
        let _ = apply(&mut project, op);
    }
    let mut start = 0;
    for &(gap, length) in &plan.cues {
        start += gap;
        let op = Op::AddCue {
            timeline: "t2".into(),
            track: "S1".into(),
            cue: Cue {
                id: String::new(),
                start,
                length,
                text: "字幕".into(),
            },
        };
        apply(&mut project, op).unwrap();
        start += length;
    }
    project
}

/// The sample that frame `frame` starts at, rounded down.
fn sample_of(frame: i64, rate: Rate, sample_rate: u32) -> i64 {
    (i128::from(frame) * i128::from(sample_rate) * i128::from(rate.den) / i128::from(rate.num))
        .try_into()
        .unwrap()
}

/// The samples that the clips of the sound of a timeline sound on, from `from` to `to`: those of
/// each clip that is on, on a track that is on, with the frames that crossfades show beyond it.
fn sounding(timeline: &Timeline, from: i64, to: i64) -> Vec<(i64, i64)> {
    let mut spans = vec![];
    for track in &timeline.tracks {
        if track.kind != TrackKind::Audio || !track.enabled {
            continue;
        }
        for clip in track.clips.iter().filter(|c| c.enabled) {
            // The frames of the timeline that it sounds on: its own, and those of a crossfade
            // at either end, which lies over the cut as it is aligned.
            let (mut first, mut end) = (clip.start, clip.end());
            for x in &track.transitions {
                let cut = if x.left == clip.id {
                    clip.end()
                } else if x.right == clip.id {
                    clip.start
                } else {
                    continue;
                };
                let (lies_from, lies_to) = lies_over(x, cut);
                first = first.min(lies_from);
                end = end.max(lies_to);
            }
            let start = sample_of(first, timeline.rate, timeline.sample_rate);
            let end = sample_of(end, timeline.rate, timeline.sample_rate);
            let (start, end) = (start.max(from), end.min(to));
            if start < end {
                spans.push((start, end));
            }
        }
    }
    spans
}

/// The frames that a transition lies over about its cut, counted from its alignment: from the
/// cut, to it, or half before it, the frame left over being after it.
fn lies_over(transition: &Transition, cut: i64) -> (i64, i64) {
    let length = transition.length;
    match transition.align {
        Alignment::Start => (cut, cut + length),
        Alignment::End => (cut - length, cut),
        Alignment::Center => (cut - length / 2, cut - length / 2 + length),
    }
}

/// How many samples a list of spans covers, once each.
fn covered(mut spans: Vec<(i64, i64)>) -> i64 {
    spans.sort_unstable();
    let mut total = 0;
    let mut reach = i64::MIN;
    for (start, end) in spans {
        let start = start.max(reach);
        if end > start {
            total += end - start;
            reach = end;
        }
    }
    total
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    #[test]
    fn a_frame_shows_at_most_two_layers_of_a_track_each_seen_in_part_or_whole(plan in plan()) {
        let project = project_of(&plan);
        let timeline = &project.timelines[0];
        let end = duration(timeline);
        let order: Vec<&str> = timeline.tracks.iter().map(|r| r.id.as_str()).collect();
        for frame in -2..end + 3 {
            let layers = video_at(timeline, &project, frame);
            let mut last = 0;
            for layer in &layers {
                prop_assert!((0.0..=1.0).contains(&layer.opacity), "{layer:?}");
                let at = order.iter().position(|id| *id == layer.track).unwrap();
                prop_assert!(at >= last, "out of order at {frame}: {layers:?}");
                last = at;
                let track = &timeline.tracks[at];
                prop_assert!(track.enabled && track.kind == TrackKind::Video);
                // A frame of media is within the media.
                if let Content::Media { media, tick } = &layer.content {
                    let duration = project.media.iter().find(|m| m.id == *media).unwrap().facts.duration;
                    prop_assert!((0..duration.ticks).contains(tick), "{layer:?} of {}", duration.ticks);
                }
            }
            // One track asked alone shows what it shows among the others.
            let alone: Vec<_> = timeline
                .tracks
                .iter()
                .flat_map(|track| video_of_track_at(timeline, &project, track, frame))
                .collect();
            prop_assert_eq!(&alone, &layers);
            for track in &timeline.tracks {
                let of_track = layers.iter().filter(|l| l.track == track.id).count();
                prop_assert!(of_track <= 2, "{of_track} layers of {} at {frame}", track.id);
                // Two layers of a track only where a transition of it lies.
                if of_track == 2 {
                    let at_transition = track.transitions.iter().any(|x| {
                        let cut = track.clips.iter().find(|c| c.id == x.right).unwrap().start;
                        let (from, to) = lies_over(x, cut);
                        (from..to).contains(&frame)
                    });
                    prop_assert!(at_transition, "two layers of {} at {frame}: {layers:?}", track.id);
                }
            }
            if frame < 0 || frame >= end {
                prop_assert!(layers.is_empty(), "{layers:?} at {frame}");
            }
            let cues = cues_at(timeline, frame);
            prop_assert!(cues.len() <= 1);
            for cue in cues {
                prop_assert!(cue.start <= frame && frame < cue.start + cue.length);
            }
        }
    }

    #[test]
    fn the_sound_of_a_track_is_pieces_one_after_another_that_cover_what_sounds(
        plan in plan(),
        from in 0i64..200_000,
        span in 1i64..400_000,
    ) {
        let project = project_of(&plan);
        let timeline = &project.timelines[0];
        let to = from + span;
        let pieces = audio_between(timeline, &project, from, to);
        let mut spans = vec![];
        for (at, piece) in pieces.iter().enumerate() {
            prop_assert!(from <= piece.from && piece.from < piece.to && piece.to <= to, "{piece:?}");
            let clip = timeline.tracks.iter().flat_map(|r| &r.clips).find(|c| c.id == piece.clip).unwrap();
            let most = 10f64.powf(clip.gain_db.fixed().unwrap() / 20.0) * 1.0001;
            for sample in [piece.from, i64::midpoint(piece.from, piece.to), piece.to - 1] {
                let gain = piece.gain.at(sample);
                prop_assert!(gain.is_finite() && (0.0..=most).contains(&f64::from(gain)), "{gain} of {piece:?}");
            }
            spans.push((piece.from, piece.to));
            // On a track, a piece starts where the one before it ends or later, but at a
            // crossfade, where the two overlap.
            if let Some(next) = pieces.get(at + 1).filter(|next| next.track == piece.track) {
                let track = timeline.tracks.iter().find(|r| r.id == piece.track).unwrap();
                let joined = track.transitions.iter().any(|x| x.left == piece.clip && x.right == next.clip);
                prop_assert!(piece.from <= next.from);
                prop_assert!(next.from >= piece.to || joined, "{piece:?} and {next:?}");
            }
        }
        prop_assert_eq!(covered(spans), covered(sounding(timeline, from, to)));
    }
}

#[test]
fn a_timeline_lasts_until_its_last_clip_or_subtitle_ends() {
    let plan = Plan {
        rate: Rate { num: 25, den: 1 },
        tracks: vec![vec![], vec![], vec![], vec![]],
        transitions: vec![],
        cues: vec![(30, 20)],
        off: vec![false; 5],
    };
    let project = project_of(&plan);
    assert_eq!(duration(&project.timelines[0]), 50);
    assert_eq!(duration(&Timeline::new("t", "Empty", plan.rate)), 0);
}
