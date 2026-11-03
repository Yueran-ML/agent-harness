//! Projects to test with, made from a seed: random projects of schema 3, as the engine of the
//! Color page wrote them, with the sound that an editor makes of them.
//!
//! [`schema3_project`] gives a project of schema 3 as JSON, so that it is read as one of that
//! schema is, through [`crate::upgrade`]. It has the things whose sound the upgrade to schema 4
//! must keep, sample for sample: tracks of sound that are on and off, clips of media of one, two
//! and six channels at several rates of sound, on and off, at several speeds, forward and
//! backward, at a gain and a pan, fading in and out, and crossfades between them, aligned every
//! way; on timelines of several rates of frames and of sound. The same seed always gives the same
//! project. They are for the tests of this crate and of the crates that play and render a
//! project, which compare its sound before and after the upgrade.

use serde_json::{json, Value};

use crate::project::Project;

/// A small generator of numbers, `SplitMix64`, so that a seed gives the same project on every
/// machine without a crate for it.
struct Numbers(u64);

impl Numbers {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// A number from `low` to `high`, both in.
    fn within(&mut self, low: i64, high: i64) -> i64 {
        let span = u64::try_from(high - low + 1).expect("a range that is not empty");
        low + i64::try_from(self.next() % span).expect("a span of an i64")
    }

    /// True `percent` times in a hundred.
    fn chance(&mut self, percent: i64) -> bool {
        self.within(0, 99) < percent
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        let last = i64::try_from(items.len()).expect("a short list") - 1;
        &items[usize::try_from(self.within(0, last)).expect("a place in the list")]
    }

    /// A number from `low` to `high`, on a grid of 1/64, so that it is written and read exactly.
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        reason = "small whole numbers"
    )]
    fn fraction(&mut self, low: f64, high: f64) -> f64 {
        let steps = ((high - low) * 64.0) as i64;
        low + self.within(0, steps) as f64 / 64.0
    }
}

/// The media of the projects: its id, channels, rate of sound, the scale of its places, and
/// whether it has a picture.
const MEDIA: [(&str, u16, u32, u32, bool); 4] = [
    ("m1", 2, 48000, 15360, true),
    ("m2", 1, 44100, 44100, false),
    ("m3", 6, 48000, 48000, false),
    ("m4", 2, 44100, 90000, true),
];

/// How long each media lasts, in seconds.
const MEDIA_SECONDS: i64 = 600;

/// A random project of schema 3 made from `seed`, as JSON: one that is valid, so that it is read
/// (and upgraded) whole. See the module.
///
/// # Panics
/// Never for a seed: a project that would not be valid is made again from the next numbers, and
/// one is found within a few tries.
#[must_use]
pub fn schema3_project(seed: u64) -> Value {
    let mut numbers = Numbers(seed.wrapping_mul(0x2545_f491_4f6c_dd1d) ^ 0x5eed);
    for _ in 0..1000 {
        let project = attempt(&mut numbers);
        if Project::from_value_upgrading(project.clone()).is_ok() {
            return project;
        }
    }
    panic!("no valid project of schema 3 was made from seed {seed}");
}

fn attempt(numbers: &mut Numbers) -> Value {
    let media: Vec<Value> = MEDIA
        .iter()
        .map(|&(id, channels, rate, scale, picture)| {
            let video = picture.then(|| {
                json!({
                    "width": 1920, "height": 1080, "rate": { "num": 25, "den": 1 },
                    "codec": "h264", "bits": 8, "rotation": 0,
                })
            });
            json!({
                "id": id,
                "path": format!("C:/media/{id}.mov"),
                "identity": { "size": 1000, "modified_us": 0, "digest": "0".repeat(64) },
                "facts": {
                    "duration": { "ticks": MEDIA_SECONDS * i64::from(scale), "scale": scale },
                    "video": video,
                    "audio": { "rate": rate, "channels": channels, "codec": "pcm_s16le" },
                },
                "name": format!("{id}.mov"),
                "bin": null,
                "mark_in": null,
                "mark_out": null,
            })
        })
        .collect();
    let rate = *numbers.pick(&[(25, 1), (24000, 1001), (30, 1), (50, 1)]);
    let sample_rate = *numbers.pick(&[48000, 44100, 96000]);
    let mut tracks = vec![];
    let mut next = 1;
    if numbers.chance(50) {
        let clip = clip_of(numbers, &mut next, "m1", 0);
        tracks.push(json!({
            "id": "V1", "kind": "video", "name": "V1", "enabled": true, "locked": false,
            "clips": [clip], "transitions": [],
        }));
    }
    for track in 1..=numbers.within(1, 3) {
        tracks.push(audio_track(numbers, &mut next, track));
    }
    json!({
        "schema": 3,
        "next_id": 1000,
        "bins": [],
        "media": media,
        "timelines": [{
            "id": "t1",
            "name": "Timeline 1",
            "rate": { "num": rate.0, "den": rate.1 },
            "width": 1920,
            "height": 1080,
            "sample_rate": sample_rate,
            "tracks": tracks,
        }],
    })
}

