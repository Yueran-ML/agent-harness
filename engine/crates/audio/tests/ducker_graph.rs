//! The Ducker in the graph (T11): a dialogue track ducking a music track on the timeline, its
//! source muted, soloed away or routed nowhere, a bus as its source, sources summed and each
//! lined up with the track, its controls and lanes, built only where it is on, the same output in
//! any blocks and from any start, and what its window reads.

#![allow(
    clippy::float_cmp,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::needless_range_loop,
    reason = "exact values compared on purpose, sample numbers, levels of tests"
)]

use std::ops::Range;

use columbina_audio::feed::{ClipFeed, FeedError};
use columbina_audio::fx::ducker;
use columbina_audio::graph::{Graph, Options, Owner, Purpose};
use columbina_audio::testing::Rng;
use columbina_model::audio::fx::{Ducker, NoiseReductionFx};
use columbina_model::{
    Bus, Effect, Insert, Lane, LaneKind, LanePoint, Output, Project, SourceRef, Time, TrackKind,
};

/// The lookahead of a new Ducker, the samples of a rise, a hold and a recovery at 48 kHz.
const L: usize = 720;
const R: usize = 480;
const W: usize = 480;
const H: usize = 7_200;
const C: usize = 36_000;

/// The level of the music.
const MUSIC: f32 = 0.1;

/// The sample project with `count` tracks of sound like its A1 (a clip of 4 s of stereo media
/// from the start, out to B1): A1, A2, ..., their clips c1, c2, ...
fn tracks(count: usize) -> Project {
    let mut project = Project::sample();
    let timeline = &mut project.timelines[0];
    timeline
        .tracks
        .retain(|track| track.kind == TrackKind::Audio);
    let first = timeline.tracks[0].clone();
    timeline.tracks = (1..=count)
        .map(|n| {
            let mut track = first.clone();
            track.id = format!("A{n}");
            track.clips[0].id = format!("c{n}");
            track
        })
        .collect();
    project
}

/// A1 ducked 12 dB by `sources`, the rest as a new Ducker; then `edit`.
fn ducked(count: usize, sources: &[SourceRef], edit: impl FnOnce(&mut Project)) -> Project {
    let mut project = tracks(count);
    project.timelines[0].tracks[0].audio.strip.track_fx = Some(Ducker {
        on: true,
        sources: sources.to_vec(),
        amount_db: 12.0,
        ..Ducker::default()
    });
    edit(&mut project);
    project
}

fn track(id: &str) -> SourceRef {
    SourceRef::Track(id.into())
}

/// Noise whose RMS is −10 dBFS, a value for each sample of the timeline.
fn speech(n: i64) -> f32 {
    0.547_7 * Rng::new(n as u64 ^ 0x5eed).signed()
}

/// The music at [`MUSIC`] on c1, speech on `talking` from 1 s to 2 s; nothing else.
fn scene(talking: &'static str) -> impl Fn(&str, i64, usize) -> f32 {
    move |clip, n, _| match clip {
        "c1" => MUSIC,
        _ if clip == talking && (48_000..96_000).contains(&n) => speech(n),
        _ => 0.0,
    }
}

/// A feed that gives `signal(clip, sample of the source, channel)`.
fn feed(
    graph: &Graph,
    signal: impl Fn(&str, i64, usize) -> f32,
) -> impl FnMut(usize, Range<i64>, &mut [&mut [f32]]) -> Result<(), FeedError> {
    let ids: Vec<String> = graph.clips().iter().map(|c| c.clip.clone()).collect();
    move |clip, range, out| {
        for (channel, out) in out.iter_mut().enumerate() {
            for (n, sample) in out.iter_mut().enumerate() {
                *sample = signal(&ids[clip], range.start + n as i64, channel);
            }
        }
        Ok(())
    }
}

