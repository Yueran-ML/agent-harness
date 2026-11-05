//! What the meters show while the timeline plays (plan T9b): a sine of −20 dBFS reads −20 before a
//! fader of −6 dB and −26 after it, by its peak (IEC 60268-18), its VU and its RMS, and the
//! Control Room meter reads the sum of the channels of the bus that is monitored; the loudness of the
//! bus that is monitored reads −23 LUFS for a tone of Tech 3341's case 1 played through the thread
//! of the mix; across a graph swapped in, the meters go on without a jump and the loudness counts
//! every cell once; the meters fall when nothing plays, and the loudness hears nothing then; the
//! loudness meter's Start, Pause, Reset and Lock Metering to Transport.

mod session;

use std::ops::Range;

use columbina_audio::feed::FeedError;
use columbina_audio::graph::{Graph, Options, Owner, Purpose};
use columbina_audio::meter::{Detector, MeterType, PeakIndicator, Scale};
use columbina_audio::metering::Board;
use columbina_audio::play::engine::MOST_BLOCK;
use columbina_audio::stream::Resampler;
use columbina_model::{Project, TrackKind};

use session::{one_track, Sim, Tone, RATE};

/// The sample project with its track of sound only: A1 (stereo), its clip c2, out to B1, the bus
/// that is monitored.
fn base() -> Project {
    let mut project = Project::sample();
    project.timelines[0]
        .tracks
        .retain(|track| track.kind == TrackKind::Audio);
    project
}

/// A sine of 1 kHz at a peak of `db` dBFS on every channel, by the sample of the media.
fn sine(db: f64) -> impl FnMut(usize, Range<i64>, &mut [&mut [f32]]) -> Result<(), FeedError> {
    let tone = Tone {
        hz: 1000.0,
        #[allow(clippy::cast_possible_truncation, reason = "a level")]
        amplitude: 10_f64.powf(db / 20.0) as f32,
    };
    move |_, range, out| {
        for out in out.iter_mut() {
            for (sample, value) in (range.start..).zip(out.iter_mut()) {
                *value = tone.at(sample);
            }
        }
        Ok(())
    }
}

/// `seconds` of `graph` from the start in blocks of 480, `board` read every 1600 samples (30 a
/// second), as the server reads it.
fn play(
    graph: &mut Graph,
    board: &mut Board,
    feed: &mut impl FnMut(usize, Range<i64>, &mut [&mut [f32]]) -> Result<(), FeedError>,
    seconds: f64,
) {
    graph.start(0, feed).unwrap();
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "samples of a test"
    )]
    let blocks = (seconds * f64::from(RATE) / 480.0) as usize;
    for block in 1..=blocks {
        graph.process(feed, 480).unwrap();
        if (block * 480) % 1600 < 480 {
            board.tick(1.0 / 30.0);
        }
    }
    board.tick(1.0 / 30.0);
}

fn close(got: f64, want: f64, tolerance: f64, what: &str) {
    assert!(
        (got - want).abs() <= tolerance,
        "{what}: {got} is not {want} ± {tolerance}"
    );
}

#[test]
fn a_sine_of_minus_20_dbfs_reads_minus_20_before_a_fader_of_minus_6_and_minus_26_after_it() {
    let mut project = base();
    project.timelines[0].tracks[0].audio.strip.fader_db = -6.0;
    for (meter, want) in [
        // IEC 60268-18: the peak of the samples; 1 kHz at 48 kHz has a sample on each crest.
        (MeterType::Iec60268_18, -20.0),
        // Digital VU: the RMS of a sine, 3.01 dB under its peak, settled after 1 s.
        (MeterType::DigitalVu, -23.0103),
        (
            MeterType::Custom {
                detector: Detector::Rms,
                scale: Scale::QuasiLinear,
                decay: 1.7,
                peak: PeakIndicator::MediumHoldFall,
            },
            -23.0103,
        ),
    ] {
        let mut graph = Graph::build(&project, 0, Options::new(4096, Purpose::Preview)).unwrap();
        graph.set_metering(true);
        let mut board = Board::new(meter);
        board.offer(graph.taps(), true);
        play(&mut graph, &mut board, &mut sine(-20.0), 1.0);
        let strips = board.strips();
        assert_eq!(strips.len(), 2);
        assert_eq!(strips[0].owner, Owner::Track("A1".into()));
        assert_eq!(strips[1].owner, Owner::Bus("B1".into()));
        for channel in 0..2 {
            let pre = strips[0].pre.shown(channel);
            let post = strips[0].post.shown(channel);
            let bus = strips[1].post.shown(channel);
            close(pre.bar, want, 0.01, &format!("{meter:?} before the fader"));
            close(post.bar, want - 6.0, 0.01, &format!("{meter:?} after it"));
            close(bus.bar, want - 6.0, 0.01, &format!("{meter:?} of the bus"));
            // The peak indicator holds the peak of the samples whatever the bar shows.
            close(pre.peak.unwrap(), -20.0, 0.01, "the peak before the fader");
            close(post.peak.unwrap(), -26.0, 0.01, "the peak after it");
        }
        // The Control Room meter: the sum of the two sides of B1, 6.02 dB over each.
        let room = board.room().unwrap().shown(0).bar;
        close(
            room,
            want - 6.0 + 20.0 * 2_f64.log10(),
            0.01,
            "the Control Room",
        );
        // The loudness of B1 (stereo, each side −26 dBFS at its peak): −26 LUFS.
        let reading = board.reading();
        close(reading.momentary, -26.0, 0.1, "Momentary");
        close(reading.sample_peak, -26.0, 0.01, "the sample peak");
        assert_eq!(board.lost(), 0);
    }
}

