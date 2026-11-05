//! A stretched clip (T13) in playback (T5): what the device is given is its export to the bit,
//! read ahead by the readers of playback; and a graph swapped in while it plays is brought to
//! where it is swapped in before it is sent, sounding as the one it takes over from.

mod session;

use columbina_audio::graph::{Graph, Options, Purpose};
use columbina_model::{ClipPitch, Project, Speed};

use session::{one_track, Sim, Tone, RATE};

const TONE: Tone = Tone {
    hz: 440.0,
    amplitude: 0.5,
};

/// [`one_track`] with its clip at `speed`, Pitch Correction as `corrected`, its pitch moved by
/// `semitones`, played backward as `reverse`.
fn stretched(speed: (u32, u32), corrected: bool, semitones: i32, reverse: bool) -> Project {
    let mut project = one_track();
    let clip = &mut project.timelines[0].tracks[0].clips[0];
    clip.speed = Speed {
        num: speed.0,
        den: speed.1,
    };
    clip.pitch_correction = corrected;
    clip.pitch = ClipPitch {
        semitones,
        cents: 0.0,
    };
    clip.reverse = reverse;
    project
}

/// The left side of `project` exported from 0 for `frames` samples, its clips sounding `TONE`.
fn exported(project: &Project, frames: usize) -> Vec<f32> {
    let mut graph = Graph::build(project, 0, Options::new(4096, Purpose::Export)).unwrap();
    let mut tone = TONE;
    graph.start(0, &mut tone).unwrap();
    let mut left = Vec::with_capacity(frames);
    while left.len() < frames {
        let n = (frames - left.len()).min(4096);
        graph.process(&mut tone, n).unwrap();
        left.extend_from_slice(&graph.output(graph.monitor().unwrap()).channel(0)[..n]);
    }
    left
}

#[test]
fn a_stretched_clip_plays_as_it_exports_to_the_bit() {
    // Across the points where the stretch starts again (at 0.8: 5 s and 10 s).
    for project in [
        stretched((4, 5), true, 0, false),
        stretched((3, 2), true, 2, true),
        stretched((1, 1), false, 3, false),
    ] {
        let mut sim = Sim::of(&project, 0, TONE);
        for _ in 0..1_250 {
            sim.tick();
        }
        let played = sim.left();
        let frames = 12 * RATE as usize;
        let want = exported(&project, frames);
        assert!(want.iter().any(|s| s.abs() > 0.1));
        let differs = played[..frames]
            .iter()
            .zip(&want)
            .position(|(a, b)| a.to_bits() != b.to_bits());
        assert_eq!(
            differs, None,
            "the first sample played otherwise than exported"
        );
        assert_eq!(sim.counters.underruns(), 0);
        assert_eq!(sim.control.shared.short(), 0);
    }
}

#[test]
fn a_clip_with_effects_plays_as_it_exports_to_the_bit() {
    // Its media read as far ahead as its effects are late (T4c): a Pitch and a Limiter, on a clip
    // as it is and on one stretched, from 1 s.
    use columbina_model::audio::fx::{LimiterFx, PitchFx};
    use columbina_model::{Effect, Insert};
    for project in [
        stretched((1, 1), false, 0, false),
        stretched((4, 5), true, 0, false),
        stretched((3, 2), false, 2, true),
    ] {
        let mut project = project;
        let clip = &mut project.timelines[0].tracks[0].clips[0];
        clip.start = 25;
        clip.effects = [
            Effect::Pitch(PitchFx {
                semitones: -5,
                ..PitchFx::default()
            }),
            Effect::Limiter(LimiterFx {
                input_db: 6.0,
                ..LimiterFx::default()
            }),
        ]
        .into_iter()
        .map(|effect| Insert {
            id: String::new(),
            on: true,
            effect,
        })
        .collect();
        let mut sim = Sim::of(&project, 0, TONE);
        for _ in 0..1_250 {
            sim.tick();
        }
        let played = sim.left();
        let frames = 12 * RATE as usize;
        let want = exported(&project, frames);
        assert!(want[RATE as usize..].iter().any(|s| s.abs() > 0.1));
        let differs = played[..frames]
            .iter()
            .zip(&want)
            .position(|(a, b)| a.to_bits() != b.to_bits());
        assert_eq!(
            differs, None,
            "the first sample played otherwise than exported"
        );
        assert_eq!(sim.counters.underruns(), 0);
        assert_eq!(sim.control.shared.short(), 0);
    }
}

#[test]
fn a_graph_swapped_in_inside_a_segment_sounds_as_the_one_it_takes_over_from() {
    // At 6 s, a second into the segment that starts at 5 s.
    let project = stretched((4, 5), true, 0, false);
    let mut reference = Sim::of(&project, 0, TONE);
    let mut swapped = Sim::of(&project, 0, TONE);
    for tick in 0..900 {
        if tick == 600 {
            let at = swapped.swap(&project, 2_400);
            assert!(at > 5 * i64::from(RATE));
        }
        reference.tick();
        swapped.tick();
    }
    assert_eq!(swapped.counters.swaps(), 1);
    let (want, got) = (reference.left(), swapped.left());
    let most = want
        .iter()
        .zip(&got)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0, f32::max);
    assert!(most < 1e-6, "{most}");
    assert_eq!(swapped.counters.underruns(), 0);
    assert_eq!(swapped.control.shared.short(), 0);
}