/// The left channel of B1 from sample `at`, `frames` of it, in blocks as long as `cut` says.
fn play_cut(
    graph: &mut Graph,
    feed: &mut dyn ClipFeed,
    (at, frames): (i64, usize),
    cut: &mut dyn FnMut() -> usize,
) -> Vec<f32> {
    graph.start(at, feed).unwrap();
    let output = graph.output_of("B1").unwrap();
    let mut played = Vec::with_capacity(frames);
    while played.len() < frames {
        let n = cut().min(frames - played.len()).min(graph.max_block());
        graph.process(feed, n).unwrap();
        played.extend_from_slice(&graph.output(output).channel(0)[..n]);
    }
    played
}

/// The left channel of B1 of `project` built for `purpose`, the clips as `signal` says, from 0
/// for 3.2 s; and the latency of the graph.
fn play(
    project: &Project,
    purpose: Purpose,
    signal: impl Fn(&str, i64, usize) -> f32,
) -> (Vec<f32>, u32) {
    let mut graph = Graph::build(project, 0, Options::new(4_096, purpose)).unwrap();
    let mut read = feed(&graph, signal);
    let played = play_cut(&mut graph, &mut read, (0, 3_200 * 48), &mut || 4_096);
    (played, graph.latency())
}

fn db(gain: f64) -> f64 {
    20.0 * gain.log10()
}

/// The gains of the music in `out`.
fn gains(out: &[f32]) -> Vec<f64> {
    out.iter()
        .map(|&s| f64::from(s) / f64::from(MUSIC))
        .collect()
}

/// The first sample of the timeline at which the music is ducked.
fn first_ducked(out: &[f32]) -> usize {
    out.iter()
        .position(|&s| s != MUSIC)
        .expect("ducked somewhere")
}

#[test]
fn a_dialogue_track_ducks_a_music_track_by_its_amount_on_the_timeline() {
    // A2 talks from 1 s to 2 s and is muted: the music of A1 alone is heard.
    let project = ducked(2, &[track("A2")], |p| {
        p.timelines[0].tracks[1].audio.mute = true;
    });
    let (out, latency) = play(&project, Purpose::Export, scene("c2"));
    assert_eq!(latency as usize, L);
    let gains = gains(&out);
    // Down 15 ms ahead of the speech (within 1 ms), by 12 dB 10 ms later.
    let first = first_ducked(&out);
    assert!((48_000 - L..=48_000 - L + 48).contains(&first), "{first}");
    assert!((db(gains[first + R]) + 12.0).abs() <= 0.1);
    for n in first + R..96_000 - L {
        assert!((db(gains[n]) + 12.0).abs() <= 1e-4, "{n}: {}", db(gains[n]));
    }
    // Held 150 ms after the window of 10 ms is past the speech, then back in 750 ms, −6 dB half
    // way, and the music to the bit after.
    let back = (first + R..out.len())
        .find(|&n| gains[n] > gains[first + R] * 1.000_01)
        .unwrap();
    let want = 96_000 + W + H - L;
    assert!((want - 48..=want + 1).contains(&back), "{back} for {want}");
    assert!((db(gains[back + C / 2 - 1]) + 6.0).abs() <= 0.2);
    assert!(out[back + C..].iter().all(|&s| s == MUSIC));
    // The same in a preview.
    assert_eq!(play(&project, Purpose::Preview, scene("c2")).0, out);
}

#[test]
fn the_source_ducks_whether_it_is_muted_soloed_away_or_routed_nowhere() {
    let base = ducked(2, &[track("A2")], |p| {
        p.timelines[0].tracks[1].audio.mute = true;
    });
    let (muted, _) = play(&base, Purpose::Export, scene("c2"));
    let soloed_away = ducked(2, &[track("A2")], |p| {
        p.timelines[0].tracks[0].audio.solo = true;
    });
    let nowhere = ducked(2, &[track("A2")], |p| {
        p.timelines[0].tracks[1].audio.strip.outputs.clear();
    });
    for (project, what) in [(soloed_away, "soloed away"), (nowhere, "routed nowhere")] {
        assert_eq!(
            play(&project, Purpose::Export, scene("c2")).0,
            muted,
            "{what}"
        );
    }
    // A track that is off is not in the graph: it ducks nothing, and the Ducker is not built.
    let off = ducked(2, &[track("A2")], |p| {
        p.timelines[0].tracks[1].enabled = false;
    });
    let (out, latency) = play(&off, Purpose::Export, scene("c2"));
    assert_eq!(latency, 0);
    assert!(out.iter().all(|&s| s == MUSIC));
}

