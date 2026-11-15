//! Where clips with speed points read their source, as the engine finds it (`retime`, `span`,
//! [`KeyClock`] and the pieces of sound of `resolve`), written out for the copies of this
//! arithmetic that clients keep: the desktop (`desktop/src/studio/retime.ts`) and the agent
//! (`src/columbina/video/retime.py`) are each held to every number here that they have a
//! function for (PROTOCOL.md, "speed points").
//!
//! The file `tests/data/retime.json` is written anew when `COLUMBINA_WRITE_EXAMPLES=1` is set;
//! otherwise it is checked to be as the engine finds the numbers now. It is a list of cases,
//! one on each line. A case is a clip of media on a timeline, and what the engine finds for it:
//!
//! - `name`, `clip` and `media` as the engine writes them, the `rate` of the timeline and its
//!   `sample_rate`. The scale of the media is that of its duration, and the samples a second of
//!   its sound are in its facts. A clip of one speed has its speed and its direction, and no
//!   speed points, as a project keeps it: it is asked the same questions.
//! - `unit` and `segments`: the unit that the speeds are counted in, and the frames and the
//!   speed of each segment.
//! - `frames` and `ticks`: frames of the clip, counted from its first, from a few before it to
//!   a few after it as a transition reads them, and the tick of the media that each shows.
//! - `span`, and `shown`: the lowest tick shown (`first`), the highest (`last`) and where what
//!   is shown ends (`end`), counted from where the clip begins in its media, with `before`
//!   frames more before the clip and `after` more after it.
//! - `frame_of`: `ticks` after where the clip begins in its media, and in `frames` the first
//!   frame of the clip that each is on show in, or null.
//! - `speed_curve` and `frame_curve`: the points of the Retime Speed and Retime Frame curves,
//!   as floating numbers.
//! - `keys`: `frames` of the timeline, the frame of the keys `at` each, and the first and the
//!   last frame of the media that the clip shows (`shown`).
//! - `sound`: the `parts` of the timeline's samples that are of each segment (null where a
//!   part has no end); the pieces heard in each of three `windows` of samples, one about the
//!   whole clip and two that begin within it; and `samples` of the timeline with the sample of
//!   the media that each is read from (`source_samples`).
//! - `from_frame` and `with_length`: the speed points, the length and how much later the clip
//!   begins in its media where its head is moved to a frame, and its tail to a length.
//!
//! A count that JavaScript does not hold exactly, further than 2^53 - 1 from 0, is written as
//! the string of its digits, and any other as a number.

use std::path::Path;

use columbina_model::keyframes::KeyClock;
use columbina_model::resolve::{audio_between, samples_at, AudioPiece};
use columbina_model::retime::{Course, Retime, SignedSpeed, Sounding, SpeedPoint};
use columbina_model::span;
use columbina_model::{Clip, Media, Project, Rate, Time, LARGEST};
use serde_json::{json, Value};

const FILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/retime.json");

/// The timeline that a case is on, and the media that its clip is cut from.
#[derive(Clone, Copy)]
struct Grid {
    /// The rate of the timeline.
    rate: Rate,
    /// The ticks a second of the media.
    scale: u32,
    /// The rate of the picture of the media.
    picture: Rate,
    /// The samples a second of the timeline.
    sample_rate: u32,
    /// The samples a second of the sound of the media.
    media_rate: u32,
}

const fn rate(num: u32, den: u32) -> Rate {
    Rate { num, den }
}

const fn grid(rate: Rate, scale: u32, picture: Rate, sample_rate: u32, media_rate: u32) -> Grid {
    Grid {
        rate,
        scale,
        picture,
        sample_rate,
        media_rate,
    }
}

const PAL: Rate = rate(25, 1);
const NTSC: Rate = rate(30000, 1001);
const FILM: Rate = rate(24000, 1001);

/// 25 frames a second, where a frame lasts 614.4 ticks of the media.
const PAL_FINE: Grid = grid(PAL, 15360, PAL, 48000, 48000);
/// 25 frames a second, where a frame lasts one tick, and the media has other samples a second.
const PAL_WHOLE: Grid = grid(PAL, 25, PAL, 48000, 44100);
/// 29.97 frames a second in thousandths of a second, of media of 25 frames a second.
const NTSC_COARSE: Grid = grid(NTSC, 1000, PAL, 48000, 44100);
/// 29.97 frames a second, where a frame lasts 1001 ticks.
const NTSC_WHOLE: Grid = grid(NTSC, 30000, NTSC, 48000, 48000);
/// 23.976 frames a second, where a frame lasts 1001 ticks, on a timeline of 44100 samples.
const FILM_WHOLE: Grid = grid(FILM, 24000, FILM, 44100, 48000);
/// 23.976 frames a second in the finest ticks that a scale holds.
const FAR: Grid = grid(FILM, u32::MAX, FILM, 48000, 96000);

