//! The colour management of the timeline monitor: a change of its settings while the timeline
//! plays is made ready on a thread of its own, the shaders of the new transforms and grades
//! compiled there, and the monitor goes on drawing by the old ones until then, never waiting on
//! a compiler.

use std::path::Path;

use columbina_model::color_management::{Preset, Science};
use columbina_model::grade::{Kind, Link, Node, NodeKind, Param};
use columbina_model::{Clip, ColorSettings, Rgba, Source as ClipSource, Track};

use super::*;

/// A monitor of no view, whose colour management is made ready on a thread of its own.
fn monitor(dir: &Path) -> Worker {
    worker(Grading::monitor(
        LutPlaces::with_library(dir, vec![]),
        Arc::new(|| {}),
    ))
}

/// A monitor of no view that makes its colour management in the frame, as an export does.
fn waiting_monitor(dir: &Path) -> Worker {
    worker(Grading::new(LutPlaces::with_library(dir, vec![])))
}

fn worker(grading: Grading) -> Worker {
    let (_, inbox) = mpsc::channel();
    Worker {
        inbox,
        shared: Arc::default(),
        notify: Arc::new(|_| {}),
        view: Arc::default(),
        graphics: Graphics::new(),
        sources: Sources::new(Hardware::Off, Arc::new(|_, _| {})),
        grading,
        output: None,
        scopes: Scopes::new(true, Graphics::new()),
        compare: Comparison::new(),
        status: Status {
            timeline: Some("t1".into()),
            ..Status::default()
        },
        project: None,
        timeline: None,
        generation: 0,
        readers: BTreeMap::new(),
        reader_budget: 0,
        previous: BTreeMap::new(),
        compositor: None,
        targets: Vec::new(),
        current: None,
        clock: None,
        sound: SoundSlot::new(crate::audio::Kind::Null),
        mixed_from: None,
        heard: Cell::new(0),
        fine: None,
        epoch: Instant::now(),
        settle: None,
        told: None,
    }
}

/// A timeline of 1920 × 1080 of two colours, each graded, managed as `color` says.
fn graded_timeline(color: ColorSettings) -> Project {
    let mut project = Project::sample();
    let timeline = &mut project.timelines[0];
    (timeline.width, timeline.height) = (1920, 1080);
    let mut node = Node::new("n1", Kind::Corrector);
    let NodeKind::Corrector(settings) = &mut node.kind else {
        unreachable!("a corrector")
    };
    settings.primaries.saturation = Param::Fixed(70.0);
    let grade = Grade {
        nodes: vec![node],
        links: vec![Link::rgb("source", "n1"), Link::rgb("n1", "output")],
    };
    let clips = [("c1", [200, 110, 40, 255]), ("c2", [40, 150, 170, 255])]
        .into_iter()
        .enumerate()
        .map(|(at, (id, rgba))| {
            let mut clip = Clip::new(id, ClipSource::Color { color: Rgba(rgba) }, 0, 400);
            clip.grade = grade.clone();
            clip.transform.scale = if at == 0 { 1.0 } else { 0.5 };
            clip
        })
        .collect();
    timeline.tracks = vec![
        Track::new("V1", TrackKind::Video).with_clips(vec![Clip::new(
            "c0",
            ClipSource::Color {
                color: Rgba([90, 90, 90, 255]),
            },
            0,
            400,
        )]),
        Track::new("V2", TrackKind::Video).with_clips(clips),
    ];
    project.color = color;
    project
}