#[test]
fn a_bus_keys_a_ducker_as_the_tracks_that_feed_it() {
    let (by_track, _) = play(
        &ducked(2, &[track("A2")], |p| {
            p.timelines[0].tracks[1].audio.strip.outputs.clear();
        }),
        Purpose::Export,
        scene("c2"),
    );
    let by_bus = ducked(2, &[SourceRef::Bus("B2".into())], |p| {
        let timeline = &mut p.timelines[0];
        let mut b2 = Bus::first();
        b2.id = "B2".into();
        b2.name = "B2".into();
        b2.strip.outputs.clear();
        timeline.buses.push(b2);
        timeline.tracks[1].audio.strip.outputs = vec![Output {
            bus: "B2".into(),
            on: true,
        }];
    });
    assert_eq!(play(&by_bus, Purpose::Export, scene("c2")).0, by_track);
}

#[test]
fn the_sources_of_a_ducker_are_summed() {
    // A2 and A3 each a tone of −48.5 dBFS of RMS, under the threshold alone; their sum, the same
    // tone 6 dB up, −42.5 dBFS, over it.
    let tone = |clip: &str, n: i64, _| match clip {
        "c1" => MUSIC,
        "c2" | "c3" => {
            let amplitude = 10_f64.powf(-48.5 / 20.0) * std::f64::consts::SQRT_2;
            (amplitude * (std::f64::consts::TAU * 1_000.0 * n as f64 / 48_000.0).sin()) as f32
        }
        _ => 0.0,
    };
    let quiet = |p: &mut Project| {
        p.timelines[0].tracks[1].audio.mute = true;
        p.timelines[0].tracks[2].audio.mute = true;
    };
    let (alone, _) = play(&ducked(3, &[track("A2")], quiet), Purpose::Export, tone);
    assert!(
        alone.iter().all(|&s| s == MUSIC),
        "one source under the threshold"
    );
    let both = ducked(3, &[track("A2"), track("A3"), track("A2")], quiet);
    let (summed, _) = play(&both, Purpose::Export, tone);
    let deepest = gains(&summed).into_iter().fold(1.0, f64::min);
    assert!((db(deepest) + 12.0).abs() <= 1e-4, "{}", db(deepest));
}

/// Noise Reduction that lets its sound through as it is, late by its latency of 1536 samples.
fn late() -> Insert {
    Insert {
        id: "fx1".into(),
        on: true,
        effect: Effect::NoiseReduction(NoiseReductionFx {
            dry_wet: 0.0,
            ..Default::default()
        }),
    }
}

#[test]
fn each_source_is_lined_up_with_the_track_that_it_ducks() {
    // A2 is late by 1536 samples, A3 is not; either talking, or the track ducked being the late
    // one, the music goes down 15 ms ahead of the speech on the timeline.
    let cases: [(&str, usize); 3] = [("c2", 1), ("c3", 1), ("c2", 0)];
    for (talking, late_track) in cases {
        let project = ducked(3, &[track("A2"), track("A3")], |p| {
            let tracks = &mut p.timelines[0].tracks;
            tracks[late_track].audio.strip.inserts.push(late());
            tracks[1].audio.mute = true;
            tracks[2].audio.mute = true;
        });
        let (out, _) = play(&project, Purpose::Export, scene(talking));
        let first = out.iter().position(|&s| s < MUSIC * 0.999).unwrap();
        assert!(
            (48_000 - L..=48_000 - L + 48).contains(&first),
            "{talking}, A{} late: {first}",
            late_track + 1
        );
    }
}