/// Where a clip lies: the frame of the timeline that it begins at, the frames that it lasts,
/// and the tick of its media that it begins at.
type Place = (i64, i64, i64);

/// A case: a clip on a timeline, and the media that it is cut from.
struct Case {
    name: &'static str,
    /// The rate of the timeline.
    rate: Rate,
    /// The samples a second of the timeline.
    sample_rate: u32,
    clip: Clip,
    media: Media,
}

/// A case of a clip that plays as `points` say, each a frame and the `num` and `den` of its
/// speed: kept as a clip keeps it, one speed as its speed and its direction. Its media is the
/// sample's, with a picture and sound, long enough for the clip and the frames that a
/// transition shows about it, with a second to spare, and no longer than a project may hold.
fn case(
    name: &'static str,
    grid: Grid,
    (start, length, begins): Place,
    points: &[(i64, i64, u32)],
) -> Case {
    let at = Time {
        ticks: begins,
        scale: grid.scale,
    };
    let mut clip = Clip::of_media("c1", "m1", at, start, length);
    let points = points.iter().map(|&(frame, num, den)| SpeedPoint {
        frame,
        speed: SignedSpeed { num, den },
    });
    clip.set_mapping(Some(Retime {
        points: points.collect(),
    }));
    let mut media = Project::sample().media[0].clone();
    media.facts.video.as_mut().unwrap().rate = grid.picture;
    media.facts.audio.as_mut().unwrap().rate = grid.media_rate;
    let ends = span::shown(&clip, 8, 8, grid.rate, grid.scale).unwrap().end;
    let ticks = begins
        .saturating_add(ends)
        .saturating_add(i64::from(grid.scale))
        .min(LARGEST);
    media.facts.duration = Time {
        ticks,
        scale: grid.scale,
    };
    Case {
        name,
        rate: grid.rate,
        sample_rate: grid.sample_rate,
        clip,
        media,
    }
}

/// The cases. The last is of numbers near the most that the engine counts: its ticks are more
/// than JavaScript holds exactly, those of its last segment more than 64 bits hold, and its
/// media is shorter than what it shows, which a project would not hold.
#[allow(clippy::too_many_lines, reason = "a case a line or two")]
fn cases() -> Vec<Case> {
    let awkward: [(i64, i64, u32); 2] = [(0, 3, 7), (13, 5, 3)];
    vec![
        case(
            "one speed, forward",
            PAL_FINE,
            (10, 40, 30727),
            &[(0, 1, 1)],
        ),
        case(
            "twice as fast, forward",
            NTSC_COARSE,
            (7, 30, 1234),
            &[(0, 2, 1)],
        ),
        case(
            "half as fast, forward",
            PAL_WHOLE,
            (0, 24, 50),
            &[(0, 1, 2)],
        ),
        case(
            "one speed, backward",
            PAL_FINE,
            (25, 36, 4000),
            &[(0, -1, 1)],
        ),
        case(
            "twice as fast, backward",
            NTSC_WHOLE,
            (1001, 30, 30000),
            &[(0, -2, 1)],
        ),
        case(
            "half as fast, backward",
            FILM_WHOLE,
            (3, 40, 1001),
            &[(0, -1, 2)],
        ),
        case(
            "a speed point where the speed does not change",
            NTSC_COARSE,
            (5, 30, 500),
            &[(0, 3, 2), (11, 3, 2), (12, 3, 2)],
        ),
        case(
            "forward, a freeze, forward",
            PAL_FINE,
            (50, 40, 30720),
            &[(0, 1, 1), (12, 0, 1), (20, 3, 4)],
        ),
        case(
            "forward, backward further than it came, forward: the lowest place inside the clip",
            PAL_WHOLE,
            (0, 30, 100),
            &[(0, 1, 1), (8, -3, 2), (22, 1, 1)],
        ),
        case(
            "forward, then backward to the end, below where it began",
            PAL_FINE,
            (4, 20, 9216),
            &[(0, 1, 2), (10, -2, 1)],
        ),
        case(
            "backward first",
            NTSC_WHOLE,
            (12, 40, 3003),
            &[(0, -1, 1), (10, 1, 2), (25, 2, 1)],
        ),
        case(
            "a freeze first",
            PAL_FINE,
            (100, 30, 15360),
            &[(0, 0, 1), (6, -1, 1), (16, 1, 1)],
        ),
        case(
            "3/7 then 5/3 at 29.97, in thousandths of a second",
            NTSC_COARSE,
            (3, 37, 777),
            &awkward,
        ),
        case(
            "3/7 then 5/3 at 29.97, in ticks of 1/30000",
            NTSC_WHOLE,
            (3, 37, 23311),
            &awkward,
        ),
        case(
            "3/7 then 5/3 at 25, in ticks of 1/25",
            PAL_WHOLE,
            (3, 37, 19),
            &awkward,
        ),
        case(
            "3/7 then 5/3 at 25, in ticks of 1/15360",
            PAL_FINE,
            (3, 37, 11937),
            &awkward,
        ),
        case(
            "5/3 backward, 3/7, then 7/11 backward at 23.976",
            FILM_WHOLE,
            (2, 41, 48048),
            &[(0, -5, 3), (9, 3, 7), (30, -7, 11)],
        ),
        case(
            "twelve speed points",
            NTSC_COARSE,
            (20, 75, 9000),
            &[
                (0, 1, 1),
                (5, 2, 1),
                (9, -1, 2),
                (16, 0, 1),
                (20, 3, 4),
                (27, -4, 3),
                (33, 1, 3),
                (40, 5, 2),
                (41, -3, 2),
                (50, 0, 1),
                (58, 7, 5),
                (66, -2, 7),
            ],
        ),
        case(
            "the slowest and the fastest that a segment plays",
            PAL_FINE,
            (0, 60, 0),
            &[(0, 1, 100), (30, 100, 1), (34, -100, 1), (37, -1, 100)],
        ),
        case(
            "large numbers",
            FAR,
            (1_000_000_007, 34_359_738_368, 123_456_789_012),
            &[
                (0, 65536, 65537),
                (17_179_869_184, -65520, 65521),
                (25_769_803_776, 100, 1),
            ],
        ),
    ]
}