/// A track of sound `A<number>`, on 3 times in 4, with clips one after another and crossfades
/// at some of the cuts where they meet.
fn audio_track(numbers: &mut Numbers, next: &mut u64, number: i64) -> Value {
    let mut clips: Vec<Value> = vec![];
    let mut start = numbers.within(0, 30);
    for _ in 0..numbers.within(1, 4) {
        let media = MEDIA[usize::try_from(numbers.within(0, 3)).expect("a place")].0;
        let clip = clip_of(numbers, next, media, start);
        start =
            clip["start"].as_i64().expect("a start") + clip["length"].as_i64().expect("a length");
        clips.push(clip);
        if numbers.chance(40) {
            start += numbers.within(1, 20);
        }
    }
    let mut transitions = vec![];
    for at in 1..clips.len() {
        let (left_end, right_start) = (
            clips[at - 1]["start"].as_i64().expect("a start")
                + clips[at - 1]["length"].as_i64().expect("a length"),
            clips[at]["start"].as_i64().expect("a start"),
        );
        if left_end != right_start || !numbers.chance(70) {
            continue;
        }
        let shortest = clips[at - 1]["length"]
            .as_i64()
            .expect("a length")
            .min(clips[at]["length"].as_i64().expect("a length"));
        let length = numbers.within(1, (shortest / 2).max(1));
        let align = *numbers.pick(&["center", "start", "end"]);
        clips[at - 1]["fade_out"] = 0.into();
        clips[at]["fade_in"] = 0.into();
        transitions.push(json!({
            "id": format!("x{}", made(next)),
            "kind": "crossfade",
            "left": clips[at - 1]["id"],
            "right": clips[at]["id"],
            "length": length,
            "align": align,
        }));
    }
    let id = format!("A{number}");
    json!({
        "id": id, "kind": "audio", "name": id, "enabled": numbers.chance(75), "locked": false,
        "clips": clips, "transitions": transitions,
    })
}

/// A clip of `media` at `start`, from a place well inside it.
fn clip_of(numbers: &mut Numbers, next: &mut u64, media: &str, start: i64) -> Value {
    let &(_, _, _, scale, _) = MEDIA
        .iter()
        .find(|m| m.0 == media)
        .expect("one of the media");
    let scale = i64::from(scale);
    let source_in = numbers.within(60 * scale, 120 * scale);
    let length = numbers.within(4, 80);
    let speed = *numbers.pick(&[(1, 1), (1, 1), (1, 2), (3, 2), (2, 1), (4, 5)]);
    let gain_db = if numbers.chance(30) {
        0.0
    } else {
        numbers.fraction(-30.0, 12.0)
    };
    let pan = match numbers.within(0, 5) {
        0 | 1 => 0.0,
        2 => -1.0,
        3 => 1.0,
        _ => numbers.fraction(-1.0, 1.0),
    };
    let fade_in = if numbers.chance(40) {
        numbers.within(1, length / 3 + 1)
    } else {
        0
    };
    let fade_out = if numbers.chance(40) {
        numbers.within(0, (length - fade_in) / 2)
    } else {
        0
    };
    json!({
        "id": format!("c{}", made(next)),
        "start": start,
        "length": length,
        "source": { "kind": "media", "media": media, "in": { "ticks": source_in, "scale": scale } },
        "enabled": numbers.chance(85),
        "speed": { "num": speed.0, "den": speed.1 },
        "reverse": numbers.chance(20),
        "link": null,
        "transform": { "x": 0.0, "y": 0.0, "scale": 1.0, "rotation": 0.0, "opacity": 1.0 },
        "gain_db": gain_db,
        "pan": pan,
        "fade_in": fade_in,
        "fade_out": fade_out,
    })
}

/// The number of the next id, below the project's `next_id` of 1000.
fn made(next: &mut u64) -> u64 {
    let number = *next;
    *next += 1;
    number
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_seed_gives_the_same_valid_project_every_time() {
        for seed in 0..20 {
            let project = schema3_project(seed);
            assert_eq!(project, schema3_project(seed));
            assert_eq!(project["schema"], 3);
            assert!(Project::from_value_upgrading(project).is_ok());
        }
        assert_ne!(schema3_project(1), schema3_project(2));
    }
}