#[test]
fn the_ducker_comes_before_the_inserts_of_its_track() {
    // Music at −0.9 dBFS into a Limiter of −6 dBTP: ducked first, by 12 dB, it is under the
    // ceiling and passes as it is (0.9 · 10^(−12/20) = 0.2265); limited first, it would be
    // under 0.5 · 0.251.
    let project = ducked(2, &[track("A2")], |p| {
        let tracks = &mut p.timelines[0].tracks;
        tracks[1].audio.mute = true;
        tracks[0].audio.strip.inserts.push(Insert {
            id: "fx1".into(),
            on: true,
            effect: Effect::Limiter(columbina_model::audio::fx::LimiterFx {
                ceiling_db: -6.0,
                ..Default::default()
            }),
        });
    });
    let loud = |clip: &str, n: i64, channel: usize| match clip {
        "c1" => 0.9,
        _ => scene("c2")(clip, n, channel),
    };
    let (out, _) = play(&project, Purpose::Export, loud);
    let want = 0.9 * 10_f64.powf(-12.0 / 20.0);
    // Late in the speech, the release of the limiting before it long over.
    assert!(
        (f64::from(out[94_000]) - want).abs() <= 1e-5,
        "{}",
        out[94_000]
    );
}

fn lane(points: &[(i64, f64)]) -> Lane {
    Lane {
        points: points
            .iter()
            .map(|&(ticks, value)| LanePoint {
                at: Time {
                    ticks,
                    scale: 48_000,
                },
                value,
            })
            .collect(),
        kind: LaneKind::Continuous,
    }
}

#[test]
fn the_controls_and_lanes_of_a_ducker_are_found_by_their_paths() {
    let project = ducked(2, &[track("A2")], |p| {
        p.timelines[0].tracks[1].audio.mute = true;
    });
    let graph = Graph::build(&project, 0, Options::new(4_096, Purpose::Preview)).unwrap();
    let a1 = Owner::Track("A1".into());
    for path in ["on", "amount_db", "rise_ms", "hold_ms", "recovery_ms"] {
        let path = format!("track_fx.{path}");
        assert!(graph.control(&a1, &path).is_some(), "{path}");
    }
    for path in [
        "track_fx.lookahead_ms",
        "track_fx.sources",
        "track_fx.listen",
    ] {
        assert!(graph.control(&a1, path).is_none(), "{path}");
    }
    assert!(graph
        .control(&Owner::Track("A2".into()), "track_fx.on")
        .is_none());

    // Set while it plays: 24 dB deep 10 ms later.
    let mut graph = graph;
    let amount = graph.control(&a1, "track_fx.amount_db").unwrap();
    let mut read = feed(&graph, scene("c2"));
    graph.start(60_000, &mut read).unwrap();
    graph.set(amount, 24.0);
    let output = graph.output_of("B1").unwrap();
    graph.process(&mut read, 1_000).unwrap();
    let got = db(f64::from(graph.output(output).channel(0)[600]) / f64::from(MUSIC));
    assert!((got + 24.0).abs() <= 1e-4, "{got}");

    // A lane of the amount: 24 dB where it says so.
    let mut laned = project.clone();
    laned.timelines[0].tracks[0]
        .audio
        .lanes
        .insert("track_fx.amount_db".into(), lane(&[(0, 24.0)]));
    let (out, _) = play(&laned, Purpose::Export, scene("c2"));
    assert!((db(f64::from(out[70_000]) / f64::from(MUSIC)) + 24.0).abs() <= 1e-4);
}