/// A count as it is written: a number where JavaScript holds it exactly, and else the string of
/// its digits.
fn whole(count: i64) -> Value {
    if (-LARGEST..=LARGEST).contains(&count) {
        json!(count)
    } else {
        json!(count.to_string())
    }
}

/// The project of a case: the sample's, with the media of the case alone, and its clip alone,
/// on the track of sound of a timeline of its rate and its samples a second.
fn project_of(case: &Case) -> Project {
    let mut project = Project::sample();
    project.media = vec![case.media.clone()];
    let timeline = &mut project.timelines[0];
    timeline.rate = case.rate;
    timeline.sample_rate = case.sample_rate;
    timeline.tracks[0].clips.clear();
    timeline.tracks[1].clips = vec![case.clip.clone()];
    project
}

/// The frames of a clip that a case is asked at, counted from its first: every frame from 3
/// before it to 3 after it, or, of a clip of more than 200 frames, those about its end and
/// about each of its speed points, and the middle of each segment.
fn frames_of(course: &Course) -> Vec<i64> {
    let length = course.length();
    if length <= 200 {
        return (-3..length + 3).collect();
    }
    let mut frames: Vec<i64> = (length - 3..length + 3).collect();
    for segment in course.segments() {
        frames.extend(segment.from - 3..segment.from + 3);
        frames.push(segment.from + (segment.to - segment.from) / 2);
    }
    frames.sort_unstable();
    frames.dedup();
    frames
}

/// The segments of a course: the frames and the speed of each.
fn segments(course: &Course) -> Vec<Value> {
    course
        .segments()
        .iter()
        .map(|segment| json!({ "from": segment.from, "to": segment.to, "speed": segment.speed }))
        .collect()
}

/// What the clip of a case shows of its media, with the frames that a transition shows of it
/// before its first and after its last.
fn handles(case: &Case) -> Vec<Value> {
    let scale = case.media.facts.duration.scale;
    [(0, 0), (3, 0), (0, 4), (2, 5), (8, 8)]
        .into_iter()
        .map(|(before, after)| {
            let shown = span::shown(&case.clip, before, after, case.rate, scale).unwrap();
            json!({
                "before": before,
                "after": after,
                "first": whole(shown.first),
                "last": whole(shown.last),
                "end": whole(shown.end),
            })
        })
        .collect()
}

