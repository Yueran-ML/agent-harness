//! The sound of an export (plan T6): the graph of playback built for export from the snapshot of
//! the project, its clips read by readers that wait for them, the bus asked for taken as it is in
//! its own format (`columbina_audio::deliver`), Audio Normalization after it where it is asked
//! for, at the rate asked for, laid on the speakers of the file and written as AAC or PCM.

use std::sync::Arc;

use columbina_audio::analysis::{Analysis, Analyzer, Level};
use columbina_audio::deliver::{normalization, Aim, Finish, Mode, Normalization, Render};
use columbina_audio::graph::{Graph, Options, Purpose};
use columbina_audio::node::{Layout, Role};
use columbina_audio::standards;
use columbina_media::{AudioCodec, AudioSettings, AUDIO_BIT_RATE};
use columbina_model::{Project, Timeline};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::play::mix::Feed;
use crate::rpc::Fault;

/// The most samples that the graph of an export processes at once.
const MAX_BLOCK: usize = 4096;

/// The rates of AAC that an export takes, bits a second (the Deliver page's menu).
const BIT_RATES: [u32; 4] = [128_000, 192_000, 256_000, 320_000];

/// The lowest and the highest rate of the sound of a file, samples a second.
const RATES: (u32, u32) = (8_000, 192_000);

/// The `audio` of `render.start`.
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AudioRequest {
    /// The bus rendered: a main output, or the bus that is monitored; that one where it is left
    /// out.
    #[serde(default)]
    bus: Option<String>,
    /// Samples a second: the timeline's where it is left out.
    #[serde(default)]
    rate: Option<u32>,
    #[serde(default)]
    codec: Option<SoundCodec>,
    /// Bits a second of AAC.
    #[serde(default)]
    bitrate: Option<u32>,
    #[serde(default)]
    normalize: Option<NormalizeRequest>,
}

#[derive(Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum SoundCodec {
    Aac,
    PcmS24le,
    PcmF32le,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NormalizeRequest {
    /// The id of a standard of `columbina_audio::standards`.
    standard: String,
    /// LUFS: the standard's target (or the timeline's Target Loudness) where it is left out.
    #[serde(default)]
    target: Option<f64>,
    mode: NormalizeMode,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum NormalizeMode {
    Normalize,
    Optimize,
}

/// What a file of an export is written as, by the extension of its path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Container {
    Mp4,
    Mov,
    Wav,
    M4a,
}

impl Container {
    /// The container that `extension` names, if an export writes it.
    pub(super) fn of(extension: &str) -> Option<Self> {
        [
            ("mp4", Self::Mp4),
            ("mov", Self::Mov),
            ("wav", Self::Wav),
            ("m4a", Self::M4a),
        ]
        .into_iter()
        .find(|(name, _)| extension.eq_ignore_ascii_case(name))
        .map(|(_, container)| container)
    }

    /// Whether it holds a picture.
    pub(super) fn pictured(self) -> bool {
        matches!(self, Self::Mp4 | Self::Mov)
    }
}

/// The sound of an export, checked.
#[derive(Clone, Debug)]
pub(super) struct SoundPlan {
    /// The bus rendered.
    pub(super) bus: String,
    /// Samples a second.
    pub(super) rate: u32,
    pub(super) codec: AudioCodec,
    pub(super) normalize: Option<Normalizing>,
}

/// Audio Normalization asked for (p. 4055).
#[derive(Clone, Copy, Debug)]
pub(super) struct Normalizing {
    standard: &'static str,
    mode: Mode,
    aim: Aim,
}

/// A refusal of the `audio` of `render.start`, with its reason.
fn refused(message: impl Into<String>, data: Value) -> Fault {
    Fault::params(message).with_data(data)
}

