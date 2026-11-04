//! The lanes of automation heard through the graph (plan T14): a fader that ramps, heard at
//! each sample as the curve of its gain; a mute that switches on its sample, wherever its point
//! falls between two; lanes of a bus, a VCA, a send and an insert; a lane that takes the place of
//! the setting; and an output that depends neither on its blocks nor on where a run starts.

use std::ops::Range;

use columbina_audio::feed::{ClipFeed, FeedError};
use columbina_audio::graph::{Graph, Options, Purpose};
use columbina_audio::testing::Rng;
use columbina_model::audio::fx::GainFx;
use columbina_model::{
    Bus, Effect, Insert, Lane, LaneKind, LanePoint, Output, Project, Send, Time, TrackKind, Vca,
};

/// The sample project with its track of sound only: A1, with clip c2 of 4 s of media m1 (stereo,
/// 48 kHz) from the start of the timeline, out to B1.
fn base() -> Project {
    let mut project = Project::sample();
    project.timelines[0]
        .tracks
        .retain(|track| track.kind == TrackKind::Audio);
    project
}

fn built(project: &Project) -> Graph {
    Graph::build(project, 0, Options::new(4096, Purpose::Export)).unwrap()
}

/// A feed that gives `signal(sample of the source, channel)` for every clip.
fn feed(
    signal: impl Fn(i64, usize) -> f32,
) -> impl FnMut(usize, Range<i64>, &mut [&mut [f32]]) -> Result<(), FeedError> {
    move |_, range, out| {
        for (channel, out) in out.iter_mut().enumerate() {
            for (n, sample) in out.iter_mut().enumerate() {
                *sample = signal(range.start + i64::try_from(n).unwrap(), channel);
            }
        }
        Ok(())
    }
}

/// `frames` samples of the monitor from sample `at`, in blocks as long as `cut` says: a channel
/// after another.
fn play(
    graph: &mut Graph,
    feed: &mut dyn ClipFeed,
    (at, frames): (i64, usize),
    cut: &mut dyn FnMut() -> usize,
) -> Vec<Vec<f32>> {
    graph.start(at, feed).unwrap();
    let output = graph.monitor().unwrap();
    let channels = graph.output(output).channels();
    let mut played = vec![Vec::new(); channels];
    let mut done = 0;
    while done < frames {
        let n = cut().min(frames - done).min(graph.max_block());
        graph.process(feed, n).unwrap();
        for (channel, played) in played.iter_mut().enumerate() {
            played.extend_from_slice(&graph.output(output).channel(channel)[..n]);
        }
        done += n;
    }
    played
}

/// The left channel of the monitor over `frames` from 0, in blocks of 4096, of every clip
/// sounding `value`.
fn heard(project: &Project, frames: usize, value: f32) -> Vec<f32> {
    let mut graph = built(project);
    let mut read = feed(move |_, _| value);
    play(&mut graph, &mut read, (0, frames), &mut || 4096).swap_remove(0)
}

/// A lane of points at `(ticks, value)` of `scale`.
fn lane(kind: LaneKind, scale: u32, points: &[(i64, f64)]) -> Lane {
    Lane {
        points: points
            .iter()
            .map(|&(ticks, value)| LanePoint {
                at: Time { ticks, scale },
                value,
            })
            .collect(),
        kind,
    }
}

fn gain_of(db: f64) -> f64 {
    10f64.powf(db / 20.0)
}

#[test]
fn a_fader_ramped_from_minus_20_to_0_db_over_a_second_is_heard_as_its_curve_at_each_sample() {
    // The lane at the rate of the timeline, and at 44.1 kHz on a timeline of 48 kHz: both are
    // the same line over the same second.
    for scale in [48_000u32, 44_100] {
        let mut project = base();
        project.timelines[0].tracks[0].audio.lanes.insert(
            "fader_db".into(),
            lane(
                LaneKind::Continuous,
                scale,
                &[(0, -20.0), (i64::from(scale), 0.0)],
            ),
        );
        let got = heard(&project, 50_000, 0.5);
        for (n, &sample) in got.iter().enumerate() {
            let n = f64::from(u32::try_from(n).unwrap());
            let db = if n >= 48_000.0 {
                0.0
            } else {
                -20.0 + 20.0 * n / 48_000.0
            };
            let want = 0.5 * gain_of(db);
            // Within a few steps of a single (1e-6, 1e-5 dB); a sample late or early is off by
            // 20/48000 dB, 5e-5 of the gain.
            assert!(
                (f64::from(sample) - want).abs() <= 1e-6 * want,
                "{scale}: sample {n}: {sample} is not {want}"
            );
        }
    }
}