/// The first frame in which each of a spread of places is on show: places from below where the
/// clip begins in its media to above all that it shows, those that the first frame and the
/// last of each segment show, and the tick after each of them.
fn ways_back(case: &Case, course: &Course) -> Value {
    let scale = case.media.facts.duration.scale;
    let span = course.span(case.rate, scale);
    let mut ticks: Vec<i64> = (-2..=span.min(60) + 2).collect();
    ticks.extend((0..=40).map(|part| span / 40 * part));
    ticks.extend([
        span - 1,
        span,
        span.saturating_add(1),
        span.saturating_add(2),
    ]);
    for segment in course.segments() {
        for frame in [segment.from, segment.to - 1] {
            let tick = course.offset(frame, case.rate, scale);
            ticks.extend([tick, tick.saturating_add(1)]);
        }
    }
    ticks.sort_unstable();
    ticks.dedup();
    let frames: Vec<Option<i64>> = ticks
        .iter()
        .map(|&tick| course.frame_of(tick, case.rate, scale))
        .collect();
    let ticks: Vec<Value> = ticks.into_iter().map(whole).collect();
    json!({ "ticks": ticks, "frames": frames })
}

/// The frame of the keys of the clip of a case at each of `frames`, counted from its first, and
/// the frames of its media that it shows from the lowest to the highest.
fn keys(case: &Case, project: &Project, frames: &[i64]) -> Value {
    let clock = KeyClock::of(&case.clip, project, case.rate);
    let frames: Vec<i64> = frames.iter().map(|k| case.clip.start + k).collect();
    let at: Vec<i64> = frames.iter().map(|&frame| clock.at(frame)).collect();
    json!({ "frames": frames, "at": at, "shown": clock.shown() })
}

/// A piece of sound as it is written: its samples of the timeline, the sample of the media
/// that its first is read from, and how fast it plays and which way.
fn piece(heard: &AudioPiece) -> Value {
    json!({
        "from": whole(heard.from),
        "to": whole(heard.to),
        "source_start": whole(heard.source_start),
        "speed": { "num": heard.speed.0, "den": heard.speed.1 },
        "reverse": heard.reverse,
    })
}

/// The pieces that are heard of the clip of a case in three windows of the samples of its
/// timeline: from before the clip to after it, from within it to after it, and within it.
fn windows(case: &Case, project: &Project) -> Vec<Value> {
    let clip = &case.clip;
    let begins = |frame: i64| samples_at(frame, case.rate, case.sample_rate);
    let after = begins(clip.end()) + 1000;
    let third = clip.length / 3;
    let within = begins(clip.start + third) + 7;
    [
        (begins(clip.start) - 1000, after),
        (within, after),
        (within, begins(clip.end() - third) + 3),
    ]
    .into_iter()
    .map(|(from, to)| {
        let pieces: Vec<Value> = audio_between(&project.timelines[0], project, from, to)
            .iter()
            .map(piece)
            .collect();
        json!({ "from": whole(from), "to": whole(to), "pieces": pieces })
    })
    .collect()
}

/// An end of a part of the sound as it is written: null where the part goes on without end.
fn edge(sample: i64) -> Value {
    if sample == i64::MIN || sample == i64::MAX {
        Value::Null
    } else {
        whole(sample)
    }
}

/// The samples of the timeline that the sound of a case is read at: about the first of the
/// clip and its last, 23 between them, and about each sample that a part begins at.
fn samples_of(case: &Case, sounding: &Sounding) -> Vec<i64> {
    let begins = |frame: i64| samples_at(frame, case.rate, case.sample_rate);
    let (first, end) = (begins(case.clip.start), begins(case.clip.end()));
    let mut samples = vec![first - 3, first, first + 1, end - 1, end, end + 4];
    samples.extend((1..24).map(|step| first + (end - first) / 24 * step));
    for part in sounding.parts() {
        for bound in [part.from, part.to] {
            if bound != i64::MIN && bound != i64::MAX {
                samples.extend([bound - 1, bound, bound + 1]);
            }
        }
    }
    samples.sort_unstable();
    samples.dedup();
    samples
}

