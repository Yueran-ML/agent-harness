//! The graph built from a project (plan T4b): the mixer of phase 2 moved into it (crossfades,
//! gain and pan, speed, the filter of a speed-up), and `FlexBus`: outputs, sends, buses into
//! buses, VCAs, mute and solo, the compensation of delays, pan into formats, the lanes, the
//! nodes of a strip, the sidechain and its Listen, the levels, the settling, and the output that
//! does not depend on its blocks.

use std::f64::consts::FRAC_1_SQRT_2;
use std::ops::Range;

use columbina_audio::feed::{ClipFeed, FeedError};
use columbina_audio::fx::{dynamics, eq};
use columbina_audio::graph::{Graph, Made, Options, Owner, Purpose};
use columbina_audio::node::{Block, Node, ParamId, Spec, Unsupported};
use columbina_audio::testing::{Delay, Rng};
use columbina_model::audio::fx::GainFx;
use columbina_model::audio::{BusFormat, ChannelMap, ChannelOrder, ChannelSource, TrackFormat};
use columbina_model::{
    Alignment, Bus, ChannelSources, CrossfadeLevel, Effect, Insert, Lane, LaneKind, LanePoint,
    Output, Project, Send, SourceRef, Speed, Time, Track, TrackKind, Transition, TransitionKind,
    Vca,
};

// --- Helpers ---------------------------------------------------------------------------------

/// The sample project with its track of sound only: A1, with clip c2 of 4 s of media m1 (stereo,
/// 48 kHz) from the start of the timeline, out to B1.
fn base() -> Project {
    let mut project = Project::sample();
    project.timelines[0]
        .tracks
        .retain(|track| track.kind == TrackKind::Audio);
    project
}

/// The project with `count` tracks like A1: A1, A2, ..., their clips c1, c2, ...
fn tracks(count: usize) -> Project {
    let mut project = base();
    let first = project.timelines[0].tracks[0].clone();
    project.timelines[0].tracks = (1..=count)
        .map(|n| {
            let mut track = first.clone();
            track.id = format!("A{n}");
            track.clips[0].id = format!("c{n}");
            track
        })
        .collect();
    project
}

/// A bus of `id`, stereo, out to `outputs`.
fn bus(id: &str, outputs: &[&str]) -> Bus {
    let mut bus = Bus::first();
    bus.id = id.into();
    bus.name = id.into();
    bus.strip.outputs = outputs
        .iter()
        .map(|bus| Output {
            bus: (*bus).into(),
            on: true,
        })
        .collect();
    bus
}

fn built(project: &Project, purpose: Purpose) -> Graph {
    Graph::build(project, 0, Options::new(4096, purpose)).unwrap()
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
                *sample = signal(&ids[clip], range.start + i64::try_from(n).unwrap(), channel);
            }
        }
        Ok(())
    }
}

