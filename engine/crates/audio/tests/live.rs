//! Values set live while the sound plays (plan T16, the recording of automation): a lane that
//! gives way to the value of a control that is touched, within 10 ms, and takes back over a
//! glide when it is let go; a lane moved by an offset (Trim); a switch held at once; a setting
//! with no lane that glides back to where it was set; and all of it the same however the blocks
//! are cut.

#![allow(
    clippy::float_cmp,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::manual_midpoint,
    reason = "samples of a test signal, compared to the bit where they are to be equal"
)]

use std::ops::Range;

use columbina_audio::feed::{ClipFeed, FeedError};
use columbina_audio::graph::{Graph, Live, Options, Owner, Purpose};
use columbina_model::{Lane, LaneKind, LanePoint, Project, Time, TrackKind, Vca};

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
    Graph::build(project, 0, Options::new(4096, Purpose::Preview)).unwrap()
}

/// A feed that gives 0.5 for every sample of every clip.
fn half() -> impl FnMut(usize, Range<i64>, &mut [&mut [f32]]) -> Result<(), FeedError> {
    |_, _, out| {
        for out in out.iter_mut() {
            out.fill(0.5);
        }
        Ok(())
    }
}

/// A lane of points at `(seconds of 48 kHz ticks, value)`.
fn lane(kind: LaneKind, points: &[(i64, f64)]) -> Lane {
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
        kind,
        scale: None,
    }
}

/// The left channel of the monitor for `frames` more samples, in blocks of `block`.
fn more(graph: &mut Graph, feed: &mut dyn ClipFeed, frames: usize, block: usize) -> Vec<f32> {
    let output = graph.monitor().unwrap();
    let mut got = Vec::with_capacity(frames);
    let mut done = 0;
    while done < frames {
        let n = block.min(frames - done);
        graph.process(feed, n).unwrap();
        got.extend_from_slice(&graph.output(output).channel(0)[..n]);
        done += n;
    }
    got
}

fn db(sample: f32) -> f64 {
    20.0 * (f64::from(sample) / 0.5).log10()
}

fn a1() -> Owner {
    Owner::Track("A1".into())
}

#[test]
fn a_fader_held_live_takes_the_place_of_its_lane_within_10_ms_and_glides_back_when_let_go() {
    let mut project = base();
    project.timelines[0].tracks[0]
        .audio
        .lanes
        .insert("fader_db".into(), lane(LaneKind::Continuous, &[(0, -10.0)]));
    let mut graph = built(&project);
    let key = graph
        .lane_key(&a1(), "fader_db")
        .expect("a lane of the fader");
    assert_eq!(graph.controls().lane_key(&a1(), "fader_db"), Some(key));
    assert_eq!(graph.lane_key(&a1(), "pan.x"), None, "no lane of the pan");
    let mut feed = half();
    graph.start(0, &mut feed).unwrap();
    let before = more(&mut graph, &mut feed, 4800, 480);
    assert!(before.iter().all(|&s| (db(s) + 10.0).abs() < 1e-4));
    // Touched at −6 dB: there 10 ms later (480 samples), straight in decibels on the way.
    graph.live(key, Live::Hold(-6.0));
    let held = more(&mut graph, &mut feed, 4800, 512);
    for (n, &sample) in held.iter().enumerate().take(480) {
        let share = f64::from(u32::try_from(n + 1).unwrap()) / 480.0;
        let want = -10.0 + 4.0 * share;
        assert!((db(sample) - want).abs() < 1e-3, "{n}: {} dB", db(sample));
    }
    assert!(held[480..].iter().all(|&s| (db(s) + 6.0).abs() < 1e-4));
    // A value that moves ramps over 10 ms from where it was.
    graph.live(key, Live::Hold(-4.0));
    let moved = more(&mut graph, &mut feed, 960, 100);
    assert!((db(moved[239]) + 5.0).abs() < 1e-3, "{}", db(moved[239]));
    assert!((db(moved[959]) + 4.0).abs() < 1e-4);
    // Let go with a glide of 250 ms: back to the lane on a straight line, the lane alone after.
    graph.live(key, Live::Release(12_000));
    let back = more(&mut graph, &mut feed, 14_000, 333);
    for n in [0usize, 2_999, 5_999, 11_998] {
        let share = f64::from(u32::try_from(n + 1).unwrap()) / 12_000.0;
        let want = -4.0 - 6.0 * share;
        assert!((db(back[n]) - want).abs() < 1e-3, "{n}: {} dB", db(back[n]));
    }
    assert!(back[11_999..].iter().all(|&s| (db(s) + 10.0).abs() < 1e-4));
    // A run started again holds nothing.
    graph.live(key, Live::Hold(-6.0));
    graph.start(0, &mut feed).unwrap();
    let again = more(&mut graph, &mut feed, 960, 480);
    assert!(again.iter().all(|&s| (db(s) + 10.0).abs() < 1e-4));
}

