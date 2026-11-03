//! The sound of a project of schema 3 is heard the same, sample for sample, once it is upgraded:
//! for 200 random projects (see `columbina_model::samples`), what `resolve::audio_between` gives,
//! piece by piece, with its gain at its points and at every seventh sample, every sample about
//! each point and the bits of every gain and pan, is the same as it was before schema 4.
//!
//! What it was is kept in `tests/data/schema3-sound.txt`, a digest a project, written by the
//! engine of schema 3. It is written anew when `COLUMBINA_WRITE_SOUND=1` is set, which is only
//! ever right on that engine.

use columbina_model::resolve::audio_between;
use columbina_model::samples::schema3_project;
use columbina_model::Project;

const FILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/schema3-sound.txt");

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

fn written() -> String {
    (0..PROJECTS)
        .map(|seed| {
            let (project, _) = Project::from_value_upgrading(schema3_project(seed)).unwrap();
            format!("{seed} {:016x}\n", sound_of(&project))
        })
        .collect()
}

#[test]
fn the_sound_of_a_project_of_schema_3_is_the_same_once_it_is_upgraded() {
    let text = written();
    if std::env::var("COLUMBINA_WRITE_SOUND").as_deref() == Ok("1") {
        std::fs::write(FILE, &text).unwrap();
        return;
    }
    let kept = std::fs::read_to_string(FILE).unwrap().replace("\r\n", "\n");
    for (now, was) in text.lines().zip(kept.lines()) {
        assert_eq!(now, was, "the sound of the project of this seed changed");
    }
    assert_eq!(text.lines().count(), kept.lines().count());
}

#[test]
fn the_projects_hold_what_the_upgrade_has_to_keep() {
    // Over the projects: tracks off and on, clips off, backward and at other speeds, fades, and
    // crossfades, so that the digests cover them.
    let (mut off, mut backward, mut faster, mut faded, mut crossfades) = (0, 0, 0, 0, 0);
    for seed in 0..PROJECTS {
        let project = schema3_project(seed);
        for track in project["timelines"][0]["tracks"].as_array().unwrap() {
            off += usize::from(track["enabled"] == false);
            crossfades += track["transitions"].as_array().unwrap().len();
            for clip in track["clips"].as_array().unwrap() {
                backward += usize::from(clip["reverse"] == true);
                faster += usize::from(clip["speed"]["num"] != clip["speed"]["den"]);
                faded += usize::from(clip["fade_in"] != 0 || clip["fade_out"] != 0);
            }
        }
    }
    for (what, count) in [
        ("tracks off", off),
        ("clips backward", backward),
        ("clips at other speeds", faster),
        ("clips that fade", faded),
        ("crossfades", crossfades),
    ] {
        assert!(count >= 20, "only {count} {what}");
    }
}