/// `frames` samples of output `output` (the monitor where none) from sample `at`, in blocks
/// as long as `cut` says: a channel after another.
fn play_output(
    graph: &mut Graph,
    feed: &mut dyn ClipFeed,
    output: Option<usize>,
    (at, frames): (i64, usize),
    cut: &mut dyn FnMut() -> usize,
) -> Vec<Vec<f32>> {
    graph.start(at, feed).unwrap();
    let output = output.or(graph.monitor()).unwrap();
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

/// [`play_output`] of the monitor in blocks of 4096.
fn play(graph: &mut Graph, feed: &mut dyn ClipFeed, at: i64, frames: usize) -> Vec<Vec<f32>> {
    play_output(graph, feed, None, (at, frames), &mut || 4096)
}

/// A clip's sound that is `value` on every channel.
fn level(value: f32) -> impl Fn(&str, i64, usize) -> f32 {
    move |_, _, _| value
}

fn close(got: f32, want: f64, tolerance: f64, what: &str) {
    assert!(
        (f64::from(got) - want).abs() <= tolerance,
        "{what}: {got} is not {want}"
    );
}

fn db(gain: f32) -> f32 {
    20.0 * gain.log10()
}

// --- The mixer of phase 2, moved -------------------------------------------------------------

#[test]
fn two_tones_crossfade_sample_by_sample_at_equal_power() {
    let mut project = base();
    let timeline = &mut project.timelines[0];
    let at = |ticks| Time {
        ticks,
        scale: 15360,
    };
    let first = columbina_model::Clip::of_media("c1", "m1", at(0), 0, 50);
    let second = columbina_model::Clip::of_media("c2", "m1", at(30720), 50, 50);
    let mut track = Track::new("A1", TrackKind::Audio).with_clips(vec![first, second]);
    track.transitions.push(Transition {
        id: "x1".into(),
        kind: TransitionKind::Crossfade,
        left: "c1".into(),
        right: "c2".into(),
        length: 10,
        align: Alignment::Center,
        fade_in_level_db: CrossfadeLevel::Plus3,
        fade_out_level_db: CrossfadeLevel::Plus3,
    });
    timeline.tracks = vec![track];
    let tone = |id: &str, sample: i64| {
        let phase = f64::from(i32::try_from(sample).unwrap());
        #[allow(clippy::cast_possible_truncation)]
        let value = if id == "c1" {
            0.2 * (phase * 0.007).sin()
        } else {
            0.3 * (phase * 0.009).cos()
        } as f32;
        value
    };
    let mut graph = built(&project, Purpose::Export);
    let mut feed = feed(&graph, move |id, sample, _| tone(id, sample));
    let got = play(&mut graph, &mut feed, 86400, 19200);
    for i in 0..19200 {
        let sample = 86400 + i64::try_from(i).unwrap();
        #[allow(clippy::cast_precision_loss)]
        let theta = i as f32 / 19200.0 * std::f32::consts::FRAC_PI_2;
        let want = tone("c1", sample) * theta.cos() + tone("c2", sample) * theta.sin();
        for channel in &got {
            close(
                channel[i],
                f64::from(want),
                1e-5,
                &format!("sample {sample}"),
            );
        }
    }
}

#[test]
#[allow(clippy::float_cmp, reason = "a hard pan drops the other side exactly")]
fn gain_and_pan_are_applied_and_a_track_that_is_off_is_not_read() {
    let mut project = base();
    project.timelines[0].tracks[0].clips[0].gain_db = (-6.0).into();
    project.timelines[0].tracks[0].clips[0].pan = (-1.0).into();
    let mut graph = built(&project, Purpose::Export);
    let mut read = feed(&graph, level(0.25));
    let got = play(&mut graph, &mut read, 0, 100);
    let half = 0.125 * std::f64::consts::SQRT_2;
    close(got[0][50], half, half * 0.01, "left");
    assert_eq!(got[1][50], 0.0);

    project.timelines[0].tracks[0].enabled = false;
    let mut graph = built(&project, Purpose::Export);
    assert_eq!(graph.clips().len(), 0);
    let mut never = |_: usize, _: Range<i64>, _: &mut [&mut [f32]]| -> Result<(), FeedError> {
        panic!("a track that is off must not be read")
    };
    let silent = play(&mut graph, &mut never, 0, 100);
    assert!(silent.iter().flatten().all(|&sample| sample == 0.0));
}

#[test]
#[allow(clippy::float_cmp, reason = "a muted track is silent to the bit")]
fn a_muted_track_is_read_but_not_heard() {
    let mut project = base();
    project.timelines[0].tracks[0].audio.mute = true;
    let mut graph = built(&project, Purpose::Export);
    assert_eq!(graph.clips().len(), 1);
    let mut read = feed(&graph, level(0.5));
    let silent = play(&mut graph, &mut read, 0, 1000);
    assert!(silent.iter().flatten().all(|&sample| sample == 0.0));
}

#[test]
#[allow(clippy::float_cmp, reason = "the clipped copy is exactly at the rails")]
fn the_sound_is_clipped_only_in_the_copy_for_a_device() {
    // Two tracks of 0.8 each: 1.6 out of the graph, 1.0 to a device.
    let project = tracks(2);
    let mut graph = built(&project, Purpose::Preview);
    let mut read = feed(&graph, level(0.8));
    let got = play(&mut graph, &mut read, 0, 100);
    for channel in &got {
        close(channel[10], 1.6, 1e-6, "the sum of the floats");
    }
    let mut device = vec![0.0; 200];
    graph.write_device(graph.monitor().unwrap(), 100, &mut device);
    assert!(device.iter().all(|&sample| sample == 1.0), "{device:?}");

    // A hot clip, as phase 2 tested it: [0.8, -0.8] at +24 dB is far beyond 1, and only the
    // device's copy is at the rails.
    let mut project = base();
    project.timelines[0].tracks[0].clips[0].gain_db = 24.0.into();
    let mut graph = built(&project, Purpose::Preview);
    let mut read = feed(
        &graph,
        |_, _, channel| if channel == 0 { 0.8 } else { -0.8 },
    );
    let got = play(&mut graph, &mut read, 0, 100);
    let hot = 0.8 * 10f64.powf(24.0 / 20.0);
    close(got[0][50], hot, 1e-4, "hot left");
    close(got[1][50], -hot, 1e-4, "hot right");
    graph.write_device(graph.monitor().unwrap(), 100, &mut device);
    for frame in device.as_chunks::<2>().0 {
        assert_eq!(*frame, [1.0, -1.0]);
    }

    // Opposing hot tracks cancel in the sum, before anything is clipped.
    let mut project = tracks(2);
    for track in &mut project.timelines[0].tracks {
        track.clips[0].gain_db = 24.0.into();
    }
    let mut graph = built(&project, Purpose::Preview);
    let mut read = feed(&graph, |clip, _, _| if clip == "c2" { -0.75 } else { 0.8 });
    let got = play(&mut graph, &mut read, 0, 100);
    let want = 0.05 * 10f64.powf(24.0 / 20.0);
    assert!(got
        .iter()
        .flatten()
        .all(|&value| (f64::from(value) - want).abs() < 1e-5));
}

#[test]
fn reverse_fractional_speed_preserves_sample_phase_across_blocks() {
    let mut project = base();
    let clip = &mut project.timelines[0].tracks[0].clips[0];
    clip.speed = Speed { num: 3, den: 2 };
    clip.reverse = true;
    let signal = |_: &str, sample: i64, channel: usize| {
        #[allow(clippy::cast_possible_truncation)]
        let value = (f64::from(i32::try_from(sample).unwrap()) / 1_000_000.0) as f32;
        if channel == 0 {
            value
        } else {
            -value
        }
    };
    let mut graph = built(&project, Purpose::Export);
    let mut read = feed(&graph, signal);
    let whole = play_output(&mut graph, &mut read, None, (101, 400), &mut || 400);
    let mut cuts = [91, 309].into_iter();
    let split = play_output(&mut graph, &mut read, None, (101, 400), &mut || {
        cuts.next().unwrap()
    });
    assert_eq!(whole, split);
    for (i, &left) in whole[0].iter().enumerate() {
        let offset = 101 + i64::try_from(i).unwrap();
        #[allow(clippy::cast_possible_truncation)]
        let want = (f64::from(i32::try_from(191_999 - offset).unwrap()) * 1.5 / 1_000_000.0) as f32;
        close(left, f64::from(want), 1e-5, &format!("sample {offset}"));
        assert_eq!(whole[1][i].to_bits(), (-left).to_bits());
    }
}

#[test]
fn speed_up_filters_frequencies_above_the_new_nyquist_limit() {
    let mut project = base();
    project.timelines[0].tracks[0].clips[0].speed = Speed { num: 2, den: 1 };
    let mut graph = built(&project, Purpose::Export);
    let mut read = feed(&graph, |_, n, _| if n % 2 == 0 { 0.5 } else { -0.5 });
    let got = play(&mut graph, &mut read, 100, 100);
    assert!(
        got.iter().flatten().all(|value| value.abs() < 0.002),
        "aliased Nyquist tone: {got:?}"
    );
}

/// How loud a mix is: the root of its mean square.
fn loudness(mix: &[f32]) -> f64 {
    let sum: f64 = mix.iter().map(|&value| f64::from(value).powi(2)).sum();
    #[allow(clippy::cast_precision_loss)]
    let count = mix.len() as f64;
    (sum / count).sqrt()
}

/// A clip at `speed` of a tone of `hertz` at 48 kHz and amplitude 0.5, mixed for 4800 samples
/// from sample 2400: its left channel.
fn sped(speed: u32, hertz: f64) -> Vec<f32> {
    let mut project = base();
    project.timelines[0].tracks[0].clips[0].speed = Speed { num: speed, den: 1 };
    let mut graph = built(&project, Purpose::Export);
    let mut read = feed(&graph, move |_, n, _| {
        let phase =
            2.0 * std::f64::consts::PI * hertz * f64::from(i32::try_from(n).unwrap()) / 48000.0;
        #[allow(clippy::cast_possible_truncation)]
        let value = (0.5 * phase.sin()) as f32;
        value
    });
    play(&mut graph, &mut read, 2400, 4800).swap_remove(0)
}

#[test]
fn at_16_times_tones_above_the_new_nyquist_frequency_do_not_fold_back() {
    // At 16 times, what is above 1500 Hz in the media would be above 24 kHz when heard.
    let mut hertz = 2000.0;
    while hertz < 20000.0 {
        let heard = loudness(&sped(16, hertz));
        // A tone of amplitude 0.5 is 0.35 loud; folded back, it must be 40 dB quieter.
        assert!(heard < 0.0035, "{hertz} Hz at 16 times is {heard} loud");
        hertz *= 1.21;
    }
    let heard = loudness(&sped(16, 600.0));
    assert!(
        (heard - 0.3535).abs() < 0.01,
        "600 Hz at 16 times is {heard}"
    );
    let heard = loudness(&sped(4, 3000.0));
    assert!(
        (heard - 0.3535).abs() < 0.01,
        "3000 Hz at 4 times is {heard}"
    );
}

#[test]
fn a_half_pan_of_a_stereo_clip_is_a_balance_of_its_two_channels() {
    let mut project = base();
    project.timelines[0].tracks[0].clips[0].pan = 0.5.into();
    let mut graph = built(&project, Purpose::Export);
    let mut read = feed(&graph, |_, _, channel| if channel == 0 { 0.2 } else { 0.1 });
    let got = play(&mut graph, &mut read, 0, 100);
    let (sin, cos) = std::f64::consts::FRAC_PI_8.sin_cos();
    let sqrt2 = std::f64::consts::SQRT_2;
    for (&left, &right) in got[0].iter().zip(&got[1]) {
        close(left, 0.2 * sqrt2 * sin, 1e-6, "left");
        close(right, 0.1 * sqrt2 * cos, 1e-6, "right");
    }
}

#[test]
fn pan_is_equal_power_between_the_hard_pans_and_keeps_its_sides() {
    use columbina_audio::graph::route::balance;
    let near = |got: [f32; 2], want: [f64; 2]| {
        for (got, want) in got.into_iter().zip(want) {
            close(got, want, 1e-6, "pan");
        }
    };
    let sqrt2 = std::f64::consts::SQRT_2;
    let (sin, cos) = std::f64::consts::FRAC_PI_8.sin_cos();
    near(balance(0.0), [1.0, 1.0]);
    near(balance(-0.5), [sqrt2 * cos, sqrt2 * sin]);
    near(balance(0.5), [sqrt2 * sin, sqrt2 * cos]);
    near(balance(-1.0), [sqrt2, 0.0]);
    near(balance(1.0), [0.0, sqrt2]);
    for value in [-0.9_f32, -0.5, -0.2, 0.0, 0.3, 0.5, 0.8] {
        let [left, right] = balance(value);
        assert!((left * left + right * right - 2.0).abs() < 1e-5, "{value}");
    }
}

// --- FlexBus ---------------------------------------------------------------------------------

#[test]
fn a_mono_track_into_a_stereo_bus_is_minus_3_db_a_side_at_the_centre() {
    let mut project = base();
    let track = &mut project.timelines[0].tracks[0];
    track.audio.format = TrackFormat::Mono;
    track.audio.channels = 1;
    let mut graph = built(&project, Purpose::Export);
    assert_eq!(graph.clips()[0].channels, 1);
    let mut read = feed(&graph, |_, n, _| {
        #[allow(clippy::cast_precision_loss)]
        let value = (n % 97) as f32 / 97.0 - 0.5;
        value
    });
    let got = play(&mut graph, &mut read, 0, 1000);
    for n in 0..1000 {
        #[allow(clippy::cast_precision_loss)]
        let input = f64::from((n % 97) as f32 / 97.0 - 0.5);
        for channel in &got {
            close(channel[n], FRAC_1_SQRT_2 * input, 1e-7, "mono into stereo");
        }
    }
}

#[test]
fn five_one_into_stereo_is_folded_down_by_bs_775() {
    let mut project = base();
    let track = &mut project.timelines[0].tracks[0];
    track.audio.format = TrackFormat::FiveOne;
    track.audio.channels = 6;
    let only = |wanted: usize| {
        let mut graph = built(&project, Purpose::Export);
        let mut read = feed(&graph, move |_, _, channel| {
            f32::from(u8::from(channel == wanted))
        });
        play(&mut graph, &mut read, 0, 100)
    };
    // The centre: 0.7071 a side.
    let centre = only(2);
    close(centre[0][50], FRAC_1_SQRT_2, 1e-6, "C into L");
    close(centre[1][50], FRAC_1_SQRT_2, 1e-6, "C into R");
    // The LFE: dropped.
    let lfe = only(3);
    assert!(lfe.iter().flatten().all(|&sample| sample == 0.0));
    // A surround: −3 dB into its side only.
    let surround = only(4);
    close(surround[0][50], FRAC_1_SQRT_2, 1e-6, "Ls into L");
    close(surround[1][50], 0.0, 0.0, "Ls into R");
}

#[test]
fn a_track_of_film_order_is_read_into_smpte() {
    let mut project = base();
    project.timelines[0].buses[0].format = BusFormat::FiveOne;
    let track = &mut project.timelines[0].tracks[0];
    track.audio.format = TrackFormat::FiveOne;
    track.audio.channels = 6;
    track.audio.order = ChannelOrder::Film;
    track.clips[0].channels = Some(
        (0..6)
            .map(|channel| ChannelMap {
                source: ChannelSources(vec![ChannelSource { stream: 0, channel }]),
                enabled: true,
                mute: false,
                level_db: 0.0,
            })
            .collect(),
    );
    let mut graph = built(&project, Purpose::Export);
    // The feed gives Film's order: L C R Ls Rs LFE as 1 to 6.
    #[allow(clippy::cast_precision_loss)]
    let mut read = feed(&graph, |_, _, channel| (channel + 1) as f32);
    let got = play(&mut graph, &mut read, 0, 10);
    let smpte: Vec<f32> = got.iter().map(|channel| channel[5]).collect();
    // L R C LFE Ls Rs, the pan of a 5.1 track into a 5.1 bus at the centre being one to one.
    assert_eq!(smpte, [1.0, 3.0, 2.0, 6.0, 4.0, 5.0]);
}

#[test]
fn a_send_after_the_fader_follows_it_and_one_before_does_not() {
    let mut project = base();
    let timeline = &mut project.timelines[0];
    timeline.buses.push(bus("B2", &[]));
    let strip = &mut timeline.tracks[0].audio.strip;
    strip.fader_db = -6.0;
    strip.sends.push(Send {
        bus: "B2".into(),
        on: true,
        pre: false,
        level_db: 0.0,
        pan: columbina_model::Pan::default(),
    });
    let sent = |project: &Project| {
        let mut graph = built(project, Purpose::Export);
        let b2 = graph.output_of("B2").unwrap();
        let mut read = feed(&graph, level(0.5));
        play_output(&mut graph, &mut read, Some(b2), (0, 100), &mut || 100)
    };
    close(
        db(sent(&project)[0][50] / 0.5),
        -6.0,
        0.01,
        "after the fader",
    );
    project.timelines[0].tracks[0].audio.strip.sends[0].pre = true;
    close(
        db(sent(&project)[0][50] / 0.5),
        0.0,
        0.01,
        "before the fader",
    );
    // A send before the fader is after the mute (C-B4).
    project.timelines[0].tracks[0].audio.mute = true;
    assert!(sent(&project).iter().flatten().all(|&s| s == 0.0));
}

#[test]
fn buses_feed_buses_six_deep() {
    let mut project = base();
    let timeline = &mut project.timelines[0];
    timeline.buses = (1..=6)
        .map(|n| {
            let next = format!("B{}", n + 1);
            if n == 6 {
                bus("B6", &[])
            } else {
                bus(&format!("B{n}"), &[next.as_str()])
            }
        })
        .collect();
    timeline.audio.monitor = Some("B6".into());
    for (n, bus) in timeline.buses.iter_mut().enumerate() {
        // −1 dB a bus: six of them, −6 dB.
        bus.strip.fader_db = -1.0;
        let _ = n;
    }
    let mut graph = built(&project, Purpose::Export);
    let outputs = graph.outputs();
    assert_eq!(outputs.len(), 1);
    assert!(outputs[0].main);
    assert_eq!(outputs[0].bus, "B6");
    let mut read = feed(&graph, level(0.5));
    let got = play(&mut graph, &mut read, 0, 100);
    close(db(got[0][50] / 0.5), -6.0, 1e-4, "through six buses");
}

#[test]
fn a_vca_adds_its_decibels_to_its_members_and_mutes_them() {
    let mut project = base();
    let timeline = &mut project.timelines[0];
    timeline.vcas.push(Vca {
        id: "VCA1".into(),
        name: "VCA 1".into(),
        fader_db: -4.0,
        mute: false,
        solo: false,
        vcas: vec![],
        lanes: columbina_model::Lanes::new(),
    });
    timeline.tracks[0].audio.strip.fader_db = -6.0;
    timeline.tracks[0].audio.strip.vcas = vec!["VCA1".into()];
    let gain = |project: &Project| {
        let mut graph = built(project, Purpose::Export);
        let mut read = feed(&graph, level(0.5));
        play(&mut graph, &mut read, 0, 100)[0][50] / 0.5
    };
    close(db(gain(&project)), -10.0, 0.01, "−6 and −4");
    // A VCA of a VCA adds its own.
    project.timelines[0].vcas.push(Vca {
        id: "VCA2".into(),
        name: "VCA 2".into(),
        fader_db: 3.0,
        mute: false,
        solo: false,
        vcas: vec![],
        lanes: columbina_model::Lanes::new(),
    });
    project.timelines[0].vcas[0].vcas = vec!["VCA2".into()];
    close(db(gain(&project)), -7.0, 0.01, "−6, −4 and +3");
    project.timelines[0].vcas[1].mute = true;
    assert!(gain(&project) == 0.0, "the mute of a VCA of a VCA");
}

/// The gain of each of three tracks of 0.1, 0.2 and 0.4 in the output, as `change` sets them.
fn heard(change: impl FnOnce(&mut Project)) -> f32 {
    let mut project = tracks(3);
    change(&mut project);
    let mut graph = built(&project, Purpose::Export);
    let mut read = feed(&graph, |clip, _, _| match clip {
        "c1" => 0.1,
        "c2" => 0.2,
        _ => 0.4,
    });
    play(&mut graph, &mut read, 0, 100)[0][50]
}

#[test]
fn a_solo_silences_what_is_not_soloed_nor_solo_safe() {
    close(heard(|_| {}), 0.7, 1e-6, "no solo");
    close(
        heard(|p| p.timelines[0].tracks[0].audio.solo = true),
        0.1,
        1e-6,
        "A1 soloed",
    );
    close(
        heard(|p| {
            p.timelines[0].tracks[0].audio.solo = true;
            p.timelines[0].tracks[1].audio.solo_safe = true;
        }),
        0.3,
        1e-6,
        "A1 soloed, A2 solo-safe",
    );
    close(
        heard(|p| {
            p.timelines[0].tracks[0].audio.solo = true;
            p.timelines[0].tracks[2].audio.solo = true;
        }),
        0.5,
        1e-6,
        "A1 and A3 soloed",
    );
    // A track that is off is not soloed.
    close(
        heard(|p| {
            p.timelines[0].tracks[0].audio.solo = true;
            p.timelines[0].tracks[0].enabled = false;
        }),
        0.6,
        1e-6,
        "A1, soloed, is off",
    );
    // A soloed bus: what feeds it and what it feeds are heard.
    close(
        heard(|p| {
            let timeline = &mut p.timelines[0];
            timeline.buses.push(bus("B2", &["B1"]));
            timeline.buses[1].solo = true;
            timeline.tracks[2].audio.strip.outputs[0].bus = "B2".into();
        }),
        0.4,
        1e-6,
        "B2 soloed, A3 into it",
    );
    // A soloed VCA solos its members.
    close(
        heard(|p| {
            let timeline = &mut p.timelines[0];
            timeline.vcas.push(Vca {
                id: "VCA1".into(),
                name: "VCA 1".into(),
                fader_db: 0.0,
                mute: false,
                solo: true,
                vcas: vec![],
                lanes: columbina_model::Lanes::new(),
            });
            timeline.tracks[1].audio.strip.vcas = vec!["VCA1".into()];
        }),
        0.2,
        1e-6,
        "VCA1 soloed, A2 of it",
    );
}

/// A maker of the nodes of the tests: a Gain effect of `n` decibels is a delay of `n` samples.
fn delays(effect: &Effect) -> Option<Made> {
    match effect {
        Effect::Gain(gain) => {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let samples = gain.gain_db as usize;
            Some(Made {
                node: Box::new(Delay::new(samples)),
                param: |_| None,
            })
        }
        _ => None,
    }
}

/// An insert of a delay of `samples` (see [`delays`]).
fn delay_insert(samples: u32) -> Insert {
    Insert {
        id: "fx1".into(),
        on: true,
        effect: Effect::Gain(GainFx {
            gain_db: f64::from(samples),
        }),
    }
}

#[test]
fn the_delays_of_the_paths_are_made_up_to_the_sample() {
    let mut project = tracks(2);
    project.timelines[0].tracks[0]
        .audio
        .strip
        .inserts
        .push(delay_insert(64));
    let impulses = |project: &Project| {
        let mut graph = Graph::build(
            project,
            0,
            Options::new(512, Purpose::Export).with_maker(delays),
        )
        .unwrap();
        let latency = graph.latency();
        let mut read = feed(&graph, |_, n, _| f32::from(u8::from(n == 1000)));
        let left = play(&mut graph, &mut read, 0, 3000).swap_remove(0);
        let at: Vec<(usize, f32)> = left
            .iter()
            .copied()
            .enumerate()
            .filter(|&(_, value)| value != 0.0)
            .collect();
        (latency, at)
    };
    assert_eq!(impulses(&project), (64, vec![(1000, 2.0)]));
    // With its ADC off, the track that is late stays late.
    project.timelines[0].tracks[0].audio.adc = false;
    assert_eq!(impulses(&project), (0, vec![(1000, 1.0), (1064, 1.0)]));
    // A bypassed insert still delays (the model's `Insert`): nothing moves.
    project.timelines[0].tracks[0].audio.adc = true;
    project.timelines[0].tracks[0].audio.strip.inserts[0].on = false;
    assert_eq!(impulses(&project), (64, vec![(1000, 2.0)]));
    // Through a bus: A1 into B2 into B1 with a delay of 100 on B2, A2 into B1.
    let mut project = tracks(2);
    let timeline = &mut project.timelines[0];
    let mut b2 = bus("B2", &["B1"]);
    b2.strip.inserts.push(delay_insert(100));
    timeline.buses.push(b2);
    timeline.tracks[0].audio.strip.outputs[0].bus = "B2".into();
    timeline.tracks[1]
        .audio
        .strip
        .inserts
        .push(delay_insert(30));
    assert_eq!(impulses(&project), (100, vec![(1000, 2.0)]));
}

/// A node that works with no channels.
struct Refusing;

impl Node for Refusing {
    fn prepare(&mut self, spec: &Spec) -> Result<(), Unsupported> {
        Err(Unsupported::from(spec))
    }
    fn process(&mut self, _: &Block, _: &[&[f32]], _: &mut [&mut [f32]]) {
        unreachable!("a node that was refused is not run")
    }
    fn latency(&self) -> u32 {
        1000
    }
    fn settle(&self) -> u32 {
        0
    }
    fn reset(&mut self) {}
    fn set(&mut self, _: ParamId, _: f32) {}
}

#[test]
fn an_insert_that_does_not_take_its_channels_is_left_out() {
    let mut project = base();
    project.timelines[0].tracks[0]
        .audio
        .strip
        .inserts
        .push(delay_insert(1));
    let refusing = |_: &Effect| {
        Some(Made {
            node: Box::new(Refusing),
            param: |_| None,
        })
    };
    let mut graph = Graph::build(
        &project,
        0,
        Options::new(256, Purpose::Export).with_maker(refusing),
    )
    .unwrap();
    assert_eq!(graph.latency(), 0);
    let mut read = feed(&graph, level(0.25));
    let got = play(&mut graph, &mut read, 0, 100);
    close(got[0][10], 0.25, 0.0, "passed as it is");
}

// --- Lanes, nodes, sidechain -----------------------------------------------------------------

/// A lane of `kind` through `points` (seconds, value), at 48 kHz.
fn lane(kind: LaneKind, points: &[(f64, f64)]) -> Lane {
    Lane {
        points: points
            .iter()
            .map(|&(at, value)| LanePoint {
                #[allow(clippy::cast_possible_truncation)]
                at: Time {
                    ticks: (at * 48000.0) as i64,
                    scale: 48000,
                },
                value,
            })
            .collect(),
        kind,
    }
}

#[test]
fn a_lane_of_the_fader_is_read_at_each_sample_and_a_lane_of_the_mute_switches_on_it() {
    let mut project = base();
    let audio = &mut project.timelines[0].tracks[0].audio;
    audio.strip.fader_db = 6.0;
    audio.lanes.insert(
        "fader_db".into(),
        lane(LaneKind::Continuous, &[(0.0, -10.0), (1.0, 0.0)]),
    );
    audio.lanes.insert(
        "mute".into(),
        lane(LaneKind::Switch, &[(0.0, 0.0), (1.5, 1.0)]),
    );
    let mut graph = built(&project, Purpose::Export);
    let mut read = feed(&graph, level(0.5));
    let got = play(&mut graph, &mut read, 0, 96000).swap_remove(0);
    close(db(got[12000] / 0.5), -7.5, 1e-5, "a quarter of a second in");
    close(db(got[24000] / 0.5), -5.0, 1e-5, "half a second in");
    close(db(got[60000] / 0.5), 0.0, 1e-5, "after the last point");
    assert!(got[71999] != 0.0 && got[72000] == 0.0, "the mute at 1.5 s");
}

#[test]
fn the_controls_are_found_by_the_paths_of_their_lanes() {
    let mut project = tracks(2);
    let timeline = &mut project.timelines[0];
    timeline.buses.push(bus("B2", &[]));
    let strip = &mut timeline.tracks[0].audio.strip;
    strip.dynamics.on = true;
    strip.eq.on = true;
    strip.inserts.push(delay_insert(4));
    strip.sends.push(Send {
        bus: "B2".into(),
        on: true,
        pre: false,
        level_db: 0.0,
        pan: columbina_model::Pan::default(),
    });
    let with_params = |effect: &Effect| {
        delays(effect).map(|made| Made {
            param: |path| (path == "gain_db").then_some(ParamId(7)),
            ..made
        })
    };
    let graph = Graph::build(
        &project,
        0,
        Options::new(256, Purpose::Export).with_maker(with_params),
    )
    .unwrap();
    let a1 = Owner::Track("A1".into());
    for path in [
        "fader_db",
        "trim_db",
        "mute",
        "invert",
        "pan.x",
        "solo",
        "dynamics.compressor.threshold_db",
        "dynamics.on",
        "eq.on",
        "eq.gain_db",
        "eq.2.gain_db",
        "eq.5.freq",
        "send.B2.level_db",
        "send.B2.on",
        "send.B2.pan.x",
        "output.B1.on",
        "insert.fx1.gain_db",
    ] {
        assert!(graph.control(&a1, path).is_some(), "{path}");
    }
    for path in [
        "dynamics.compressor.sidechain.listen",
        "dynamics.nothing",
        "eq.6.gain_db",
        "eq.2.kind",
        "send.B3.level_db",
        "output.B2.on",
        "insert.fx2.gain_db",
        "insert.fx1.ceiling_db",
        "track_fx.amount_db",
    ] {
        assert!(graph.control(&a1, path).is_none(), "{path}");
    }
    // A2 has neither Dynamics nor an equalizer built.
    let a2 = Owner::Track("A2".into());
    assert!(graph.control(&a2, "dynamics.on").is_none());
    assert!(graph.control(&a2, "eq.on").is_none());
    assert!(graph
        .control(&Owner::Bus("B1".into()), "fader_db")
        .is_some());
    assert!(graph
        .control(&Owner::Track("A9".into()), "fader_db")
        .is_none());
}

#[test]
fn a_fader_set_while_it_plays_ramps_over_10_ms() {
    let project = base();
    let mut graph = built(&project, Purpose::Preview);
    let mut read = feed(&graph, level(0.5));
    graph.start(0, &mut read).unwrap();
    graph.process(&mut read, 100).unwrap();
    let fader = graph
        .control(&Owner::Track("A1".into()), "fader_db")
        .unwrap();
    graph.set(fader, -100.0);
    graph.process(&mut read, 1000).unwrap();
    let out = graph.output(graph.monitor().unwrap()).channel(0);
    close(out[239], 0.25, 1e-6, "half way through the ramp");
    close(out[479], 0.0, 0.0, "at the end of the ramp");
}

#[test]
fn dynamics_and_an_equalizer_are_built_only_when_they_are_on() {
    let mut project = base();
    assert_eq!(built(&project, Purpose::Export).settle(), 0);
    let strip = &mut project.timelines[0].tracks[0].audio.strip;
    strip.eq.on = true;
    let rate = 48000;
    let eq_settle = eq::settle_for(
        &eq::Settings {
            on: true,
            ..eq::Settings::strip()
        },
        rate,
    );
    assert_eq!(built(&project, Purpose::Export).settle(), eq_settle);
    project.timelines[0].tracks[0].audio.strip.eq.on = false;
    // Dynamics off with a lane that turns its Compressor on: built, settled as on.
    let audio = &mut project.timelines[0].tracks[0].audio;
    audio
        .lanes
        .insert("dynamics.on".into(), lane(LaneKind::Switch, &[(1.0, 1.0)]));
    audio.lanes.insert(
        "dynamics.compressor.on".into(),
        lane(LaneKind::Switch, &[(2.0, 1.0)]),
    );
    let mut on = dynamics::Settings {
        on: true,
        ..dynamics::Settings::default()
    };
    on.compressor.on = true;
    assert_eq!(
        built(&project, Purpose::Export).settle(),
        dynamics::Dynamics::settle_for(&on, rate)
    );
    assert!(built(&project, Purpose::Export).settle() > 48000);
}

#[test]
fn listen_is_heard_in_a_preview_and_never_in_an_export() {
    let mut project = tracks(2);
    let compressor = &mut project.timelines[0].tracks[0].audio.strip.dynamics;
    compressor.on = true;
    compressor.compressor.on = true;
    compressor.compressor.sidechain.on = true;
    compressor.compressor.sidechain.listen = true;
    compressor.compressor.sidechain.source = Some(SourceRef::Track("A2".into()));
    // A2 goes nowhere: only A1 is heard, and A2 keys it.
    project.timelines[0].tracks[1].audio.strip.outputs.clear();
    let left = |purpose| {
        let mut graph = built(&project, purpose);
        let mut read = feed(&graph, |clip, _, _| if clip == "c1" { 0.1 } else { 0.05 });
        play(&mut graph, &mut read, 0, 2000).swap_remove(0)[1500]
    };
    close(
        left(Purpose::Preview),
        0.05,
        1e-6,
        "the sidechain, listened to",
    );
    close(left(Purpose::Export), 0.1, 1e-6, "A1 under its threshold");
    close(left(Purpose::Analysis), 0.1, 1e-6, "A1 under its threshold");
}

#[test]
fn a_sidechain_from_a_track_that_goes_nowhere_keys_the_compressor() {
    let mut project = tracks(2);
    let dynamics = &mut project.timelines[0].tracks[0].audio.strip.dynamics;
    dynamics.on = true;
    dynamics.compressor.on = true;
    dynamics.compressor.threshold_db = -30.0;
    dynamics.compressor.ratio = 4.0;
    dynamics.compressor.sidechain.on = true;
    dynamics.compressor.sidechain.source = Some(SourceRef::Track("A2".into()));
    project.timelines[0].tracks[1].audio.strip.outputs.clear();
    let mut graph = built(&project, Purpose::Export);
    // A1 at −40 dBFS, under the threshold; A2 at −6 dBFS, keying it down.
    let mut read = feed(&graph, |clip, _, _| if clip == "c1" { 0.01 } else { 0.5 });
    let got = play(&mut graph, &mut read, 0, 48000).swap_remove(0);
    assert!(db(got[40000] / 0.01) < -15.0, "{}", db(got[40000] / 0.01));
}

#[test]
fn the_levels_are_taken_before_and_after_the_fader() {
    let mut project = base();
    project.timelines[0].tracks[0].audio.strip.fader_db = -6.0;
    let mut graph = built(&project, Purpose::Preview);
    let meters = graph.meters();
    assert_eq!(meters.len(), 2);
    assert_eq!(meters[0].owner, Owner::Track("A1".into()));
    let mut read = feed(&graph, level(0.5));
    play(&mut graph, &mut read, 0, 1000);
    let (frames, pre, pre_squares) = meters[0].pre.read();
    let (_, post, _) = meters[0].post.read();
    assert_eq!(frames, 1000);
    close(pre[0], 0.5, 0.0, "the peak before the fader");
    close(
        pre_squares[1],
        0.25,
        1e-7,
        "the mean square before the fader",
    );
    close(db(post[0] / pre[0]), -6.0, 1e-4, "after the fader");
    let (_, bus, _) = meters[1].post.read();
    close(bus[0], f64::from(post[0]), 0.0, "the bus");
}

#[test]
fn a_run_started_in_the_middle_sounds_as_one_from_the_start_after_its_preroll() {
    let mut project = tracks(2);
    let timeline = &mut project.timelines[0];
    let strip = &mut timeline.tracks[0].audio.strip;
    strip.eq.on = true;
    strip.eq.bands[1].gain_db = 12.0;
    strip.eq.bands[1].q = 6.0;
    strip.eq.bands[1].freq = 60.0;
    strip.dynamics.on = true;
    strip.dynamics.compressor.on = true;
    strip.dynamics.compressor.threshold_db = -20.0;
    strip.dynamics.compressor.hold_ms = 150.0;
    strip.dynamics.expander.on = true;
    strip.dynamics.expander.mode = columbina_model::audio::fx::GateMode::Gate;
    strip.dynamics.expander.threshold_db = -12.0;
    strip.dynamics.expander.hold_ms = 200.0;
    let mut graph = built(&project, Purpose::Export);
    assert!(graph.preroll() > 0);
    let noise = |clip: &str, n: i64, channel: usize| {
        let mut rng = Rng::new(n.cast_unsigned() ^ ((channel as u64) << 40) ^ clip.len() as u64);
        rng.signed() * 0.3
    };
    let mut read = feed(&graph, noise);
    let from_start = play(&mut graph, &mut read, 0, 150_000);
    // Not on the grid of 32 samples of the holds of Dynamics: the graph lines them up.
    let late = 100_007;
    let from_middle = play(&mut graph, &mut read, late, 49_993);
    let mut largest = 0.0_f32;
    for (whole, part) in from_start.iter().zip(&from_middle) {
        for (a, b) in whole[100_007..].iter().zip(part) {
            largest = largest.max((a - b).abs());
        }
    }
    assert!(largest < 3.16e-5, "{largest}");
}

#[test]
fn the_output_of_the_whole_graph_does_not_depend_on_its_blocks() {
    // Three tracks, two buses, a send, a crossfade, a clip at 3/2, Dynamics, an equalizer and
    // lanes: the whole, and cut at random.
    let mut project = tracks(3);
    let timeline = &mut project.timelines[0];
    timeline.buses.push(bus("B2", &["B1"]));
    let at = |ticks| Time {
        ticks,
        scale: 15360,
    };
    let left = columbina_model::Clip::of_media("c1", "m1", at(0), 0, 50);
    let right = columbina_model::Clip::of_media("c4", "m1", at(30720), 50, 50);
    timeline.tracks[0].clips = vec![left, right];
    timeline.tracks[0].transitions.push(Transition {
        id: "x1".into(),
        kind: TransitionKind::Crossfade,
        left: "c1".into(),
        right: "c4".into(),
        length: 10,
        align: Alignment::Center,
        fade_in_level_db: CrossfadeLevel::Zero,
        fade_out_level_db: CrossfadeLevel::Minus3,
    });
    timeline.tracks[0].audio.strip.sends.push(Send {
        bus: "B2".into(),
        on: true,
        pre: true,
        level_db: -3.0,
        pan: columbina_model::Pan {
            x: 0.4,
            ..columbina_model::Pan::default()
        },
    });
    let a2 = &mut timeline.tracks[1];
    a2.audio.format = TrackFormat::Mono;
    a2.audio.channels = 1;
    a2.clips[0].speed = Speed { num: 3, den: 2 };
    a2.audio.strip.outputs[0].bus = "B2".into();
    a2.audio.lanes.insert(
        "fader_db".into(),
        lane(LaneKind::Continuous, &[(0.2, -20.0), (1.1, 3.0)]),
    );
    a2.audio.lanes.insert(
        "pan.x".into(),
        lane(LaneKind::Continuous, &[(0.0, -1.0), (2.0, 1.0)]),
    );
    let a3 = &mut timeline.tracks[2].audio;
    a3.strip.eq.on = true;
    a3.strip.eq.bands[2].gain_db = 9.0;
    a3.strip.dynamics.on = true;
    a3.strip.dynamics.compressor.on = true;
    a3.strip.dynamics.compressor.threshold_db = -24.0;
    a3.strip.dynamics.compressor.sidechain.on = true;
    a3.strip.dynamics.compressor.sidechain.source = Some(SourceRef::Track("A1".into()));
    a3.lanes.insert(
        "dynamics.compressor.ratio".into(),
        lane(LaneKind::Continuous, &[(0.0, 1.0), (1.5, 8.0)]),
    );
    let b2 = &mut timeline.buses[1];
    b2.strip.eq.on = true;
    b2.strip.eq.bands[0].on = true;
    let signal = |clip: &str, n: i64, channel: usize| {
        let mut rng = Rng::new(
            n.cast_unsigned().wrapping_mul(3) ^ (channel as u64) ^ ((clip.len() as u64) << 50),
        );
        rng.signed() * 0.4
    };
    let mut graph = built(&project, Purpose::Export);
    let mut read = feed(&graph, signal);
    let frames = 120_000;
    let whole = play_output(&mut graph, &mut read, None, (37_000, frames), &mut || 4096);
    assert!(whole[0].iter().any(|&s| s != 0.0));
    for seed in 0..3 {
        let mut rng = Rng::new(seed);
        let cut = play_output(&mut graph, &mut read, None, (37_000, frames), &mut || {
            if rng.range(0, 8) == 0 {
                rng.range(1, 4)
            } else {
                rng.log_range(4096)
            }
        });
        for (whole, cut) in whole.iter().zip(&cut) {
            let first = whole
                .iter()
                .zip(cut)
                .position(|(a, b)| a.to_bits() != b.to_bits());
            assert_eq!(first, None, "seed {seed}");
        }
    }
}

#[test]
fn lanes_reach_the_nodes_the_pan_and_the_sends() {
    // A threshold of 0 dB does nothing to a sound of −6 dBFS; its lane at −40 dB does.
    let mut project = base();
    let dynamics = &mut project.timelines[0].tracks[0].audio.strip.dynamics;
    dynamics.on = true;
    dynamics.compressor.on = true;
    dynamics.compressor.threshold_db = 0.0;
    dynamics.compressor.ratio = 10.0;
    let level_at = |project: &Project, at: usize| {
        let mut graph = built(project, Purpose::Export);
        let mut read = feed(&graph, level(0.5));
        db(play(&mut graph, &mut read, 0, at + 1)[0][at] / 0.5)
    };
    close(level_at(&project, 20_000), 0.0, 1e-4, "no compression");
    project.timelines[0].tracks[0].audio.lanes.insert(
        "dynamics.compressor.threshold_db".into(),
        lane(LaneKind::Continuous, &[(0.0, -40.0)]),
    );
    assert!(level_at(&project, 20_000) < -20.0);

    // A lane of the gain of a band of the equalizer: +12 dB at its bell, of a tone there.
    let mut project = base();
    let eq = &mut project.timelines[0].tracks[0].audio.strip.eq;
    eq.on = true;
    eq.bands[2].freq = 1000.0;
    project.timelines[0].tracks[0].audio.lanes.insert(
        "eq.2.gain_db".into(),
        lane(LaneKind::Continuous, &[(0.0, 12.0)]),
    );
    let mut graph = built(&project, Purpose::Export);
    let mut read = feed(&graph, |_, n, _| {
        #[allow(clippy::cast_precision_loss)]
        let phase = 2.0 * std::f32::consts::PI * 1000.0 * (n as f32) / 48000.0;
        0.1 * phase.sin()
    });
    let got = play(&mut graph, &mut read, 0, 48_000).swap_remove(0);
    let peak = got[24_000..].iter().fold(0.0_f32, |a, b| a.max(b.abs()));
    close(db(peak / 0.1), 12.0, 0.1, "the bell raised by its lane");

    // A lane of the pan of a mono track: all to the left, then all to the right.
    let mut project = base();
    let audio = &mut project.timelines[0].tracks[0].audio;
    audio.format = TrackFormat::Mono;
    audio.channels = 1;
    audio.lanes.insert(
        "pan.x".into(),
        lane(LaneKind::Continuous, &[(0.5, -1.0), (0.5, 1.0)]),
    );
    let mut graph = built(&project, Purpose::Export);
    let mut read = feed(&graph, level(0.5));
    let got = play(&mut graph, &mut read, 0, 48_000);
    close(got[0][1000], 0.5, 1e-6, "left before");
    close(got[1][1000], 0.0, 0.0, "right before");
    close(got[0][40_000], 0.0, 0.0, "left after");
    close(got[1][40_000], 0.5, 1e-6, "right after");

    // A lane of the level of a send.
    let mut project = base();
    let timeline = &mut project.timelines[0];
    timeline.buses.push(bus("B2", &[]));
    let audio = &mut timeline.tracks[0].audio;
    audio.strip.sends.push(Send {
        bus: "B2".into(),
        on: true,
        pre: false,
        level_db: 0.0,
        pan: columbina_model::Pan::default(),
    });
    audio.lanes.insert(
        "send.B2.level_db".into(),
        lane(LaneKind::Continuous, &[(0.0, -12.0)]),
    );
    let mut graph = built(&project, Purpose::Export);
    let b2 = graph.output_of("B2").unwrap();
    let mut read = feed(&graph, level(0.5));
    let got = play_output(&mut graph, &mut read, Some(b2), (0, 100), &mut || 100);
    close(db(got[0][50] / 0.5), -12.0, 1e-4, "the send at its lane");
}

#[test]
fn a_gate_shuts_on_the_same_sample_whether_the_run_starts_at_the_beginning_or_in_the_middle() {
    // T8: the holds of Dynamics are kept in bins of 32 samples, laid on the timeline by
    // `Node::align`; a run that starts off the grid lines them up all the same.
    let mut project = base();
    project.timelines[0].tracks[0].clips[0].length = 225;
    let dynamics = &mut project.timelines[0].tracks[0].audio.strip.dynamics;
    dynamics.on = true;
    let gate = &mut dynamics.expander;
    gate.on = true;
    gate.mode = columbina_model::audio::fx::GateMode::Gate;
    gate.threshold_db = -20.0;
    gate.range_db = 40.0;
    gate.hold_ms = 200.0;
    let mut graph = built(&project, Purpose::Export);
    // A tone of 1 kHz falling 4 dB a second from 0 dBFS: under the gate after 5 s.
    let falling = |_: &str, n: i64, _: usize| {
        #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
        let value = {
            let seconds = n as f64 / 48000.0;
            10f64.powf(-4.0 * seconds / 20.0)
                * (2.0 * std::f64::consts::PI * 1000.0 * seconds).sin()
        } as f32;
        value
    };
    let mut read = feed(&graph, falling);
    let frames = 48_000 * 8;
    let whole = play(&mut graph, &mut read, 0, frames).swap_remove(0);
    let preroll = i64::from(graph.preroll());
    // A start whose run begins 7 samples past the grid.
    let late = preroll + 32 * ((3 * 48_000 - preroll) / 32) + 7;
    let late_at = usize::try_from(late).unwrap();
    let part = play(&mut graph, &mut read, late, frames - late_at).swap_remove(0);
    let largest = whole[late_at..]
        .iter()
        .zip(&part)
        .fold(0.0_f32, |most, (a, b)| most.max((a - b).abs()));
    assert!(largest < 3.16e-5, "{largest}");
}