/// The sound of the clip of a case: the parts of its segments, the pieces heard, and the
/// sample of the media that each of a spread of samples of the timeline is read from. A clip
/// of one speed is read as the engine reads it for a piece, at its speed.
fn sound(case: &Case, project: &Project, course: &Course) -> Value {
    let clip = &case.clip;
    let sounding = course.sounding(clip.start, case.rate, case.sample_rate);
    let begins = clip.source_in().unwrap();
    let media_rate = case.media.facts.audio.as_ref().unwrap().rate;
    let samples = samples_of(case, &sounding);
    let read: Vec<Value> = samples
        .iter()
        .map(|&sample| {
            whole(if clip.retime.is_some() {
                sounding.source_sample(begins, sample, media_rate)
            } else {
                span::source_sample(clip, case.rate, sample, case.sample_rate, media_rate)
            })
        })
        .collect();
    let parts: Vec<Value> = sounding
        .parts()
        .map(|part| json!({ "from": edge(part.from), "to": edge(part.to), "speed": part.speed }))
        .collect();
    let samples: Vec<Value> = samples.into_iter().map(whole).collect();
    json!({
        "parts": parts,
        "windows": windows(case, project),
        "samples": samples,
        "source_samples": read,
    })
}

/// The clip of a case from each of some frames on: 4 frames before its first, its second
/// frame, its middle, its last, and the frame of each of its speed points.
fn heads(case: &Case) -> Vec<Value> {
    let clip = &case.clip;
    let scale = case.media.facts.duration.scale;
    let mapping = clip.mapping();
    let mut from = vec![-4, 1, clip.length / 2, clip.length - 1];
    from.extend(mapping.points.iter().map(|point| point.frame));
    from.sort_unstable();
    from.dedup();
    from.into_iter()
        .map(|frame| {
            let trimmed = mapping
                .from_frame(frame, clip.length, case.rate, scale)
                .unwrap();
            json!({
                "from": frame,
                "points": trimmed.retime.points,
                "length": trimmed.length,
                "later": whole(trimmed.later),
            })
        })
        .collect()
}

/// The clip of a case made to last each of some lengths: a frame more than half of its own,
/// to the frame of its last speed point, a frame less than its own, and 5 frames more.
fn tails(case: &Case) -> Vec<Value> {
    let clip = &case.clip;
    let scale = case.media.facts.duration.scale;
    let mapping = clip.mapping();
    let last = mapping.points.last().map_or(0, |point| point.frame);
    let mut to = vec![clip.length / 2 + 1, last, clip.length - 1, clip.length + 5];
    to.sort_unstable();
    to.dedup();
    to.into_iter()
        .filter(|&length| length >= 1)
        .map(|length| {
            let trimmed = mapping
                .with_length(length, clip.length, case.rate, scale)
                .unwrap();
            json!({
                "to": length,
                "points": trimmed.retime.points,
                "length": trimmed.length,
                "later": whole(trimmed.later),
            })
        })
        .collect()
}

/// What the engine finds for a case, as it is written.
fn answers(case: &Case) -> Value {
    let clip = &case.clip;
    let scale = case.media.facts.duration.scale;
    let project = project_of(case);
    let course = clip.mapping().course(clip.length).unwrap();
    let frames = frames_of(&course);
    let ticks: Vec<Value> = frames
        .iter()
        .map(|&k| whole(span::source_tick(clip, k, case.rate, scale)))
        .collect();
    json!({
        "name": case.name,
        "clip": clip,
        "media": case.media,
        "rate": case.rate,
        "sample_rate": case.sample_rate,
        "unit": course.unit(),
        "segments": segments(&course),
        "frames": frames,
        "ticks": ticks,
        "span": whole(span::source_span(clip, case.rate, scale)),
        "shown": handles(case),
        "frame_of": ways_back(case, &course),
        "speed_curve": course.speed_curve(),
        "frame_curve": course.frame_curve(),
        "keys": keys(case, &project, &frames),
        "sound": sound(case, &project, &course),
        "from_frame": heads(case),
        "with_length": tails(case),
    })
}

fn written() -> String {
    let lines: Vec<String> = cases()
        .iter()
        .map(|case| answers(case).to_string())
        .collect();
    format!("[\n{}\n]\n", lines.join(",\n"))
}

#[test]
fn the_numbers_of_speed_points_are_written_as_the_engine_finds_them() {
    let text = written();
    if std::env::var("COLUMBINA_WRITE_EXAMPLES").as_deref() == Ok("1") {
        std::fs::write(FILE, &text).unwrap();
        return;
    }
    let kept = std::fs::read_to_string(Path::new(FILE)).unwrap_or_default();
    let read = |text: &str| serde_json::from_str::<Value>(text).ok();
    assert!(
        read(&kept).is_some() && read(&kept) == read(&text),
        "{FILE} is not as the engine finds the numbers of speed points now: run the tests of \
         columbina-model with COLUMBINA_WRITE_EXAMPLES=1 to write it anew"
    );
}
