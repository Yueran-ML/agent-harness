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
        scale: None,
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

// The review of T14.

/// A1 and A2 with every effect between them, Dynamics, an equalizer, a Ducker, sends and
/// outputs; B2 with an insert; both buses with Dynamics and an equalizer; and a VCA.
fn everything() -> Project {
    use columbina_model::audio::fx::{
        DeEsserFx, DeHummerFx, LimiterFx, NoiseReductionFx, PitchFx, SoftClipperFx, StereoWidthFx,
    };
    let mut project = routed();
    let timeline = &mut project.timelines[0];
    let mut a2 = timeline.tracks[0].clone();
    a2.id = "A2".into();
    a2.clips[0].id = "c9".into();
    timeline.tracks.push(a2);
    let insert = |id: &str, effect| Insert {
        id: id.into(),
        on: true,
        effect,
    };
    let send = |bus: &str| Send {
        bus: bus.into(),
        on: true,
        pre: false,
        level_db: -6.0,
        pan: columbina_model::Pan::default(),
    };
    for (r, effects) in [
        (
            0,
            vec![
                Effect::Gain(GainFx { gain_db: 0.0 }),
                Effect::Limiter(LimiterFx::default()),
                Effect::SoftClipper(SoftClipperFx::default()),
                Effect::StereoWidth(StereoWidthFx::default()),
                Effect::DeEsser(DeEsserFx::default()),
                Effect::DeHummer(DeHummerFx::default()),
            ],
        ),
        (
            1,
            vec![
                Effect::NoiseReduction(NoiseReductionFx::default()),
                Effect::Pitch(PitchFx::default()),
            ],
        ),
    ] {
        let strip = &mut timeline.tracks[r].audio.strip;
        strip.inserts = effects
            .into_iter()
            .enumerate()
            .map(|(k, effect)| insert(&format!("fx{}", k + 1), effect))
            .collect();
        strip.dynamics.on = true;
        strip.eq.on = true;
        strip.sends = vec![send("B2")];
    }
    timeline.tracks[0].audio.strip.track_fx = Some(columbina_model::audio::fx::Ducker {
        on: true,
        sources: vec![columbina_model::SourceRef::Track("A2".into())],
        ..columbina_model::audio::fx::Ducker::default()
    });
    timeline.buses[1].strip.inserts = vec![insert("fx1", Effect::Gain(GainFx { gain_db: 0.0 }))];
    // Dynamics and an equalizer are built where they are on (or a lane turns them on).
    for bus in &mut timeline.buses {
        bus.strip.dynamics.on = true;
        bus.strip.eq.on = true;
    }
    project.validate().unwrap();
    project
}

#[test]
fn every_setting_that_a_lane_may_be_of_is_one_that_the_graph_moves() {
    use columbina_audio::graph::Owner;
    use columbina_model::audio::automation::{no_lane, paths, Owner as ModelOwner};
    let project = everything();
    let graph = built(&project);
    let timeline = &project.timelines[0];
    let mut strips: Vec<(Owner, ModelOwner)> = Vec::new();
    for track in &timeline.tracks {
        let audio = &track.audio;
        let owner = ModelOwner::Strip {
            strip: &audio.strip,
            mute: audio.mute,
            bus: false,
        };
        strips.push((Owner::Track(track.id.clone()), owner));
    }
    for bus in &timeline.buses {
        let owner = ModelOwner::Strip {
            strip: &bus.strip,
            mute: bus.mute,
            bus: true,
        };
        strips.push((Owner::Bus(bus.id.clone()), owner));
    }
    strips.push((
        Owner::Vca("VCA1".into()),
        ModelOwner::Vca(&timeline.vcas[0]),
    ));
    let mut checked = 0;
    let mut missing = Vec::new();
    for (owner, model) in &strips {
        for path in paths(*model) {
            if graph.control(owner, &path).is_none() {
                missing.push(format!("{owner:?} {path}"));
            }
            checked += 1;
        }
    }
    assert_eq!(
        missing,
        Vec::<String>::new(),
        "lanes of settings that the graph does not move"
    );
    assert!(checked > 150, "{checked}");
    // The cents, the mix and the switch of a Pitch have lanes, which the graph moves (T4c).
    let (a2_owner, a2_model) = &strips[1];
    for path in ["insert.fx2.cents", "insert.fx2.dry_wet", "insert.fx2.on"] {
        assert!(
            paths(*a2_model).iter().any(|p| p == path),
            "{path} has no lane"
        );
        assert!(graph.control(a2_owner, path).is_some(), "{path}");
    }
    // What the graph does not move as a lane would has no lane, and is refused as such.
    let (_, a1) = &strips[0];
    let (_, a2) = &strips[1];
    for (owner, path, why) in [
        (a1, "inserts_on", "lane_not_wired"),
        (a1, "pan.on", "lane_not_wired"),
        (a1, "pan.lfe_db", "lane_not_wired"),
        (a1, "pan.lfe_on", "lane_not_wired"),
        (a1, "pan.lfe_pre", "lane_not_wired"),
        (a1, "pan.rotate", "lane_not_wired"),
        (a1, "pan.spread", "lane_not_wired"),
        (a1, "send.B2.pre", "lane_not_wired"),
        (a1, "send.B2.pan.divergence", "lane_not_wired"),
        (a1, "send.B2.pan.on", "lane_not_wired"),
        (a1, "track_fx.lookahead_ms", "lane_not_wired"),
        (a2, "insert.fx2.semitones", "lane_integer"),
        (a1, "eq.1.slope", "lane_integer"),
    ] {
        assert_eq!(no_lane(*owner, path), why, "{path}");
        assert!(
            !paths(*owner).iter().any(|p| p == path),
            "{path} has a lane"
        );
    }
}

