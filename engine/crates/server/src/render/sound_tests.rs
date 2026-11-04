//! The sound of an export, written and read back: the graph's bus to the bit as floats, PCM of
//! 24 bits, the rate of the timeline and the channels of the bus, a range with its preroll,
//! Audio Normalization, and what is refused.

use std::path::Path;
use std::sync::Arc;

use columbina_audio::graph::{Graph, Options, Purpose};
use columbina_media::{probe, AudioDecoder, Samples};
use columbina_model::audio::BusFormat;
use columbina_model::{Project, Time, TrackKind};
use serde_json::json;

use super::super::{Plan, Request, Worker};
use crate::play::mix::Feed;
use crate::rpc;
use crate::test_media::{self, Sample};

/// The sample project with its track of sound only, its clip the whole of `sample` (2 s at 25
/// frames a second) from the start of the timeline.
fn project_of(sample: Sample) -> Option<Project> {
    let made = test_media::media(sample)?;
    let mut project = Project::sample();
    let facts = probe(&made.path).unwrap();
    project.media[0].path = made.path.to_string_lossy().into_owned();
    project.media[0].facts = facts.clone();
    let timeline = &mut project.timelines[0];
    timeline
        .tracks
        .retain(|track| track.kind == TrackKind::Audio);
    let clip = &mut timeline.tracks[0].clips[0];
    clip.source = columbina_model::Source::Media {
        media: "m1".into(),
        source_in: Time {
            ticks: 0,
            scale: facts.duration.scale,
        },
    };
    clip.start = 0;
    clip.length = 50;
    Some(project)
}

/// A request of `render.start` read from `request`, of timeline t1, written to `name` in `dir`.
fn request(dir: &Path, name: &str, mut request: serde_json::Value) -> Request {
    request["timeline"] = json!("t1");
    request["path"] = json!(dir.join(name));
    serde_json::from_value(request).unwrap()
}

/// The worker that exported `project` as `asked` to `name` in `dir`.
fn export(dir: &Path, project: &Project, name: &str, asked: serde_json::Value) -> Worker {
    let mut worker = super::super::tests::worker(dir);
    worker.plan = Plan::new(project.clone(), request(dir, name, asked), dir).unwrap();
    worker.run().unwrap();
    worker
}

/// Every sample of `path`, in its own channels and order.
fn read_back(path: &Path) -> Vec<f32> {
    let facts = probe(path).unwrap();
    let stream = facts.audio.unwrap().streams.unwrap()[0].index;
    let mut decoder = AudioDecoder::open_unmixed(path, stream as usize, None).unwrap();
    let mut data = vec![];
    while let Some(Samples { data: run, .. }) = decoder.next().unwrap() {
        data.extend(run);
    }
    data
}

/// `frames` samples of output `bus` of the graph of export of `project` from `at`, interleaved.
fn graph_of(project: &Project, bus: &str, at: i64, frames: usize) -> Vec<f32> {
    let mut graph = Graph::build(project, 0, Options::new(4096, Purpose::Export)).unwrap();
    let mut feed = Feed::new(Arc::new(project.clone()), 0, graph.clips(), false);
    let output = graph.output_of(bus).unwrap();
    graph.start(at, &mut feed).unwrap();
    let channels = graph.output(output).channels();
    let mut out = vec![];
    let mut done = 0;
    while done < frames {
        let count = (frames - done).min(4096);
        graph.process(&mut feed, count).unwrap();
        for n in 0..count {
            for channel in 0..channels {
                out.push(graph.output(output).channel(channel)[n]);
            }
        }
        done += count;
    }
    out
}

/// One step of PCM of 24 bits.
const STEP: f32 = 1.0 / 8_388_608.0;