#[test]
fn what_a_run_throws_away_before_its_first_sample_is_not_metered() {
    // An equalizer that settles: a run that starts at 1 s prerolls before it.
    let mut project = base();
    project.timelines[0].tracks[0].audio.strip.eq.on = true;
    let mut graph = Graph::build(&project, 0, Options::new(4096, Purpose::Preview)).unwrap();
    assert!(graph.preroll() > 4800);
    graph.set_metering(true);
    let mut taps = graph.taps();
    let mut feed = sine(-20.0);
    graph.start(48_000, &mut feed).unwrap();
    assert!(graph.metering());
    for _ in 0..10 {
        graph.process(&mut feed, 480).unwrap();
    }
    // Ten cells of what was played, of every tap, and nothing of the preroll.
    assert_eq!(taps.meters[0].pre.waiting(), 10);
    assert_eq!(taps.meters[1].post.waiting(), 10);
    assert_eq!(taps.loudness.as_ref().unwrap().waiting(), 10);
    assert_eq!(taps.room.as_ref().unwrap().waiting(), 10);
    let mut peaks = Vec::new();
    taps.meters[0].pre.drain(|cell| peaks.push(cell.peaks[0]));
    assert!(
        peaks.iter().all(|&peak| (peak - 0.1).abs() < 0.01),
        "{peaks:?}"
    );
}

#[test]
fn the_meters_fall_when_nothing_plays_and_the_loudness_hears_nothing_then() {
    let mut graph = Graph::build(&base(), 0, Options::new(4096, Purpose::Preview)).unwrap();
    graph.set_metering(true);
    let mut board = Board::new(MeterType::Iec60268_18);
    board.offer(graph.taps(), true);
    play(&mut graph, &mut board, &mut sine(-20.0), 1.0);
    let seconds = board.reading().seconds;
    close(board.strips()[0].post.shown(0).bar, -20.0, 0.01, "playing");
    // A read with nothing to read falls nothing yet (the mix may be late by a block: `play` ends
    // with one, and this is the second) ...
    board.tick(1.0 / 30.0);
    close(
        board.strips()[0].post.shown(0).bar,
        -20.0,
        1e-6,
        "two reads late",
    );
    // ... but a pause of 0.1 s or more falls as a PPM falls on silence (20 dB in 1.7 s), from the
    // last cell, a cell of 10 ms for each 10 ms of it: 52 reads, 1.73 s, 173 cells.
    for _ in 0..50 {
        board.tick(1.0 / 30.0);
    }
    let fallen = board.strips()[0].post.shown(0).bar;
    close(
        fallen,
        -20.0 - 20.0 * 1.73 / 1.7,
        1e-6,
        "1.73 s after it stopped",
    );
    assert_eq!(board.reading().seconds.to_bits(), seconds.to_bits());
    // It plays again: the meter rises at once.
    play(&mut graph, &mut board, &mut sine(-20.0), 0.1);
    close(board.strips()[0].post.shown(0).bar, -20.0, 0.01, "again");
}