/// The sound of an export of `timeline` into `container`, as `request` asks: its bus (a main
/// output or the bus that is monitored), its rate, its codec (AAC for MP4, `QuickTime` and
/// MPEG-4 audio, PCM of 24 bits for WAV, where none is asked for) and its Audio Normalization.
pub(super) fn plan(
    request: Option<AudioRequest>,
    timeline: &Timeline,
    container: Container,
) -> Result<SoundPlan, Fault> {
    let request = request.unwrap_or_default();
    let mains: Vec<&str> = timeline
        .buses
        .iter()
        .filter(|bus| bus.strip.outputs.is_empty())
        .map(|bus| bus.id.as_str())
        .collect();
    let monitor = timeline
        .audio
        .monitor
        .as_deref()
        .filter(|monitor| timeline.buses.iter().any(|bus| bus.id == *monitor));
    let bus = match (&request.bus, monitor) {
        (Some(bus), _) => {
            if !mains.contains(&bus.as_str()) && monitor != Some(bus.as_str()) {
                return Err(refused(
                    format!("bus {bus} is neither a main output nor the bus monitored"),
                    json!({"reason": "bus", "bus": bus}),
                ));
            }
            bus.clone()
        }
        (None, Some(monitor)) => monitor.to_owned(),
        (None, None) => mains.first().map(|bus| (*bus).to_owned()).ok_or_else(|| {
            refused(
                "the timeline has no main output to export",
                json!({"reason": "bus"}),
            )
        })?,
    };
    let rate = request.rate.unwrap_or(timeline.sample_rate);
    if !(RATES.0..=RATES.1).contains(&rate) {
        return Err(refused(
            format!(
                "a rate of sound from {} to {} samples a second",
                RATES.0, RATES.1
            ),
            json!({"reason": "rate", "rate": rate}),
        ));
    }
    let wanted = request.codec.unwrap_or(if container == Container::Wav {
        SoundCodec::PcmS24le
    } else {
        SoundCodec::Aac
    });
    let fits = match wanted {
        SoundCodec::Aac => container != Container::Wav,
        SoundCodec::PcmS24le | SoundCodec::PcmF32le => {
            matches!(container, Container::Mov | Container::Wav)
        }
    };
    if !fits {
        return Err(refused(
            "AAC is written in .mp4, .mov or .m4a, PCM in .mov or .wav",
            json!({"reason": "audio_codec"}),
        ));
    }
    let codec = match wanted {
        SoundCodec::Aac => {
            let bitrate = request.bitrate.unwrap_or(AUDIO_BIT_RATE);
            if !BIT_RATES.contains(&bitrate) {
                return Err(refused(
                    "AAC is written at 128000, 192000, 256000 or 320000 bits a second",
                    json!({"reason": "bitrate", "bitrate": bitrate}),
                ));
            }
            AudioCodec::Aac { bitrate }
        }
        _ if request.bitrate.is_some() => {
            return Err(refused(
                "PCM takes no rate of bits",
                json!({"reason": "bitrate"}),
            ))
        }
        SoundCodec::PcmS24le => AudioCodec::PcmS24,
        SoundCodec::PcmF32le => AudioCodec::PcmF32,
    };
    let normalize = request
        .normalize
        .map(|asked| normalizing(&asked, timeline))
        .transpose()?;
    Ok(SoundPlan {
        bus,
        rate,
        codec,
        normalize,
    })
}

/// Audio Normalization as `asked`, to a standard of `columbina_audio::standards`.
fn normalizing(asked: &NormalizeRequest, timeline: &Timeline) -> Result<Normalizing, Fault> {
    let standard = standards::by_id(&asked.standard).ok_or_else(|| {
        refused(
            format!("there is no standard of loudness {}", asked.standard),
            json!({"reason": "standard", "standard": asked.standard}),
        )
    })?;
    #[allow(
        clippy::cast_possible_truncation,
        reason = "a target of −36 to −10 LUFS"
    )]
    let target = asked
        .target
        .unwrap_or_else(|| f64::from(standard.target_or(timeline.audio.loudness.target as f32)));
    let (low, high) = columbina_audio::constants::NORMALIZE_RANGE;
    if !target.is_finite() || target < f64::from(low) || target > f64::from(high) {
        return Err(refused(
            format!("a target of {low} to {high} LUFS"),
            json!({"reason": "target"}),
        ));
    }
    Ok(Normalizing {
        standard: standard.id,
        mode: match asked.mode {
            NormalizeMode::Normalize => Mode::Normalize,
            NormalizeMode::Optimize => Mode::Optimize,
        },
        aim: Aim {
            level: Level::of(standard),
            target,
            true_peak: standard.true_peak.map(f64::from),
        },
    })
}