#[test]
#[allow(clippy::cast_precision_loss, reason = "milliseconds")]
fn settings_changed_while_playing_are_made_ready_without_a_frame_waiting_for_them() {
    let Some(gpu) = columbina_compose::gpu_for_tests() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let sdr = ColorSettings {
        science: Science::YrgbManaged,
        preset: Preset::SdrRec709,
        ..ColorSettings::default()
    };
    // ACES 2.0, its ODT of Rec.709: the output transform that FXC takes longest over.
    let aces = ColorSettings {
        science: Science::Acescct,
        ..ColorSettings::default()
    };
    let mut monitor = monitor(dir.path());
    monitor.replace(Arc::new(graded_timeline(sdr)));
    // Loaded: its colour made before its first frame, as a load waits for it.
    monitor.status.frame = Some(0);
    monitor.draw(true, false).unwrap();
    let before = monitor.current.as_ref().unwrap().read_rgba().unwrap();
    let mut times = Vec::new();
    let mut frame = 1;
    let mut draw = |monitor: &mut Worker, times: &mut Vec<f64>| {
        monitor.status.frame = Some(frame);
        frame += 1;
        let since = Instant::now();
        monitor.draw(true, false).unwrap();
        times.push(since.elapsed().as_secs_f64() * 1e3);
    };
    for _ in 0..10 {
        draw(&mut monitor, &mut times);
    }
    let steady = times.iter().copied().fold(0.0, f64::max);
    // The settings change while it plays: it goes on drawing until the new colour is ready.
    let changed = Instant::now();
    monitor.replace(Arc::new(graded_timeline(aces.clone())));
    let mut during = Vec::new();
    loop {
        assert!(changed.elapsed() < Duration::from_secs(60), "never made");
        draw(&mut monitor, &mut during);
        if !monitor.grading.making() {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let ready = changed.elapsed().as_secs_f64() * 1e3;
    // The frame that took the new colour, and the next.
    let took = *during.last().unwrap();
    let mut after = Vec::new();
    for _ in 0..5 {
        draw(&mut monitor, &mut after);
    }
    let shown = monitor.current.as_ref().unwrap().read_rgba().unwrap();
    assert_ne!(before, shown, "the new colour is shown");
    let worst = during.iter().copied().fold(0.0, f64::max);
    // What a monitor that waited for the compiler would have shown: the first frame of the new
    // colour on a compositor and grading of its own, made and compiled in the frame.
    let mut waiting = waiting_monitor(dir.path());
    waiting.replace(Arc::new(graded_timeline(aces)));
    waiting.status.frame = Some(0);
    let since = Instant::now();
    waiting.draw(true, false).unwrap();
    let stalled = since.elapsed().as_secs_f64() * 1e3;
    eprintln!(
        "steady {steady:.1} ms; ready after {ready:.0} ms, {} frames drawn meanwhile, the worst \
         {worst:.1} ms, the frame that took it {took:.1} ms, then {after:.1?} ms; a monitor that \
         waited would have stalled {stalled:.0} ms",
        during.len()
    );
    // No frame waited for the compiler: those drawn meanwhile are as the steady ones; the frame
    // that takes the new colour makes its pipelines of shaders compiled already, which the
    // driver still makes ready (some 50 to 200 ms on this card), against the second and more of
    // a frame that compiles them.
    assert!(during.len() >= 2, "{during:?}");
    let meanwhile = during[..during.len() - 1]
        .iter()
        .copied()
        .fold(0.0, f64::max);
    assert!(meanwhile < steady.max(10.0) * 4.0 + 40.0, "{during:?}");
    assert!(worst < 300.0, "{during:?}");
    assert!(stalled > worst * 3.0, "{stalled} against {worst}");
    assert_eq!(gpu.uncaught_errors().0, 0);
}

#[test]
fn a_frame_drawn_off_screen_is_measured_as_the_monitor_shows_it() {
    let Some(gpu) = columbina_compose::gpu_for_tests() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    // A timeline of DaVinci Wide Gamut / Intermediate put out as Rec.709 through the DaVinci
    // DRT: its colours go through input transforms (as graphics) and an output transform.
    let managed = ColorSettings {
        science: Science::YrgbManaged,
        preset: Preset::Custom,
        timeline: "DaVinci Wide Gamut/DaVinci Intermediate".into(),
        working_luminance: columbina_model::color_management::WorkingLuminance::Hdr1000,
        output_drt: columbina_model::color_management::ToneMapping::Davinci,
        ..ColorSettings::default()
    };
    let project = Arc::new(graded_timeline(managed));
    let mut monitor = monitor(dir.path());
    monitor.replace(Arc::clone(&project));
    monitor.status.frame = Some(12);
    monitor.draw(true, false).unwrap();
    let shown = Shown::Target {
        gpu: Arc::clone(monitor.compositor.as_ref().unwrap().gpu()),
        target: Arc::clone(monitor.current.as_ref().unwrap()),
        frame: 12,
    };
    let on_monitor = monitor
        .scopes
        .statistics(Monitor::Timeline, &shown, 256)
        .unwrap();
    let probe = crate::probe::Probe::new(
        Arc::clone(&monitor.graphics),
        LutPlaces::with_library(dir.path(), vec![]),
    );
    let measure = |project: &Project| {
        let drawn = probe.draw(project, "t1", 12, None).unwrap();
        let signal = drawn.signal;
        let shown = Shown::Target {
            gpu: drawn.gpu,
            target: drawn.target,
            frame: 12,
        };
        monitor
            .scopes
            .statistics_by(Monitor::Timeline, &shown, 256, Some(signal))
            .unwrap()
    };
    let off_screen = measure(&project);
    for channel in ["red", "green", "blue", "luma"] {
        for what in ["min", "max", "mean"] {
            let (a, b) = (
                on_monitor[channel][what].as_f64().unwrap(),
                off_screen[channel][what].as_f64().unwrap(),
            );
            assert!(
                (a - b).abs() < 1e-4,
                "{channel} {what}: {a} on the monitor, {b} off it"
            );
        }
    }
    // And not as it would be without colour management: the transforms are there.
    let mut plain = (*project).clone();
    plain.color = ColorSettings::default();
    let unmanaged = measure(&plain);
    let mean = |figures: &Value| figures["luma"]["mean"].as_f64().unwrap();
    assert!(
        (mean(&unmanaged) - mean(&off_screen)).abs() > 0.01,
        "{} and {}",
        mean(&unmanaged),
        mean(&off_screen)
    );
    assert_eq!(gpu.uncaught_errors().0, 0);
}