#[test]
fn a_fader_trimmed_live_moves_its_lane_by_the_offset_and_leaves_minus_infinity_where_it_is() {
    let mut project = base();
    // −10 dB to 0 dB over 4 s, then −∞ (−100 dB) from 1.5 s on for a while.
    project.timelines[0].tracks[0].audio.lanes.insert(
        "fader_db".into(),
        lane(
            LaneKind::Continuous,
            &[
                (0, -10.0),
                (48_000, -100.0),
                (96_000, -100.0),
                (192_000, 0.0),
            ],
        ),
    );
    let mut graph = built(&project);
    let key = graph.lane_key(&a1(), "fader_db").unwrap();
    let mut feed = half();
    graph.start(0, &mut feed).unwrap();
    let _ = more(&mut graph, &mut feed, 24_000, 480);
    graph.live(key, Live::Offset(3.0));
    let trimmed = more(&mut graph, &mut feed, 120_000, 480);
    // At 0.75 s the lane is −55 dB: heard at −52.
    let at = |seconds: f64, got: &[f32]| {
        let n = (seconds * 48_000.0) as usize - 24_000;
        db(got[n])
    };
    let lane_at = |seconds: f64| {
        if seconds < 1.0 {
            -10.0 - 90.0 * seconds
        } else if seconds < 2.0 {
            -100.0
        } else {
            -100.0 + 100.0 * (seconds - 2.0) / 2.0
        }
    };
    for seconds in [0.75, 0.9, 2.5] {
        let want = lane_at(seconds) + 3.0;
        assert!(
            (at(seconds, &trimmed) - want).abs() < 2e-3,
            "{seconds} s: {} dB, not {want}",
            at(seconds, &trimmed)
        );
    }
    // Where the lane is at −∞ it stays silent.
    assert_eq!(trimmed[(1.5 * 48_000.0) as usize - 24_000], 0.0, "silent at −∞");
}

#[test]
fn a_mute_held_live_switches_at_once_and_a_release_gives_it_back_at_once() {
    let mut project = base();
    project.timelines[0].tracks[0]
        .audio
        .lanes
        .insert("mute".into(), lane(LaneKind::Switch, &[(0, 0.0)]));
    let mut graph = built(&project);
    let key = graph.lane_key(&a1(), "mute").unwrap();
    let mut feed = half();
    graph.start(0, &mut feed).unwrap();
    let _ = more(&mut graph, &mut feed, 960, 480);
    graph.live(key, Live::Hold(1.0));
    let muted = more(&mut graph, &mut feed, 960, 480);
    assert!(
        muted.iter().all(|&s| s == 0.0),
        "muted from the first sample"
    );
    graph.live(key, Live::Release(12_000));
    let heard = more(&mut graph, &mut feed, 960, 480);
    assert!(heard.iter().all(|&s| s == 0.5), "a switch does not glide");
}

#[test]
fn a_vca_held_live_reaches_its_member_and_a_key_of_another_graph_does_nothing() {
    let mut project = base();
    let t = &mut project.timelines[0];
    t.vcas.push(Vca {
        id: "V1".into(),
        name: "VCA 1".into(),
        fader_db: 0.0,
        mute: false,
        solo: false,
        vcas: vec![],
        lanes: [(
            "fader_db".to_owned(),
            lane(LaneKind::Continuous, &[(0, 0.0)]),
        )]
        .into_iter()
        .collect(),
    });
    t.tracks[0].audio.strip.vcas.push("V1".into());
    let mut graph = built(&project);
    let other = built(&project);
    let vca = Owner::Vca("V1".into());
    let key = graph.lane_key(&vca, "fader_db").unwrap();
    let theirs = other.lane_key(&vca, "fader_db").unwrap();
    assert_ne!(key, theirs);
    let mut feed = half();
    graph.start(0, &mut feed).unwrap();
    graph.live(theirs, Live::Hold(-20.0));
    let same = more(&mut graph, &mut feed, 960, 480);
    assert!(same.iter().all(|&s| s == 0.5));
    graph.live(key, Live::Hold(-20.0));
    let held = more(&mut graph, &mut feed, 960, 480);
    assert!((db(held[959]) + 20.0).abs() < 1e-4);
}