/// The speakers of the channels of `layout` in a file: the mask of libav's (`AV_CH_*`), and for
/// each channel of the file, in the order of the bits of the mask, the channel of the bus.
///
/// The surrounds of 5.0 and 5.1 are written as the back pair (5.1 is `0x3F`, as WAV and AAC have
/// it), those of 7.0 and 7.1 at the sides with the rears at the back (7.1 is `0x63F`); the one
/// surround of LCRS at the back centre.
pub(super) fn speakers(layout: Layout) -> (u64, Vec<usize>) {
    const FL: u64 = 0x1;
    const FR: u64 = 0x2;
    const FC: u64 = 0x4;
    const LFE: u64 = 0x8;
    const BL: u64 = 0x10;
    const BR: u64 = 0x20;
    const BC: u64 = 0x100;
    const SL: u64 = 0x200;
    const SR: u64 = 0x400;
    let roles = layout.roles();
    let rears = roles.contains(&Role::LeftRear);
    let bits: Option<Vec<u64>> = roles
        .iter()
        .map(|role| match role {
            Role::Mono | Role::Centre => Some(FC),
            Role::Left => Some(FL),
            Role::Right => Some(FR),
            Role::Lfe => Some(LFE),
            Role::Surround => Some(BC),
            Role::LeftSurround => Some(if rears { SL } else { BL }),
            Role::RightSurround => Some(if rears { SR } else { BR }),
            Role::LeftRear => Some(BL),
            Role::RightRear => Some(BR),
            Role::Discrete(_) => None,
        })
        .collect();
    match bits {
        Some(bits) => {
            let mut order: Vec<usize> = (0..bits.len()).collect();
            order.sort_by_key(|&channel| bits[channel]);
            (bits.iter().fold(0, |mask, bit| mask | bit), order)
        }
        None => (0, (0..roles.len()).collect()),
    }
}

/// The settings of the sound of a file of `plan` from a bus of `layout`.
pub(super) fn settings(plan: &SoundPlan, layout: Layout) -> AudioSettings {
    AudioSettings {
        rate: plan.rate,
        channels: u16::try_from(layout.channels()).unwrap_or(u16::MAX),
        mask: speakers(layout).0,
        codec: plan.codec,
    }
}

/// The sound of an export as it is written: the bus with its [`Finish`], at the rate of the
/// file, a run at a time.
pub(super) struct Mix {
    graph: Graph,
    feed: Feed,
    render: Render,
    /// The channel of the bus of each channel of the file.
    order: Vec<usize>,
    /// Whether what is written is clipped to −1…1: all but PCM of floats.
    clipped: bool,
}

impl Mix {
    /// The bus of `plan` of timeline `timeline` with `finish` after it, from sample `at` of the
    /// timeline.
    pub(super) fn start(
        project: &Arc<Project>,
        timeline: usize,
        plan: &SoundPlan,
        finish: Finish,
        at: i64,
    ) -> Result<Self, String> {
        let mut graph = Graph::build(project, timeline, Options::new(MAX_BLOCK, Purpose::Export))
            .map_err(|error| error.to_string())?;
        for clip in graph.unplayable() {
            crate::log::log("sound_of_a_clip_unplayable", &json!({ "clip": clip }));
        }
        let output = graph
            .output_of(&plan.bus)
            .ok_or_else(|| format!("bus {} is not an output of the timeline", plan.bus))?;
        // An export fails where a clip that is heard cannot be read.
        let mut feed = Feed::new(Arc::clone(project), timeline, graph.clips(), false);
        let render = Render::start(&mut graph, &mut feed, (output, finish), at, plan.rate)
            .map_err(|_| feed.take_error())?;
        let order = speakers(render.layout()).1;
        Ok(Mix {
            graph,
            feed,
            render,
            order,
            clipped: plan.codec != AudioCodec::PcmF32,
        })
    }

    /// The channels of the bus.
    pub(super) fn layout(&self) -> Layout {
        self.render.layout()
    }

