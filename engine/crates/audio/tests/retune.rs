//! A change of the mix that the graph takes as it plays (plan T5, `graph::retune`): the settings
//! that it has controls of come out as the values to set them to, and set so the graph sounds as
//! one built from the changed project; everything else needs another graph.

use columbina_audio::graph::{retune, without_clip_levels, Graph, Options, Purpose};
use columbina_audio::testing::Rng;
use columbina_model::audio::fx::GainFx;
use columbina_model::{Effect, Insert, Lane, LaneKind, LanePoint, Project, Time, TrackKind, Vca};

/// Three stereo tracks out to B1, A3 also sending to B2, which goes out to B1; a VCA over A2;
/// the equalizer of A1 and the Dynamics of A2 on.
fn project() -> Project {
    let mut project = Project::sample();
    let timeline = &mut project.timelines[0];
    timeline
        .tracks
        .retain(|track| track.kind == TrackKind::Audio);
    let first = timeline.tracks[0].clone();
    timeline.tracks = (1..=3)
        .map(|n| {
            let mut track = first.clone();
            track.id = format!("A{n}");
            track.clips[0].id = format!("c{n}");
            track
        })
        .collect();
    let mut b2 = columbina_model::Bus::first();
    b2.id = "B2".into();
    b2.strip.outputs = vec![columbina_model::Output {
        bus: "B1".into(),
        on: true,
    }];
    timeline.buses.push(b2);
    timeline.tracks[2]
        .audio
        .strip
        .sends
        .push(columbina_model::Send {
            bus: "B2".into(),
            on: true,
            pre: false,
            level_db: -6.0,
            pan: columbina_model::Pan::default(),
        });
    timeline.tracks[0].audio.strip.eq.on = true;
    timeline.tracks[1].audio.strip.dynamics.on = true;
    timeline.tracks[1].audio.strip.dynamics.compressor.on = true;
    timeline.tracks[1].audio.strip.vcas = vec!["VCA1".into()];
    timeline.vcas.push(Vca {
        id: "VCA1".into(),
        name: "VCA 1".into(),
        fader_db: 0.0,
        mute: false,
        solo: false,
        vcas: vec![],
        lanes: columbina_model::Lanes::new(),
    });
    timeline.tracks[0].audio.strip.inserts.push(Insert {
        id: "fx1".into(),
        on: true,
        effect: Effect::Gain(GainFx { gain_db: 0.0 }),
    });
    project
}