#[test]
fn the_sound_alone_is_exported_as_floats_to_the_bit_of_the_graph_hot_and_all() {
    let Some(mut project) = project_of(Sample::LeftRight) else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    // +16 dB: over 1, which floats keep.
    project.timelines[0].tracks[0].clips[0].gain_db = 16.0.into();
    let worker = export(
        dir.path(),
        &project,
        "mix.wav",
        json!({"video": null, "audio": {"codec": "pcm_f32le"}}),
    );
    assert_eq!(worker.encoder.as_deref(), Some("pcm_f32le"));
    let path = dir.path().join("mix.wav");
    let audio = probe(&path).unwrap().audio.unwrap();
    assert_eq!((audio.rate, audio.channels), (48_000, 2));
    let got = read_back(&path);
    let want = graph_of(&project, "B1", 0, 96_000);
    assert_eq!(got.len(), want.len());
    assert!(got
        .iter()
        .zip(&want)
        .all(|(a, b)| a.to_bits() == b.to_bits()));
    assert!(got.iter().any(|sample| sample.abs() > 1.0), "not clipped");
    // PCM of 24 bits: the same, clipped.
    export(dir.path(), &project, "mix24.wav", json!({"video": null}));
    let path = dir.path().join("mix24.wav");
    assert_eq!(probe(&path).unwrap().audio.unwrap().codec, "pcm_s24le");
    let fixed = read_back(&path);
    assert_eq!(fixed.len(), want.len());
    for (got, want) in fixed.iter().zip(&want) {
        assert!((got - want.clamp(-1.0, 1.0)).abs() <= STEP, "{got} {want}");
    }
}

#[test]
fn the_rate_follows_the_timeline_and_the_channels_the_bus() {
    let Some(mut project) = project_of(Sample::SixSines) else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let timeline = &mut project.timelines[0];
    timeline.sample_rate = 96_000;
    timeline.buses[0].format = BusFormat::FiveOne;
    timeline.tracks[0].audio.format = columbina_model::audio::TrackFormat::FiveOne;
    timeline.tracks[0].audio.channels = 6;
    export(dir.path(), &project, "six.wav", json!({"video": null}));
    let path = dir.path().join("six.wav");
    let audio = probe(&path).unwrap().audio.unwrap();
    assert_eq!((audio.rate, audio.channels), (96_000, 6));
    assert_eq!(audio.layout.as_deref(), Some("5.1"));
    // Each channel of the bus at its speaker.
    let got = read_back(&path);
    let want = graph_of(&project, "B1", 0, 192_000);
    assert_eq!(got.len(), want.len());
    assert!(want.iter().step_by(6).any(|sample| sample.abs() > 0.05));
    for (got, want) in got.iter().zip(&want) {
        assert!((got - want).abs() <= STEP);
    }
    // AAC at 320 kb/s in MPEG-4 audio, six channels, at another rate than the timeline's.
    export(
        dir.path(),
        &project,
        "six.m4a",
        json!({"video": null, "audio": {"codec": "aac", "bitrate": 320_000, "rate": 48_000}}),
    );
    let audio = probe(&dir.path().join("six.m4a")).unwrap().audio.unwrap();
    assert_eq!(
        (audio.codec.as_str(), audio.rate, audio.channels),
        ("aac", 48_000, 6)
    );
}

#[test]
fn a_range_is_exported_with_its_preroll_as_the_whole_has_it() {
    let Some(mut project) = project_of(Sample::LeftRight) else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let strip = &mut project.timelines[0].tracks[0].audio.strip;
    strip.eq.on = true;
    strip.dynamics.on = true;
    strip.dynamics.compressor.on = true;
    let floats = json!({"video": null, "audio": {"codec": "pcm_f32le"}});
    export(dir.path(), &project, "whole.wav", floats.clone());
    let mut range = floats;
    range["range"] = json!({"from": 30, "to": 50});
    export(dir.path(), &project, "range.wav", range);
    let whole = read_back(&dir.path().join("whole.wav"));
    let range = read_back(&dir.path().join("range.wav"));
    assert_eq!(range.len(), 2 * 38_400);
    assert_eq!(&whole[2 * 57_600..], &range[..]);
}