    /// The next `frames` samples of the bus, a vector for each of its channels.
    fn planar(&mut self, frames: usize) -> Result<Vec<Vec<f32>>, String> {
        self.render
            .take(&mut self.graph, &mut self.feed, frames)
            .map_err(|_| self.feed.take_error())
    }

    /// The next `frames` samples as the file has them: interleaved, on its speakers, clipped
    /// where the file is not of floats (the one place an export clips, T4b).
    pub(super) fn take(&mut self, frames: usize) -> Result<Vec<f32>, String> {
        let planar = self.planar(frames)?;
        let mut out = vec![0.0; frames * self.order.len()];
        for (at, &channel) in self.order.iter().enumerate() {
            let into = out.iter_mut().skip(at).step_by(self.order.len());
            for (into, &sample) in into.zip(&planar[channel]) {
                *into = if self.clipped {
                    sample.clamp(-1.0, 1.0)
                } else {
                    sample
                };
            }
        }
        Ok(out)
    }
}

/// What Audio Normalization of `plan` finds for `frames` samples of the file of the bus from
/// sample `at` of the timeline, measuring it with a graph of its own each time (a graph is not
/// run twice: `RNNoise` has no reset, T12); `go_on` is asked between blocks, and stops it where it
/// fails (the export is cancelled).
pub(super) fn normalize(
    project: &Arc<Project>,
    timeline: usize,
    plan: &SoundPlan,
    (at, frames): (i64, usize),
    go_on: &dyn Fn() -> Result<(), String>,
) -> Result<Option<(Normalizing, Normalization)>, String> {
    let Some(normalizing) = plan.normalize else {
        return Ok(None);
    };
    let found = normalization(normalizing.mode, &normalizing.aim, |finish| {
        measure(project, timeline, plan, finish, (at, frames), go_on)
    })?;
    Ok(Some((normalizing, found)))
}

/// The analysis of `frames` samples of the file of the bus of `plan` with `finish` after it,
/// from sample `at` of the timeline.
fn measure(
    project: &Arc<Project>,
    timeline: usize,
    plan: &SoundPlan,
    finish: Finish,
    (at, frames): (i64, usize),
    go_on: &dyn Fn() -> Result<(), String>,
) -> Result<Analysis, String> {
    let mut mix = Mix::start(project, timeline, plan, finish, at)?;
    let mut analyzer = Analyzer::new(plan.rate, mix.layout(), true);
    let mut left = frames;
    while left > 0 {
        go_on()?;
        let count = left.min(MAX_BLOCK);
        let run = mix.planar(count)?;
        let channels: Vec<&[f32]> = run.iter().map(Vec::as_slice).collect();
        analyzer.take(&channels);
        left -= count;
    }
    Ok(analyzer.finish())
}

/// What `render.progress` tells of Audio Normalization: the standard and the mode, what was put
/// after the bus, the loudness and the true peak before and after, and whether it met the
/// standard.
pub(super) fn told(normalizing: &Normalizing, found: &Normalization) -> Value {
    let reading = |analysis: &Analysis| {
        json!({
            "integrated": analysis.integrated,
            "true_peak": finite(analysis.true_peak),
        })
    };
    let after = found.after.map_or_else(
        || {
            json!({
                "integrated": found
                    .before
                    .integrated
                    .map(|integrated| integrated + found.finish.gain_db),
                "true_peak": finite(found.before.true_peak + found.finish.gain_db),
            })
        },
        |after| reading(&after),
    );
    json!({
        "standard": normalizing.standard,
        "mode": match normalizing.mode { Mode::Normalize => "normalize", Mode::Optimize => "optimize" },
        "target": normalizing.aim.target,
        "true_peak_max": normalizing.aim.true_peak,
        "gain_db": found.finish.gain_db,
        "ceiling_db": found.finish.ceiling_db,
        "before": reading(&found.before),
        "after": after,
        "met": found.met,
    })
}

/// A number of JSON: none for −∞ (the true peak of silence).
fn finite(value: f64) -> Option<f64> {
    value.is_finite().then_some(value)
}

#[cfg(test)]
#[path = "sound_tests.rs"]
mod tests;