#[test]
fn start_pause_reset_and_lock_metering_to_transport() {
    let project = base();
    let mut board = Board::new(MeterType::Iec60268_18);
    let session = |board: &mut Board, restart: bool, seconds: f64| {
        let mut graph = Graph::build(&project, 0, Options::new(4096, Purpose::Preview)).unwrap();
        graph.set_metering(true);
        board.offer(graph.taps(), restart);
        play(&mut graph, board, &mut sine(-23.0), seconds);
    };
    session(&mut board, true, 1.0);
    close(board.reading().seconds, 1.0, 0.011, "a second heard");
    // Paused: it plays on and is not heard.
    board.pause();
    assert!(!board.running());
    session(&mut board, false, 1.0);
    close(board.reading().seconds, 1.0, 0.011, "nothing more heard");
    board.start();
    session(&mut board, false, 0.5);
    close(board.reading().seconds, 1.5, 0.021, "heard again");
    // A session that starts where the playhead was put clears it, Lock Metering to Transport on.
    session(&mut board, true, 0.5);
    close(
        board.reading().seconds,
        0.5,
        0.011,
        "cleared by a new session",
    );
    board.set_lock(false);
    session(&mut board, true, 0.5);
    close(
        board.reading().seconds,
        1.0,
        0.021,
        "kept, Lock Metering to Transport off",
    );
    board.reset();
    assert!(board.reading().seconds.abs() < f64::EPSILON);
    assert!(board.reading().integrated.is_none());
}

/// The playback of [`one_track`] with every clip a sine of 1 kHz at −23 dBFS on both sides
/// (Tech 3341, case 1: −23 LUFS), read by a board of `meter` 30 times a second for 12 s, with
/// another graph of the same project swapped in after 5 s. What the board read after each tick
/// of the device, the post-fader bar of A1, from 1 s on; and how many samples the graphs metered.
fn played_through_a_swap(meter: MeterType) -> (Board, Vec<f64>, i64) {
    let tone = Tone {
        hz: 1000.0,
        #[allow(clippy::cast_possible_truncation, reason = "a level")]
        amplitude: 10_f64.powf(-23.0 / 20.0) as f32,
    };
    let project = one_track();
    let mut sim = Sim::of(&project, 0, tone);
    let mut board = Board::new(meter);
    board.offer(sim.taps[0].clone(), true);
    let mut bars = Vec::new();
    let mut swap = 0;
    for tick in 0..1200 {
        if tick == 500 {
            swap = sim.swap(&project, 2_400);
            board.offer(sim.taps[1].clone(), false);
        }
        sim.tick();
        if tick % 3 == 2 {
            board.tick(0.030);
            if tick >= 100 {
                bars.push(board.strips()[0].post.shown(0).bar);
            }
        }
    }
    board.tick(0.030);
    assert_eq!(sim.counters.swaps(), 1);
    assert_eq!(sim.counters.underruns(), 0);
    // The first graph meters from where the session's graph starts (the reach of the device's
    // filter before sample 0) to the swap, the second from the swap; a cell under way at the swap
    // is not finished. Not one of the crossfade twice, nor of the preroll.
    let mut resampler = Resampler::new(RATE, RATE, 2, MOST_BLOCK);
    resampler.start(0);
    let first = resampler.first_input();
    let cells = |samples: i64| samples / 480;
    let metered = 480 * (cells(swap - first) + cells(sim.counters.mixed() - swap));
    (board, bars, metered)
}

#[test]
fn the_loudness_of_the_bus_heard_reads_minus_23_lufs_through_playback_and_a_swap() {
    let (board, bars, metered) = played_through_a_swap(MeterType::Iec60268_18);
    let reading = board.reading();
    println!(
        "through playback: I {:?}, M {}, S {}, LRA {:?}, TP {}, {} s heard of {} metered",
        reading.integrated,
        reading.momentary,
        reading.short_term,
        reading.range,
        reading.true_peak,
        reading.seconds,
        metered
    );
    close(reading.integrated.unwrap(), -23.0, 0.1, "Integrated");
    close(reading.momentary, -23.0, 0.1, "Momentary");
    close(reading.short_term, -23.0, 0.1, "Short Term");
    close(reading.short_term_max, -23.0, 0.1, "Short Max");
    close(reading.true_peak, -23.0, 0.2, "the true peak");
    // Every cell once.
    #[allow(clippy::cast_precision_loss, reason = "cells of a test")]
    let want = metered as f64 / f64::from(RATE);
    close(reading.seconds, want, 1e-9, "the seconds heard");
    assert_eq!(board.lost(), 0);
    for bar in bars {
        close(bar, -23.0, 0.01, "the bar of A1");
    }
}

#[test]
fn the_meters_go_on_across_a_graph_swapped_in_without_a_jump() {
    // A VU rises over 300 ms: a meter made anew for the graph swapped in would sink and rise.
    let (_, bars, _) = played_through_a_swap(MeterType::DigitalVu);
    let low = bars.iter().copied().fold(f64::INFINITY, f64::min);
    let high = bars.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    println!("the VU of A1 from 1 s to 12 s: {low} to {high}");
    close(low, -26.0103, 0.01, "the lowest");
    close(high, -26.0103, 0.01, "the highest");
}