#[test]
fn a_ducker_is_built_only_where_it_is_on_and_has_sources() {
    let silent = |p: &Project| play(p, Purpose::Export, scene("c2"));
    let plain = tracks(2);
    let (none, _) = silent(&plain);
    let off = ducked(2, &[track("A2")], |p| {
        p.timelines[0].tracks[0]
            .audio
            .strip
            .track_fx
            .as_mut()
            .unwrap()
            .on = false;
    });
    let sourceless = ducked(2, &[], |_| {});
    for (project, what) in [(off, "off"), (sourceless, "with no source")] {
        let graph = Graph::build(&project, 0, Options::new(4_096, Purpose::Export)).unwrap();
        assert_eq!(graph.latency(), 0, "{what}");
        assert_eq!(graph.settle(), 0, "{what}");
        assert!(graph.ducker(&Owner::Track("A1".into())).is_none(), "{what}");
        assert_eq!(silent(&project).0, none, "{what}");
    }
    // Off, but a lane turns it on: built, and settling for the longest hold of its lane.
    let laned = ducked(2, &[track("A2")], |p| {
        let a1 = &mut p.timelines[0].tracks[0].audio;
        a1.strip.track_fx.as_mut().unwrap().on = false;
        a1.lanes
            .insert("track_fx.on".into(), lane(&[(0, 0.0), (48_000, 1.0)]));
        a1.lanes.insert(
            "track_fx.hold_ms".into(),
            lane(&[(0, 150.0), (96_000, 4_000.0)]),
        );
    });
    let graph = Graph::build(&laned, 0, Options::new(4_096, Purpose::Export)).unwrap();
    assert_eq!(graph.latency() as usize, L);
    let slowest = ducker::Settings {
        on: true,
        amount_db: 12.0,
        hold_ms: 4_000.0,
        ..ducker::Settings::default()
    };
    assert_eq!(graph.settle(), ducker::settle_for(&slowest, 48_000));
    assert!(graph.ducker(&Owner::Track("A1".into())).is_some());
    assert!(graph.ducker(&Owner::Track("A2".into())).is_none());
}

#[test]
fn a_graph_with_a_ducker_does_not_depend_on_its_blocks_nor_on_where_it_starts() {
    let project = ducked(3, &[track("A2"), track("A3")], |p| {
        let tracks = &mut p.timelines[0].tracks;
        tracks[2].audio.strip.inserts.push(late());
        tracks[0].audio.lanes.insert(
            "track_fx.amount_db".into(),
            lane(&[(0, 6.0), (150_000, 24.0)]),
        );
    });
    let signal = |clip: &str, n: i64, channel: usize| match clip {
        "c1" => 0.2 * Rng::new((n as u64).wrapping_mul(2).wrapping_add(channel as u64)).signed(),
        "c2" if (40_000..60_000).contains(&n) => speech(n),
        "c3" if (100_000..105_000).contains(&n) => speech(n),
        _ => 0.0,
    };
    let frames = 180_000;
    let mut graph = Graph::build(&project, 0, Options::new(4_096, Purpose::Export)).unwrap();
    let mut read = feed(&graph, signal);
    let whole = play_cut(&mut graph, &mut read, (0, frames), &mut || 4_096);
    for seed in [1, 2, 3] {
        let mut rng = Rng::new(seed);
        let cut = play_cut(&mut graph, &mut read, (0, frames), &mut || {
            rng.log_range(4_096)
        });
        assert_eq!(cut, whole, "seed {seed}");
    }
    // Started in the speech, in the hold, in the recovery: as from the start after its preroll.
    for at in [50_000_usize, 62_000, 75_000] {
        let late = play_cut(&mut graph, &mut read, (at as i64, frames - at), &mut || {
            1_000
        });
        let largest = late
            .iter()
            .zip(&whole[at..])
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f32::max);
        assert!(largest < 3.16e-5, "from {at}: {largest}");
    }
}

#[test]
fn the_window_of_a_ducker_reads_its_reduction_through_the_graph() {
    let project = ducked(2, &[track("A2")], |p| {
        p.timelines[0].tracks[1].audio.mute = true;
    });
    let mut graph = Graph::build(&project, 0, Options::new(480, Purpose::Preview)).unwrap();
    let meter = graph.ducker(&Owner::Track("A1".into())).unwrap();
    let mut read = feed(&graph, scene("c2"));
    graph.start(0, &mut read).unwrap();
    for _ in 0..150 {
        graph.process(&mut read, 480).unwrap();
    }
    let (input, source, reduction) = meter.read();
    assert!((input + 20.0).abs() <= 1e-3, "{input}");
    assert!((source + 10.0).abs() <= 1.0, "{source}");
    assert!((reduction + 12.0).abs() <= 1e-3, "{reduction}");
}