#[test]
fn audio_normalization_brings_the_export_to_its_standard_and_tells_what_it_did() {
    let Some(project) = project_of(Sample::LeftRight) else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let worker = export(
        dir.path(),
        &project,
        "loud.wav",
        json!({"video": null, "audio": {"codec": "pcm_f32le",
            "normalize": {"standard": "spotify", "mode": "optimize"}}}),
    );
    let told = worker.normalization.clone().unwrap();
    assert_eq!(told["standard"], "spotify");
    assert_eq!(told["met"], true, "{told}");
    let got = read_back(&dir.path().join("loud.wav"));
    let left: Vec<f32> = got.iter().step_by(2).copied().collect();
    let right: Vec<f32> = got.iter().skip(1).step_by(2).copied().collect();
    let reading = columbina_audio::analysis::analyze(
        48_000,
        columbina_audio::Layout::Stereo,
        true,
        &[&left, &right],
    );
    let integrated = reading.integrated.unwrap();
    assert!((integrated + 14.0).abs() <= 0.1, "{integrated}");
    assert!(reading.true_peak <= -1.0, "{}", reading.true_peak);
    assert!((told["after"]["integrated"].as_f64().unwrap() - integrated).abs() < 0.05);
    assert!(worker.message.is_none(), "{:?}", worker.message);
}

#[test]
fn what_the_sound_of_an_export_cannot_be_is_refused_with_its_reason() {
    let project = Project::sample();
    let dir = tempfile::tempdir().unwrap();
    let refused = |name: &str, asked: serde_json::Value| {
        let Err(fault) = Plan::new(
            project.clone(),
            request(dir.path(), name, asked),
            dir.path(),
        ) else {
            panic!("{name} was not refused");
        };
        assert_eq!(fault.code, rpc::INVALID_PARAMS, "{name}");
        fault
            .data
            .map_or(json!(null), |data| data["reason"].clone())
    };
    let film =
        |audio: serde_json::Value| json!({"codec": "h264", "quality": "high", "audio": audio});
    // The sound alone with a codec of picture, into a film's container; a picture into WAV.
    assert_eq!(
        refused("a.wav", json!({"video": null, "codec": "h264"})),
        json!(null)
    );
    assert_eq!(refused("a.mp4", json!({"video": null})), json!(null));
    assert_eq!(
        refused("a.wav", json!({"codec": "h264", "quality": "high"})),
        json!(null)
    );
    assert_eq!(refused("a.mov", json!({"codec": "h264"})), json!(null));
    assert_eq!(
        refused("a.wav", json!({"video": null, "audio": {"codec": "aac"}})),
        "audio_codec"
    );
    assert_eq!(
        refused("a.mp4", film(json!({"codec": "pcm_s24le"}))),
        "audio_codec"
    );
    assert_eq!(refused("a.mov", film(json!({"bus": "B9"}))), "bus");
    assert_eq!(
        refused("a.mov", film(json!({"bitrate": 100_000}))),
        "bitrate"
    );
    assert_eq!(
        refused(
            "a.mov",
            film(json!({"codec": "pcm_f32le", "bitrate": 192_000}))
        ),
        "bitrate"
    );
    assert_eq!(refused("a.mov", film(json!({"rate": 7}))), "rate");
    assert_eq!(
        refused(
            "a.mov",
            film(json!({"normalize": {"standard": "loud", "mode": "optimize"}}))
        ),
        "standard"
    );
    assert_eq!(
        refused(
            "a.mov",
            film(json!({"normalize": {"standard": "spotify", "mode": "optimize", "target": 5.0}}))
        ),
        "target"
    );
    // A video of anything but null is not read.
    assert!(serde_json::from_value::<Request>(
        json!({"timeline": "t1", "path": "C:/a.wav", "video": {"codec": "h264"}})
    )
    .is_err());
}

#[test]
fn the_speakers_of_each_format_of_a_bus() {
    use columbina_audio::Layout;
    assert_eq!(super::speakers(Layout::Stereo), (0x3, vec![0, 1]));
    assert_eq!(super::speakers(Layout::Mono), (0x4, vec![0]));
    assert_eq!(
        super::speakers(Layout::Surround51),
        (0x3F, vec![0, 1, 2, 3, 4, 5])
    );
    // 7.1 of SMPTE (L R C LFE Lss Rss Lrs Rrs) in libav's order (FL FR FC LFE BL BR SL SR).
    assert_eq!(
        super::speakers(Layout::Surround71),
        (0x63F, vec![0, 1, 2, 3, 6, 7, 4, 5])
    );
    assert_eq!(super::speakers(Layout::Lcrs), (0x107, vec![0, 1, 2, 3]));
}