#[test]
fn a_mute_switches_on_the_first_sample_at_or_after_its_point() {
    // A seventh of a second is 6857 1/7 samples: the mute takes the 6858th sample on, and lets
    // the sound back in at two sevenths (13714 2/7: from 13715).
    let mut project = base();
    project.timelines[0].tracks[0].audio.lanes.insert(
        "mute".into(),
        lane(LaneKind::Switch, 7, &[(0, 0.0), (1, 1.0), (2, 0.0)]),
    );
    let got = heard(&project, 20_000, 0.5);
    assert_eq!(got[6857], 0.5);
    assert_eq!(got[6858], 0.0);
    assert_eq!(got[13_714], 0.0);
    assert_eq!(got[13_715], 0.5);
    assert!(got[6858..13_715].iter().all(|&s| s == 0.0));
    // The same at a sample of a scale of its own, 1 s at 48 kHz.
    let mut project = base();
    project.timelines[0].tracks[0].audio.lanes.insert(
        "mute".into(),
        lane(LaneKind::Switch, 48_000, &[(48_000, 1.0)]),
    );
    let got = heard(&project, 50_000, 0.5);
    assert_eq!((got[0], got[47_999], got[48_000]), (0.0, 0.0, 0.0));
    let mut project = base();
    project.timelines[0].tracks[0].audio.lanes.insert(
        "mute".into(),
        lane(LaneKind::Switch, 48_000, &[(0, 0.0), (48_000, 1.0)]),
    );
    let got = heard(&project, 50_000, 0.5);
    assert_eq!((got[47_999], got[48_000]), (0.5, 0.0));
}

#[test]
fn a_lane_takes_the_place_of_its_setting_until_it_is_taken_away() {
    let mut project = base();
    let audio = &mut project.timelines[0].tracks[0].audio;
    audio.lanes.insert(
        "fader_db".into(),
        lane(LaneKind::Continuous, 48_000, &[(0, -6.0)]),
    );
    let with_lane = heard(&project, 1000, 0.5);
    project.timelines[0].tracks[0].audio.strip.fader_db = -20.0;
    assert_eq!(
        heard(&project, 1000, 0.5),
        with_lane,
        "the setting is not heard"
    );
    project.timelines[0].tracks[0].audio.lanes.clear();
    let without = heard(&project, 1000, 0.5);
    assert!((f64::from(without[500]) - 0.5 * gain_of(-20.0)).abs() < 1e-7);
    assert!((f64::from(with_lane[500]) - 0.5 * gain_of(-6.0)).abs() < 1e-7);
}

/// A1 with a Gain (`fx1`), a send to B2 before its fader, in VCA1; B2 out to B1.
fn routed() -> Project {
    let mut project = base();
    let timeline = &mut project.timelines[0];
    let mut b2 = Bus::first();
    b2.id = "B2".into();
    b2.name = "B2".into();
    b2.strip.outputs = vec![Output {
        bus: "B1".into(),
        on: true,
    }];
    timeline.buses.push(b2);
    timeline.vcas.push(Vca {
        id: "VCA1".into(),
        name: "VCA".into(),
        fader_db: 0.0,
        mute: false,
        solo: false,
        vcas: vec![],
        lanes: columbina_model::Lanes::new(),
    });
    let strip = &mut timeline.tracks[0].audio.strip;
    strip.inserts.push(Insert {
        id: "fx1".into(),
        on: true,
        effect: Effect::Gain(GainFx { gain_db: 0.0 }),
    });
    strip.vcas = vec!["VCA1".into()];
    project
}