#[test]
fn a_fader_with_no_lane_glides_back_to_where_it_was_set() {
    let mut project = base();
    project.timelines[0].tracks[0].audio.strip.fader_db = -10.0;
    let mut graph = built(&project);
    let control = graph.control(&a1(), "fader_db").unwrap();
    let mut feed = half();
    graph.start(0, &mut feed).unwrap();
    graph.set(control, -6.0);
    let _ = more(&mut graph, &mut feed, 960, 480);
    graph.glide(control, -10.0, 12_000);
    let back = more(&mut graph, &mut feed, 12_480, 480);
    // A gain glides straight in the gain.
    let (from, to) = (10f64.powf(-6.0 / 20.0), 10f64.powf(-10.0 / 20.0));
    for n in [0usize, 5_999, 11_998] {
        let share = f64::from(u32::try_from(n + 1).unwrap()) / 12_000.0;
        let want = 0.5 * (from + (to - from) * share);
        assert!((f64::from(back[n]) - want).abs() < 1e-6, "{n}");
    }
    assert!(back[12_000..].iter().all(|&s| (db(s) + 10.0).abs() < 1e-4));
}

#[test]
fn what_is_set_live_sounds_the_same_however_the_blocks_are_cut() {
    let mut project = base();
    project.timelines[0].tracks[0].audio.lanes.insert(
        "fader_db".into(),
        lane(LaneKind::Continuous, &[(0, -10.0), (192_000, 0.0)]),
    );
    let run = |blocks: &[usize]| {
        let mut graph = built(&project);
        let key = graph.lane_key(&a1(), "fader_db").unwrap();
        let mut feed = half();
        graph.start(0, &mut feed).unwrap();
        let mut got = Vec::new();
        let mut cut = blocks.iter().cycle();
        // The same sample for each command, the blocks cut at it.
        for (at, live) in [
            (10_000, Some(Live::Hold(-3.0))),
            (20_000, Some(Live::Hold(-1.0))),
            (30_000, Some(Live::Release(12_000))),
            (60_000, Some(Live::Offset(2.0))),
            (70_000, Some(Live::Release(500))),
            (80_000, None),
        ] {
            while got.len() < at {
                let n = (*cut.next().unwrap()).min(at - got.len());
                got.extend(more(&mut graph, &mut feed, n, n));
            }
            if let Some(live) = live {
                graph.live(key, live);
            }
        }
        got
    };
    let one = run(&[4096]);
    let other = run(&[1, 7, 480, 333, 4096, 64]);
    assert_eq!(one.len(), other.len());
    assert!(one
        .iter()
        .zip(&other)
        .all(|(a, b)| a.to_bits() == b.to_bits()));
}

#[test]
fn a_value_held_live_takes_the_place_of_a_lane_read_at_another_speed_too() {
    use columbina_audio::graph::Warp;
    // A lane that ramps, read at speed 2 (T18): held, the value; let go, the lane again.
    let mut project = base();
    project.timelines[0].tracks[0].audio.lanes.insert(
        "fader_db".into(),
        lane(LaneKind::Continuous, &[(0, -10.0), (192_000, 0.0)]),
    );
    let mut graph = built(&project);
    graph.set_warp(Warp::new(2, 0, 0));
    let key = graph.lane_key(&a1(), "fader_db").unwrap();
    let mut feed = half();
    graph.start(0, &mut feed).unwrap();
    let read = more(&mut graph, &mut feed, 4_800, 480);
    // At speed 2, sample 4799 out is at 9598 of the timeline: −10 + 10 · 9598 / 192000 dB.
    assert!((db(read[4_799]) - (-10.0 + 10.0 * 9_598.0 / 192_000.0)).abs() < 1e-3);
    graph.live(key, Live::Hold(-30.0));
    let held = more(&mut graph, &mut feed, 960, 480);
    assert!((db(held[959]) + 30.0).abs() < 1e-3, "{}", db(held[959]));
    graph.live(key, Live::Release(0));
    let back = more(&mut graph, &mut feed, 480, 480);
    let at = 2.0 * f64::from(4_800 + 960 + 479);
    assert!((db(back[479]) - (-10.0 + 10.0 * at / 192_000.0)).abs() < 1e-3);
}
