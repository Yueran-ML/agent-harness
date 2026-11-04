//! The sound of a project of an earlier schema is heard the same, sample for sample, once it is
//! upgraded: for 200 random projects of schema 3 (see `columbina_model::samples`), what
//! `resolve::audio_between` gives, piece by piece, with its gain at its points and at every
//! seventh sample, every sample about each point and the bits of every gain and pan, is the
//! same as it was before schema 4. So it is for the same projects written in schema 2, which
//! is read through schema 3, and in schema 1.
//!
//! What it was is kept in `tests/data/schema3-sound.txt` and `tests/data/schema1-sound.txt`, a
//! digest a project, written by the engine of schema 3. They are written anew when
//! `COLUMBINA_WRITE_SOUND=1` is set, which is only ever right on that engine.

use std::fmt::Write;

use columbina_model::resolve::audio_between;
use columbina_model::samples::{schema1_project, schema2_project, schema3_project};
use columbina_model::Project;

const FILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/schema3-sound.txt");
const FILE_1: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/schema1-sound.txt");

/// How many projects are compared.
const PROJECTS: u64 = 200;

/// FNV-1a of 64 bits, over what is fed to it.
struct Digest(u64);

impl Digest {
    fn feed(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }

    fn number(&mut self, number: i64) {
        self.feed(&number.to_le_bytes());
    }
}

/// The digest of what is heard on the first timeline of a project, from its first sample to its
/// last.
fn sound_of(project: &Project) -> u64 {
    let mut digest = Digest(0xcbf2_9ce4_8422_2325);
    let timeline = &project.timelines[0];
    let pieces = audio_between(timeline, project, 0, i64::from(u32::MAX));
    digest.number(i64::try_from(pieces.len()).unwrap());
    for piece in &pieces {
        digest.feed(piece.clip.as_bytes());
        digest.feed(piece.track.as_bytes());
        digest.feed(piece.media.as_bytes());
        for number in [piece.from, piece.to, piece.source_start] {
            digest.number(number);
        }
        digest.number(i64::from(piece.speed.0));
        digest.number(i64::from(piece.speed.1));
        digest.number(i64::from(piece.reverse));
        digest.number(i64::from(piece.pan.to_bits()));
        let mut samples: Vec<i64> = (piece.from..piece.to).step_by(7).collect();
        for &(sample, gain) in &piece.gain.points {
            digest.number(sample);
            digest.number(i64::from(gain.to_bits()));
            samples.extend(sample - 3..=sample + 3);
        }
        samples.retain(|sample| (piece.from..piece.to).contains(sample));
        samples.sort_unstable();
        samples.dedup();
        for sample in samples {
            digest.number(sample);
            digest.number(i64::from(piece.gain.at(sample).to_bits()));
        }
    }
    digest.0
}

/// The digest of each project that `make` gives, a line a seed; `-` for one that is not valid.
fn written(make: fn(u64) -> serde_json::Value) -> String {
    let mut text = String::new();
    for seed in 0..PROJECTS {
        match Project::from_value_upgrading(make(seed)) {
            Ok((project, _)) => writeln!(text, "{seed} {:016x}", sound_of(&project)).unwrap(),
            Err(_) => writeln!(text, "{seed} -").unwrap(),
        }
    }
    text
}

/// Compare what is heard now with what was, or write it where `COLUMBINA_WRITE_SOUND=1`.
fn compare(text: &str, file: &str) {
    if std::env::var("COLUMBINA_WRITE_SOUND").as_deref() == Ok("1") {
        std::fs::write(file, text).unwrap();
        return;
    }
    let kept = std::fs::read_to_string(file).unwrap().replace("\r\n", "\n");
    for (now, was) in text.lines().zip(kept.lines()) {
        assert_eq!(now, was, "the sound of the project of this seed changed");
    }
    assert_eq!(text.lines().count(), kept.lines().count());
}

#[test]
fn the_sound_of_a_project_of_schema_3_is_the_same_once_it_is_upgraded() {
    let text = written(schema3_project);
    assert!(!text.contains('-'), "every project of schema 3 is valid");
    compare(&text, FILE);
}

#[test]
fn the_sound_of_a_project_of_schema_2_is_that_of_the_same_project_of_schema_3() {
    assert_eq!(written(schema2_project), written(schema3_project));
}

#[test]
fn the_sound_of_a_project_of_schema_1_is_the_same_once_it_is_upgraded() {
    let text = written(schema1_project);
    let valid = text.lines().filter(|line| !line.ends_with('-')).count();
    assert!(valid >= 150, "only {valid} projects of schema 1 are valid");
    compare(&text, FILE_1);
}

#[test]
fn the_projects_hold_what_the_upgrade_has_to_keep() {
    // Over the projects: tracks off and on, clips off, backward and at other speeds, fades,
    // fades as long together as their clip, clips from the start and to the end of their
    // media, and crossfades, some longer than half the shorter of their clips.
    let mut counts = [0; 9];
    for seed in 0..PROJECTS {
        let project = schema3_project(seed);
        for track in project["timelines"][0]["tracks"].as_array().unwrap() {
            let clips = track["clips"].as_array().unwrap();
            counts[0] += usize::from(track["enabled"] == false);
            for clip in clips {
                counts[1] += usize::from(clip["reverse"] == true);
                counts[2] += usize::from(clip["speed"]["num"] != clip["speed"]["den"]);
                counts[3] += usize::from(clip["fade_in"] != 0 || clip["fade_out"] != 0);
                let fades = clip["fade_in"].as_i64().unwrap() + clip["fade_out"].as_i64().unwrap();
                counts[4] += usize::from(fades == clip["length"].as_i64().unwrap());
                counts[5] += usize::from(clip["source"]["in"]["ticks"] == 0);
                let place = &clip["source"]["in"];
                let scale = place["scale"].as_i64().unwrap();
                counts[6] += usize::from(place["ticks"].as_i64().unwrap() > 500 * scale);
            }
            for x in track["transitions"].as_array().unwrap() {
                counts[7] += 1;
                let length = |id: &serde_json::Value| {
                    clips.iter().find(|c| c["id"] == *id).unwrap()["length"]
                        .as_i64()
                        .unwrap()
                };
                let shortest = length(&x["left"]).min(length(&x["right"]));
                counts[8] += usize::from(x["length"].as_i64().unwrap() * 2 > shortest);
            }
        }
    }
    let names = [
        "tracks off",
        "clips backward",
        "clips at other speeds",
        "clips that fade",
        "clips whose fades meet",
        "clips from the start of their media",
        "clips to the end of their media",
        "crossfades",
        "crossfades over half a clip",
    ];
    for (name, count) in names.iter().zip(counts) {
        assert!(count >= 5, "only {count} {name}");
    }
}