#[test]
fn the_lanes_of_a_bus_a_vca_a_send_and_an_insert_are_heard() {
    let at_one = |project: &Project| f64::from(heard(project, 2000, 0.5)[1000]);
    let step = |value| lane(LaneKind::Continuous, 48_000, &[(0, value)]);
    let mut project = routed();
    project.timelines[0].tracks[0]
        .audio
        .lanes
        .insert("insert.fx1.gain_db".into(), step(-6.0));
    assert!(
        (at_one(&project) - 0.5 * gain_of(-6.0)).abs() < 1e-6,
        "insert"
    );
    let mut project = routed();
    project.timelines[0].vcas[0]
        .lanes
        .insert("fader_db".into(), step(-10.0));
    assert!(
        (at_one(&project) - 0.5 * gain_of(-10.0)).abs() < 1e-6,
        "VCA"
    );
    let mut project = routed();
    project.timelines[0].buses[0]
        .lanes
        .insert("fader_db".into(), step(-3.0));
    assert!((at_one(&project) - 0.5 * gain_of(-3.0)).abs() < 1e-6, "bus");
    // A send to B2 before the fader, at -100 dB, raised by its lane to 0 dB, and the track's
    // fader at -100: what B1 hears comes by way of B2, the lane's level.
    let mut project = routed();
    let audio = &mut project.timelines[0].tracks[0].audio;
    audio.strip.fader_db = -100.0;
    audio.strip.sends.push(Send {
        bus: "B2".into(),
        on: true,
        pre: true,
        level_db: -100.0,
        pan: columbina_model::Pan::default(),
    });
    audio.lanes.insert("send.B2.level_db".into(), step(0.0));
    assert!((at_one(&project) - 0.5).abs() < 1e-6, "send");
}

#[test]
fn the_output_with_lanes_depends_neither_on_its_blocks_nor_on_where_a_run_starts() {
    let mut project = routed();
    let timeline = &mut project.timelines[0];
    let ramp = |points: &[(i64, f64)]| lane(LaneKind::Continuous, 44_100, points);
    let audio = &mut timeline.tracks[0].audio;
    audio.strip.sends.push(Send {
        bus: "B2".into(),
        on: true,
        pre: false,
        level_db: -6.0,
        pan: columbina_model::Pan::default(),
    });
    for (path, value) in [
        (
            "fader_db",
            ramp(&[(1000, -30.0), (40_000, 6.0), (40_000, -12.0), (90_000, 0.0)]),
        ),
        ("pan.x", ramp(&[(0, -1.0), (150_000, 1.0)])),
        ("send.B2.level_db", ramp(&[(20_000, -40.0), (60_000, 3.0)])),
        ("insert.fx1.gain_db", ramp(&[(5000, 6.0), (70_000, -12.0)])),
        (
            "mute",
            lane(
                LaneKind::Switch,
                7,
                &[(0, 0.0), (5, 1.0), (6, 0.0), (11, 1.0), (12, 0.0)],
            ),
        ),
    ] {
        audio.lanes.insert(path.into(), value);
    }
    timeline.vcas[0]
        .lanes
        .insert("fader_db".into(), ramp(&[(0, 0.0), (120_000, -9.0)]));
    timeline.buses[1]
        .lanes
        .insert("fader_db".into(), ramp(&[(30_000, -20.0), (100_000, 0.0)]));
    let signal = |n: i64, channel: usize| {
        let mut rng = Rng::new(n.cast_unsigned().wrapping_mul(7) ^ (channel as u64));
        rng.signed() * 0.3
    };
    let mut graph = built(&project);
    let mut read = feed(signal);
    let frames = 150_000;
    let whole = play(&mut graph, &mut read, (3000, frames), &mut || 4096);
    assert!(whole[0].iter().any(|&s| s != 0.0));
    let same = |a: &[f32], b: &[f32]| {
        a.iter()
            .zip(b)
            .position(|(x, y)| x.to_bits() != y.to_bits())
    };
    for seed in 0..3 {
        let mut rng = Rng::new(seed);
        let cut = play(&mut graph, &mut read, (3000, frames), &mut || {
            if rng.range(0, 8) == 0 {
                rng.range(1, 4)
            } else {
                rng.log_range(4096)
            }
        });
        for (whole, cut) in whole.iter().zip(&cut) {
            assert_eq!(same(whole, cut), None, "seed {seed}");
        }
    }
    // Runs that start in the middle, after another run has gone further.
    for start in [3000 + 77_777, 3000 + 6858, 3000 + 149_000] {
        let skip = usize::try_from(start - 3000).unwrap();
        let part = play(&mut graph, &mut read, (start, frames - skip), &mut || 1000);
        for (whole, part) in whole.iter().zip(&part) {
            assert_eq!(same(&whole[skip..], part), None, "from {start}");
        }
    }
}