#[test]
fn with_effects_in_off_the_lane_of_the_switch_of_an_insert_turns_none_on() {
    // A Gain of -20 dB, switched off, whose lane switches it on: with Effects In on, it is heard
    // at -20 dB; with Effects In off, not at all.
    let mut project = routed();
    let audio = &mut project.timelines[0].tracks[0].audio;
    audio.strip.inserts[0].on = false;
    audio.strip.inserts[0].effect = Effect::Gain(GainFx { gain_db: -20.0 });
    audio.lanes.insert(
        "insert.fx1.on".into(),
        lane(LaneKind::Switch, 48_000, &[(0, 1.0)]),
    );
    let on = heard(&project, 2000, 0.5)[1000];
    assert!((f64::from(on) - 0.5 * gain_of(-20.0)).abs() < 1e-6, "{on}");
    project.timelines[0].tracks[0].audio.strip.inserts_on = false;
    let off = heard(&project, 2000, 0.5)[1000];
    assert!((f64::from(off) - 0.5).abs() < 1e-6, "{off}");
}

#[test]
fn a_lane_names_what_it_is_of_and_says_whether_a_new_graph_is_needed_for_another() {
    use columbina_audio::graph::{lanes_need_a_new_graph, Owner};
    let mut project = routed();
    let timeline = &mut project.timelines[0];
    // A second output, so that the pan is read after each output's delay.
    timeline.tracks[0].audio.strip.outputs.push(Output {
        bus: "B2".into(),
        on: true,
    });
    let flat = |value| lane(LaneKind::Continuous, 48_000, &[(0, value)]);
    let audio = &mut timeline.tracks[0].audio;
    audio.lanes.insert("pan.x".into(), flat(0.5));
    audio.lanes.insert("fader_db".into(), flat(-6.0));
    timeline.vcas[0].lanes.insert("fader_db".into(), flat(-3.0));
    let graph = built(&project);
    let a1 = Owner::Track("A1".into());
    assert_eq!(graph.reads_of_lane(&a1, "fader_db"), 1);
    assert_eq!(graph.reads_of_lane(&a1, "pan.x"), 2);
    assert_eq!(
        graph.reads_of_lane(&Owner::Vca("VCA1".into()), "fader_db"),
        1
    );
    assert_eq!(graph.reads_of_lane(&a1, "mute"), 0);
    // The fader's lane may be put in place of another as it plays; a lane of a node's settings,
    // or one added, needs a new graph.
    let audio = &project.timelines[0].tracks[0].audio;
    let before = audio.lanes.clone();
    let mut after = before.clone();
    after.insert("fader_db".into(), flat(-12.0));
    assert!(!lanes_need_a_new_graph(
        &audio.strip,
        &before,
        &after,
        48_000
    ));
    after.insert("insert.fx1.gain_db".into(), flat(3.0));
    assert!(lanes_need_a_new_graph(
        &audio.strip,
        &before,
        &after,
        48_000
    ));
    let mut eq = before.clone();
    eq.insert("eq.on".into(), lane(LaneKind::Switch, 48_000, &[(0, 0.0)]));
    let mut eq_on = eq.clone();
    eq_on.insert("eq.on".into(), lane(LaneKind::Switch, 48_000, &[(0, 1.0)]));
    assert!(lanes_need_a_new_graph(&audio.strip, &eq, &eq_on, 48_000));
}