/// A change of a project, named.
type Change = (&'static str, Box<dyn Fn(&mut Project)>);

fn built(project: &Project) -> Graph {
    Graph::build(project, 0, Options::new(4096, Purpose::Preview)).unwrap()
}

/// 9600 samples of the monitor from sample 0, the clips sounding noise, with `sets` set first.
fn played(graph: &mut Graph, sets: &[(columbina_audio::graph::Control, f32)]) -> Vec<f32> {
    let mut feed = |clip: usize,
                    range: std::ops::Range<i64>,
                    out: &mut [&mut [f32]]|
     -> Result<(), columbina_audio::feed::FeedError> {
        for (channel, out) in out.iter_mut().enumerate() {
            for (n, sample) in out.iter_mut().enumerate() {
                let at = range.start + i64::try_from(n).unwrap();
                let seed = u64::try_from(at.rem_euclid(10_007)).unwrap()
                    + 7 * u64::try_from(clip + channel).unwrap();
                *sample = Rng::new(seed).signed() * 0.3;
            }
        }
        Ok(())
    };
    graph.start(0, &mut feed).unwrap();
    for &(control, value) in sets {
        graph.set(control, value);
    }
    let monitor = graph.monitor().unwrap();
    let mut left = Vec::new();
    for _ in 0..4 {
        graph.process(&mut feed, 2400).unwrap();
        left.extend_from_slice(&graph.output(monitor).channel(0)[..2400]);
    }
    left
}

/// Whether the graph of `project`, retuned to `changed`, sounds as a graph of `changed` once its
/// ramps are done; the settings it set.
fn retuned_sounds_as_built(project: &Project, changed: &Project) -> usize {
    let mut graph = built(project);
    let sets = retune(
        &project.timelines[0],
        &changed.timelines[0],
        &graph.controls(),
    )
    .expect("a change of settings");
    let got = played(&mut graph, &sets);
    let want = played(&mut built(changed), &[]);
    let most = got[960..]
        .iter()
        .zip(&want[960..])
        .map(|(a, b)| (a - b).abs())
        .fold(0.0, f32::max);
    assert!(most < 1e-5, "{most}");
    sets.len()
}

#[test]
fn settings_with_controls_are_set_as_the_graph_plays_and_sound_as_built() {
    let base = project();
    let changes: Vec<Change> = vec![
        (
            "fader",
            Box::new(|p| p.timelines[0].tracks[0].audio.strip.fader_db = -7.5),
        ),
        (
            "pan",
            Box::new(|p| p.timelines[0].tracks[0].audio.strip.pan.x = 0.4),
        ),
        (
            "trim",
            Box::new(|p| p.timelines[0].tracks[2].audio.strip.trim_db = 3.0),
        ),
        (
            "mute",
            Box::new(|p| p.timelines[0].tracks[2].audio.mute = true),
        ),
        (
            "send",
            Box::new(|p| p.timelines[0].tracks[2].audio.strip.sends[0].level_db = -20.0),
        ),
        (
            "eq gain",
            Box::new(|p| p.timelines[0].tracks[0].audio.strip.eq.bands[2].gain_db = 6.0),
        ),
        (
            "threshold",
            Box::new(|p| {
                p.timelines[0].tracks[1]
                    .audio
                    .strip
                    .dynamics
                    .compressor
                    .threshold_db = -30.0;
            }),
        ),
        ("vca", Box::new(|p| p.timelines[0].vcas[0].fader_db = -4.0)),
        (
            "bus",
            Box::new(|p| p.timelines[0].buses[1].strip.fader_db = -3.0),
        ),
        (
            "insert",
            Box::new(|p| {
                p.timelines[0].tracks[0].audio.strip.inserts[0].effect =
                    Effect::Gain(GainFx { gain_db: -5.0 });
            }),
        ),
    ];
    for (what, change) in &changes {
        let mut project = base.clone();
        change(&mut project);
        assert_eq!(retuned_sounds_as_built(&base, &project), 1, "{what}");
    }
    // Several at once.
    let mut all = base.clone();
    for (_, change) in &changes {
        change(&mut all);
    }
    assert_eq!(retuned_sounds_as_built(&base, &all), changes.len());
    // Nothing changed: nothing to set.
    let graph = built(&base);
    assert_eq!(
        retune(&base.timelines[0], &base.timelines[0], &graph.controls()),
        Some(vec![])
    );
}

#[test]
fn a_fixed_gain_or_pan_of_a_clip_is_set_as_the_graph_plays_and_sounds_as_built() {
    let base = project();
    let mut louder = base.clone();
    louder.timelines[0].tracks[0].clips[0].gain_db = (-7.5).into();
    assert_eq!(retuned_sounds_as_built(&base, &louder), 1);
    let mut panned = louder.clone();
    panned.timelines[0].tracks[0].clips[0].pan = 0.6.into();
    assert_eq!(retuned_sounds_as_built(&louder, &panned), 1);
    // And back, from a graph built with the clip panned.
    assert_eq!(retuned_sounds_as_built(&panned, &base), 2);
    // What is left to compare once the levels are taken out is the same.
    assert_eq!(
        without_clip_levels(&base.timelines[0], &panned.timelines[0]),
        base.timelines[0]
    );
    // Keyframes are another graph: no control, and left as they are.
    let mut keyed = base.clone();
    keyed.timelines[0].tracks[0].clips[0].gain_db =
        columbina_model::AudioParam::Keys(vec![columbina_model::AudioKey {
            at: Time {
                ticks: 0,
                scale: 48_000,
            },
            value: -3.0,
            interp: columbina_model::KeyInterp::Linear,
        }]);
    let graph = built(&keyed);
    let mut moved = keyed.clone();
    moved.timelines[0].tracks[0].clips[0].pan = 0.5.into();
    let controls = graph.controls();
    assert!(controls
        .control(
            &columbina_audio::graph::Owner::Track("A1".into()),
            "clip.c1.gain_db"
        )
        .is_none());
    assert!(controls
        .control(
            &columbina_audio::graph::Owner::Track("A1".into()),
            "clip.c1.pan"
        )
        .is_some());
    assert_ne!(
        without_clip_levels(&base.timelines[0], &keyed.timelines[0]),
        base.timelines[0]
    );
}

#[test]
fn a_solo_is_set_as_what_each_strip_hears_by_the_solos() {
    let base = project();
    let mut soloed = base.clone();
    soloed.timelines[0].tracks[0].audio.solo = true;
    // A2 and A3 are silenced, and B2, which only A3 feeds; A1 and B1 are heard as before.
    assert_eq!(retuned_sounds_as_built(&base, &soloed), 3);
    let mut safe = soloed.clone();
    safe.timelines[0].tracks[2].audio.solo_safe = true;
    assert_eq!(retuned_sounds_as_built(&soloed, &safe), 2);
    let mut by_vca = base.clone();
    by_vca.timelines[0].vcas[0].solo = true;
    retuned_sounds_as_built(&base, &by_vca);
}

#[test]
#[allow(clippy::too_many_lines, reason = "a list of changes")]
fn a_change_of_what_the_graph_is_made_of_needs_another_graph() {
    let base = project();
    let lane = Lane {
        points: vec![LanePoint {
            at: Time {
                ticks: 0,
                scale: 48_000,
            },
            value: -3.0,
        }],
        kind: LaneKind::Continuous,
    };
    let changes: Vec<Change> = vec![
        (
            "an equalizer turned on",
            Box::new(|p| p.timelines[0].tracks[2].audio.strip.eq.on = true),
        ),
        (
            "Dynamics turned on",
            Box::new(|p| p.timelines[0].tracks[2].audio.strip.dynamics.on = true),
        ),
        (
            "an insert added",
            Box::new(|p| {
                p.timelines[0].tracks[2].audio.strip.inserts.push(Insert {
                    id: "fx9".into(),
                    on: true,
                    effect: Effect::Gain(GainFx { gain_db: 1.0 }),
                });
            }),
        ),
        (
            "a send before the fader",
            Box::new(|p| p.timelines[0].tracks[2].audio.strip.sends[0].pre = true),
        ),
        (
            "an output taken away",
            Box::new(|p| p.timelines[0].tracks[1].audio.strip.outputs.clear()),
        ),
        (
            "a lane",
            Box::new(move |p| {
                p.timelines[0].tracks[0]
                    .audio
                    .lanes
                    .insert("fader_db".into(), lane.clone());
            }),
        ),
        (
            "a format",
            Box::new(|p| {
                let audio = &mut p.timelines[0].tracks[0].audio;
                audio.format = columbina_model::TrackFormat::Mono;
                audio.channels = 1;
            }),
        ),
        (
            "a track turned off",
            Box::new(|p| p.timelines[0].tracks[0].enabled = false),
        ),
        (
            "the monitor",
            Box::new(|p| p.timelines[0].audio.monitor = Some("B2".into())),
        ),
        (
            "a VCA joined",
            Box::new(|p| p.timelines[0].tracks[0].audio.strip.vcas = vec!["VCA1".into()]),
        ),
        (
            "a pan's rotation",
            Box::new(|p| p.timelines[0].tracks[0].audio.strip.pan.rotate = 30.0),
        ),
        (
            "a Listen",
            Box::new(|p| {
                p.timelines[0].tracks[1]
                    .audio
                    .strip
                    .dynamics
                    .compressor
                    .sidechain
                    .listen = true;
            }),
        ),
        (
            "a kind of band",
            Box::new(|p| {
                p.timelines[0].tracks[0].audio.strip.eq.bands[2].kind =
                    columbina_model::audio::fx::EqKind::Notch;
            }),
        ),
    ];
    let graph = built(&base);
    for (what, change) in &changes {
        let mut project = base.clone();
        change(&mut project);
        assert_eq!(
            retune(&base.timelines[0], &project.timelines[0], &graph.controls()),
            None,
            "{what}"
        );
    }
}
