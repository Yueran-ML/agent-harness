//! The frames of the keys of clips' grades, as [`KeyClock`] finds them from frames of their
//! timelines, written out for the copies of the project that clients keep: an editor puts a key
//! at the frame that the engine reads it at (PROTOCOL.md, "Frames of keys").
//!
//! The file `tests/data/key-clocks.json` is written anew when `COLUMBINA_WRITE_EXAMPLES=1` is
//! set; otherwise it is checked to be as the engine finds the frames now. Each case is a clip as
//! the engine writes it, its media (or none), the rate of its timeline, and the frames of the
//! keys at frames of the timeline from a little before the clip to a little after it.

use std::path::Path;

use columbina_model::keyframes::KeyClock;
use columbina_model::{
    Clip, Media, Project, Rate, Rgba, Source, Speed, TextAlign, Time, TitleStyle,
};

const FILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/key-clocks.json");

/// Media like the sample's: a picture of `rate`, `seconds` long, in ticks of `scale`.
fn media(id: &str, rate: Rate, scale: u32, ticks: i64, still: bool) -> Media {
    let mut media = Project::sample().media[0].clone();
    media.id = id.into();
    let video = media.facts.video.as_mut().unwrap();
    video.rate = rate;
    video.still = still;
    media.facts.duration = Time { ticks, scale };
    media
}

/// A clip of `media` from tick `begins`, at `start` for `length` frames, at `speed`, backward
/// where `reverse` is.
fn cut(media: &Media, begins: i64, start: i64, length: i64, speed: Speed, reverse: bool) -> Clip {
    let at = Time {
        ticks: begins,
        scale: media.facts.duration.scale,
    };
    let mut clip = Clip::of_media("c1", media.id.clone(), at, start, length);
    clip.speed = speed;
    clip.reverse = reverse;
    clip
}

const fn rate(num: u32, den: u32) -> Rate {
    Rate { num, den }
}

const fn speed(num: u32, den: u32) -> Speed {
    Speed { num, den }
}

/// The cases: (name, clip, its media, the rate of the timeline).
#[allow(clippy::too_many_lines, reason = "a case a line or two")]
fn cases() -> Vec<(&'static str, Clip, Option<Media>, Rate)> {
    let pal = media("m1", rate(25, 1), 15360, 15360 * 10, false);
    let film = media("m2", rate(24000, 1001), 24000, 24000 * 8, false);
    let fifty = media("m3", rate(50, 1), 90000, 90000 * 6, false);
    let ntsc = media("m4", rate(30000, 1001), 30000, 30000 * 5 + 7, false);
    let still = media("m5", rate(25, 1), 15360, 15360 * 5, true);
    let mut sound = media("m6", rate(25, 1), 48000, 48000 * 10, false);
    sound.facts.video = None;
    let style = TitleStyle {
        font: String::new(),
        size: 40.0.into(),
        color: Rgba([255; 4]),
        x: 0.5.into(),
        y: 0.5.into(),
        align: TextAlign::Center,
        stroke: None,
        shadow: None,
        background: None,
        spans: Vec::new(),
    };
    let title = Clip::new(
        "c1",
        Source::Title {
            text: "Hi".into(),
            style,
        },
        40,
        20,
    );
    let colour = Clip::new(
        "c1",
        Source::Color {
            color: Rgba([0, 0, 0, 255]),
        },
        7,
        12,
    );
    vec![
        (
            "media at the timeline's rate",
            cut(&pal, 15360 / 25 * 20, 100, 50, speed(1, 1), false),
            Some(pal.clone()),
            rate(25, 1),
        ),
        (
            "25p media on a timeline of 29.97",
            cut(&pal, 15360 * 2, 30, 40, speed(1, 1), false),
            Some(pal.clone()),
            rate(30000, 1001),
        ),
        (
            "23.976p media from a tick between frames on a timeline of 25",
            cut(&film, 12345, 0, 60, speed(1, 1), false),
            Some(film.clone()),
            rate(25, 1),
        ),
        (
            "twice as fast, backward",
            cut(&pal, 15360, 10, 30, speed(2, 1), true),
            Some(pal.clone()),
            rate(25, 1),
        ),
        (
            "half as fast",
            cut(&pal, 15360 / 25 * 7, 5, 40, speed(1, 2), false),
            Some(pal.clone()),
            rate(25, 1),
        ),
        (
            "50p media at three halves, backward, on a timeline of 59.94",
            cut(&fifty, 90000 + 1800 * 3, 200, 45, speed(3, 2), true),
            Some(fifty.clone()),
            rate(60000, 1001),
        ),
        (
            "29.97p media at seven thirds on a timeline of 24",
            cut(&ntsc, 1001 * 4, 12, 25, speed(7, 3), false),
            Some(ntsc.clone()),
            rate(24, 1),
        ),
        (
            "near the end of its media, kept to its last frame",
            cut(&pal, 15360 * 10 - 15360 / 25 * 3, 0, 3, speed(1, 1), false),
            Some(pal.clone()),
            rate(25, 1),
        ),
        (
            "a still",
            cut(&still, 0, 60, 20, speed(1, 1), false),
            Some(still),
            rate(25, 1),
        ),
        (
            "media without a picture",
            cut(&sound, 48000, 20, 15, speed(1, 1), false),
            Some(sound),
            rate(25, 1),
        ),
        ("a title", title, None, rate(25, 1)),
        ("a colour", colour, None, rate(30, 1)),
    ]
}

fn written() -> String {
    let lines: Vec<String> = cases()
        .into_iter()
        .map(|(name, clip, media, rate)| {
            let mut project = Project::sample();
            project.media.extend(media.iter().cloned());
            let clock = KeyClock::of(&clip, &project, rate);
            let frames: Vec<i64> = (clip.start - 3..clip.start + clip.length + 3).collect();
            let keys: Vec<i64> = frames.iter().map(|&frame| clock.at(frame)).collect();
            let case = serde_json::json!({
                "name": name,
                "clip": clip,
                "media": media,
                "rate": rate,
                "frames": frames,
                "keys": keys,
            });
            serde_json::to_string(&case).unwrap()
        })
        .collect();
    format!("[\n{}\n]\n", lines.join(",\n"))
}

#[test]
fn the_frames_of_keys_are_written_as_the_engine_finds_them() {
    let text = written();
    if std::env::var("COLUMBINA_WRITE_EXAMPLES").as_deref() == Ok("1") {
        std::fs::write(FILE, &text).unwrap();
        return;
    }
    let kept = std::fs::read_to_string(Path::new(FILE)).unwrap_or_default();
    let read = |text: &str| serde_json::from_str::<serde_json::Value>(text).ok();
    assert!(
        read(&kept).is_some() && read(&kept) == read(&text),
        "{FILE} is not as the engine finds the frames of keys now: run the tests of \
         columbina-model with COLUMBINA_WRITE_EXAMPLES=1 to write it anew"
    );
}
