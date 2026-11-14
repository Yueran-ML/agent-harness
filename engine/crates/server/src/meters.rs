//! The meters of the timeline as it plays (plan T9b): the levels of each track and bus before and
//! after its fader, the Control Room meter, and the loudness meter of the bus that is monitored
//! (p. 3688, p. 3887–3888, p. 3979–3986), read from the graphs that play into a
//! [`columbina_audio::metering::Board`], and `audio.meters` and `audio.loudness`.
//!
//! One board for the engine ([`hub`]): the thread that builds the graphs of the sound
//! (`play::sound`) offers it the taps of each graph it builds, and a thread of its own reads them
//! [`METER_RATE_HZ`] times a second, so that no cell is lost however seldom a client asks, and the
//! ballistics are the same however often. A client polls `audio.meters` (the desktop at the rate of
//! its drawing, at most 30 times a second): what it gets is what the meters show then. The meter
//! type, the standard and the target of the loudness and Lock Metering to Transport are the
//! timeline's (`audio` of the timeline in the project), read at each request.

use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use columbina_audio::constants::METER_RATE_HZ;
use columbina_audio::graph::{Graph, Owner};
use columbina_audio::loudness::Reading;
use columbina_audio::meter::{position, Ballistics, Detector, MeterType, PeakIndicator, Scale};
use columbina_audio::metering::Board;
use columbina_audio::standards::{self, Measure, Standard, Verdict};
use columbina_model::audio::{
    Detector as ModelDetector, LoudnessStandard, MeterKind, MeterScale, PeakHold,
};
use columbina_model::TimelineAudio;
use serde_json::{json, Value};

/// What the meters of the engine read, and the timeline whose graphs it reads.
pub(crate) struct Meters {
    board: Mutex<(Option<String>, Board)>,
    reading: OnceLock<()>,
}

/// The meters of the engine.
pub(crate) fn hub() -> &'static Arc<Meters> {
    static HUB: OnceLock<Arc<Meters>> = OnceLock::new();
    HUB.get_or_init(|| Arc::new(Meters::new()))
}

impl Meters {
    /// Meters that have read nothing, read by no thread yet.
    pub(crate) fn new() -> Self {
        Meters {
            board: Mutex::new((None, Board::new(MeterType::default()))),
            reading: OnceLock::new(),
        }
    }

    fn board(&self) -> MutexGuard<'_, (Option<String>, Board)> {
        self.board.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Offers the taps of `graph`, built to play timeline `timeline` with the settings `audio`;
    /// `restart` where it is the first graph of a session that starts where the playhead was put.
    /// Meters of another timeline start anew. The first offer starts the thread that reads them.
    pub(crate) fn offer(
        self: &Arc<Self>,
        timeline: &str,
        audio: &TimelineAudio,
        graph: &Graph,
        restart: bool,
    ) {
        crate::fx_meters::offer(timeline, graph, restart);
        {
            let mut board = self.board();
            if board.0.as_deref() != Some(timeline) {
                *board = (Some(timeline.to_owned()), Board::new(meter_type(audio)));
            }
            board.1.set_meter(meter_type(audio));
            board.1.set_lock(audio.loudness.lock_to_transport);
            board.1.offer(graph.taps(), restart);
        }
        self.reading.get_or_init(|| {
            let meters = Arc::downgrade(self);
            let started = std::thread::Builder::new()
                .name("columbina-meters".into())
                .spawn(move || {
                    let period = Duration::from_secs_f64(1.0 / f64::from(METER_RATE_HZ));
                    let mut last = Instant::now();
                    loop {
                        std::thread::sleep(period);
                        let Some(meters) = meters.upgrade() else {
                            return;
                        };
                        let now = Instant::now();
                        meters.tick(now.duration_since(last).as_secs_f64());
                        last = now;
                    }
                });
            if let Err(error) = started {
                crate::log::log(
                    "meters_unavailable",
                    &json!({"message": format!("the thread of the meters cannot be started: {error}")}),
                );
            }
        });
    }

    /// Reads what the graphs gave in the last `elapsed` seconds.
    pub(crate) fn tick(&self, elapsed: f64) {
        self.board().1.tick(elapsed);
    }

    /// What `audio.meters` answers for timeline `timeline` with the settings `audio`; the meters
    /// of its tracks and buses let go of their peaks first where `reset`.
    pub(crate) fn shown(&self, timeline: &str, audio: &TimelineAudio, reset: bool) -> Value {
        let mut board = self.board();
        let ours = board.0.as_deref() == Some(timeline);
        let board = &mut board.1;
        let meter = meter_type(audio);
        let scale = meter.scale();
        if ours {
            board.set_meter(meter);
            board.set_lock(audio.loudness.lock_to_transport);
            if reset {
                board.reset_meters();
            }
        }
        let strips: Vec<Value> = if ours {
            board
                .strips()
                .iter()
                .map(|strip| {
                    let owner = match &strip.owner {
                        Owner::Track(id) => json!({"track": id}),
                        Owner::Bus(id) => json!({"bus": id}),
                        Owner::Vca(id) => json!({"vca": id}),
                    };
                    json!({
                        "owner": owner,
                        "pre": channels(&strip.pre, strip.channels, scale),
                        "post": channels(&strip.post, strip.channels, scale),
                    })
                })
                .collect()
        } else {
            Vec::new()
        };
        let room = if ours {
            board.room().map(|room| channel(room, 0, scale))
        } else {
            None
        };
        json!({
            "timeline": timeline,
            "playing": ours && board.hearing(),
            "rate": METER_RATE_HZ,
            "scale": match scale {
                Scale::Iec => "iec",
                Scale::QuasiLinear => "linear",
            },
            "strips": strips,
            "control_room": room,
            "loudness": loudness(board, ours, audio),
            "lost": board.lost(),
        })
    }

    /// `audio.loudness`: Start, Pause or Reset the loudness meter of timeline `timeline`, and what
    /// it reads then. Only a graph offered switches the timeline of the meters (the review of
    /// T9b): of another timeline, a read is of nothing measured, and an action is refused (none,
    /// which the caller tells as `not_metered`).
    pub(crate) fn control(
        &self,
        timeline: &str,
        audio: &TimelineAudio,
        action: Action,
    ) -> Option<Value> {
        let mut board = self.board();
        let ours = board.0.as_deref() == Some(timeline);
        let board = &mut board.1;
        match (action, ours) {
            (Action::Read, _) => {}
            (_, false) => return None,
            (Action::Start, true) => board.start(),
            (Action::Pause, true) => board.pause(),
            (Action::Reset, true) => board.reset(),
            (Action::ResetPeak, true) => board.reset_peaks(),
        }
        Some(loudness(board, ours, audio))
    }
}

/// What `audio.loudness` asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Action {
    /// Start, or Resume.
    Start,
    /// Pause.
    Pause,
    /// Reset.
    Reset,
    /// The true and sample peaks alone cleared (a click on TP, p. 3983).
    ResetPeak,
    /// Nothing: what it reads.
    Read,
}

/// The meter type of the timeline's settings (Project Settings › Fairlight › Audio Metering).
pub(crate) fn meter_type(audio: &TimelineAudio) -> MeterType {
    match audio.meter {
        MeterKind::Ppm => MeterType::Iec60268_18,
        MeterKind::Vu => MeterType::DigitalVu,
        MeterKind::Custom => {
            let custom = &audio.custom_meter;
            MeterType::Custom {
                detector: match custom.detector {
                    ModelDetector::Sample => Detector::SamplePeak,
                    ModelDetector::Vu => Detector::Vu,
                    ModelDetector::Rms => Detector::Rms,
                },
                scale: match custom.scale {
                    MeterScale::Iec => Scale::Iec,
                    MeterScale::Linear => Scale::QuasiLinear,
                },
                decay: custom.decay_s,
                peak: match custom.peak {
                    PeakHold::Off => PeakIndicator::Off,
                    PeakHold::ShortHoldFall => PeakIndicator::ShortHoldFall,
                    PeakHold::MediumHoldFall => PeakIndicator::MediumHoldFall,
                    PeakHold::MediumHold => PeakIndicator::MediumHold,
                    PeakHold::LongHold => PeakIndicator::LongHold,
                    PeakHold::InfiniteHold => PeakIndicator::InfiniteHold,
                },
            }
        }
    }
}

/// The standard of `columbina_audio::standards` that the project's names.
pub(crate) fn standard_of(standard: LoudnessStandard) -> &'static Standard {
    let id = match standard {
        LoudnessStandard::Bs1770_1 => "bs1770-1",
        LoudnessStandard::Bs1770_4 => "bs1770-4",
        LoudnessStandard::AtscA85 => "atsc-a85",
        LoudnessStandard::EbuR128 => "ebu-r128",
        LoudnessStandard::Op59 => "op-59",
        LoudnessStandard::TrB32 => "tr-b32",
        LoudnessStandard::Agcom219 => "agcom-219",
        LoudnessStandard::Netflix => "netflix",
        LoudnessStandard::Youtube => "youtube",
        LoudnessStandard::Disney2_0 => "disney-2.0",
        LoudnessStandard::Disney5_1 => "disney-5.1",
        LoudnessStandard::ApplePodcasts => "apple-podcasts",
        LoudnessStandard::Spotify => "spotify",
        LoudnessStandard::AesStreaming => "aes-streaming-dialogue",
        LoudnessStandard::WebVideo => "web-video",
    };
    standards::by_id(id).unwrap_or(&standards::STANDARDS[3])
}

/// Decibels to a hundredth, or `null` for silence (−∞) and what is not a number.
pub(crate) fn decibels(db: f64) -> Value {
    if db.is_finite() {
        json!((db * 100.0).round() / 100.0)
    } else {
        Value::Null
    }
}

/// What a meter of `channels` shows, each channel as [`channel`].
fn channels(meter: &Ballistics, channels: usize, scale: Scale) -> Value {
    (0..channels).map(|at| channel(meter, at, scale)).collect()
}

/// What channel `channel` of a meter shows, `{bar, peak, bar_position, peak_position}`: decibels,
/// and where they stand on `scale` from 0 (−60 dB and under) to 1 (0 dB and over).
fn channel(meter: &Ballistics, channel: usize, scale: Scale) -> Value {
    #[allow(clippy::cast_possible_truncation, reason = "a level shown")]
    let at = |db: f64| json!((f64::from(position(db as f32, scale)) * 1e4).round() / 1e4);
    let shown = meter.shown(channel);
    json!({
        "bar": decibels(shown.bar),
        "peak": shown.peak.map_or(Value::Null, decibels),
        "bar_position": at(shown.bar),
        "peak_position": shown.peak.map_or(Value::Null, at),
    })
}

/// What the loudness meter reads, by the timeline's standard and target: the integrated loudness
/// as the standard measures it (BS.1770-1 ungated), and where it stands.
fn loudness(board: &Board, ours: bool, audio: &TimelineAudio) -> Value {
    // What a meter that has heard nothing reads, made once.
    static NOTHING: OnceLock<Reading> = OnceLock::new();
    let standard = standard_of(audio.loudness.standard);
    #[allow(clippy::cast_possible_truncation, reason = "a loudness in LUFS")]
    let target = standard.target_or(audio.loudness.target as f32);
    let reading = if ours {
        board.reading()
    } else {
        *NOTHING.get_or_init(|| columbina_audio::loudness::Loudness::new().reading())
    };
    let integrated = match standard.measure {
        Measure::Gated => reading.integrated,
        Measure::Ungated => reading.integrated_ungated,
    };
    let verdict = integrated.filter(|lufs| lufs.is_finite()).map(|lufs| {
        match standard.verdict(lufs, target) {
            Verdict::Low => "low",
            Verdict::Within => "within",
            Verdict::High => "high",
        }
    });
    json!({
        "running": if ours { board.running() } else { true },
        "standard": standard.id,
        "target": decibels(f64::from(target)),
        "momentary": decibels(reading.momentary),
        "short_term": decibels(reading.short_term),
        "momentary_max": decibels(reading.momentary_max),
        "short_term_max": decibels(reading.short_term_max),
        "integrated": integrated.map_or(Value::Null, decibels),
        "range": reading.range.map_or(Value::Null, decibels),
        "true_peak": decibels(reading.true_peak),
        "sample_peak": decibels(reading.sample_peak),
        "seconds": (reading.seconds * 100.0).round() / 100.0,
        "verdict": verdict,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use columbina_model::audio::CustomMeter;

    #[test]
    fn every_standard_of_the_project_is_one_of_the_table() {
        let all = [
            LoudnessStandard::Bs1770_1,
            LoudnessStandard::Bs1770_4,
            LoudnessStandard::AtscA85,
            LoudnessStandard::EbuR128,
            LoudnessStandard::Op59,
            LoudnessStandard::TrB32,
            LoudnessStandard::Agcom219,
            LoudnessStandard::Netflix,
            LoudnessStandard::Youtube,
            LoudnessStandard::Disney2_0,
            LoudnessStandard::Disney5_1,
            LoudnessStandard::ApplePodcasts,
            LoudnessStandard::Spotify,
            LoudnessStandard::AesStreaming,
            LoudnessStandard::WebVideo,
        ];
        let ids: Vec<&str> = all
            .iter()
            .map(|&standard| standard_of(standard).id)
            .collect();
        // In the order of the table, each once.
        let table: Vec<&str> = standards::STANDARDS
            .iter()
            .map(|standard| standard.id)
            .collect();
        assert_eq!(ids, table);
        assert_eq!(standard_of(LoudnessStandard::default()).id, "ebu-r128");
    }

    #[test]
    fn the_meter_type_is_the_timelines() {
        let mut audio = TimelineAudio::default();
        assert_eq!(meter_type(&audio), MeterType::Iec60268_18);
        audio.meter = MeterKind::Vu;
        assert_eq!(meter_type(&audio), MeterType::DigitalVu);
        audio.meter = MeterKind::Custom;
        audio.custom_meter = CustomMeter {
            detector: ModelDetector::Rms,
            scale: MeterScale::Linear,
            decay_s: 0.5,
            peak: PeakHold::InfiniteHold,
        };
        assert_eq!(
            meter_type(&audio),
            MeterType::Custom {
                detector: Detector::Rms,
                scale: Scale::QuasiLinear,
                decay: 0.5,
                peak: PeakIndicator::InfiniteHold,
            }
        );
    }

    #[test]
    fn only_a_graph_offered_switches_the_timeline_of_the_meters() {
        use columbina_audio::graph::{Options, Purpose};
        let project = columbina_model::Project::sample();
        let graph = Graph::build(&project, 0, Options::new(512, Purpose::Preview)).unwrap();
        let audio = TimelineAudio::default();
        let meters = Arc::new(Meters::new());
        // Nothing metered yet: a read reads nothing, an action is refused.
        let read = meters.control("t1", &audio, Action::Read).unwrap();
        assert_eq!(read["seconds"], json!(0.0));
        assert!(meters.control("t1", &audio, Action::Pause).is_none());
        meters.offer("t1", &audio, &graph, true);
        assert_eq!(
            meters.control("t1", &audio, Action::Pause).unwrap()["running"],
            false
        );
        // Another timeline: read, it is of nothing and leaves the meters of t1 as they are.
        let other = meters.control("t2", &audio, Action::Read).unwrap();
        assert_eq!(other["running"], true);
        assert!(meters.control("t2", &audio, Action::Reset).is_none());
        assert!(meters.control("t2", &audio, Action::ResetPeak).is_none());
        assert_eq!(meters.shown("t2", &audio, false)["strips"], json!([]));
        let shown = meters.shown("t1", &audio, false);
        assert_eq!(shown["loudness"]["running"], false);
        // A click on TP clears the peaks of t1 and leaves it paused.
        let cleared = meters.control("t1", &audio, Action::ResetPeak).unwrap();
        assert_eq!(cleared["running"], false);
        assert_eq!(cleared["true_peak"], Value::Null);
        let action: Action = serde_json::from_value(json!("reset_peak")).unwrap();
        assert_eq!(action, Action::ResetPeak);
    }

    #[test]
    fn decibels_are_told_to_a_hundredth_and_silence_as_null() {
        assert_eq!(decibels(-23.004_9), json!(-23.0));
        assert_eq!(decibels(-6.026), json!(-6.03));
        assert_eq!(decibels(f64::NEG_INFINITY), Value::Null);
        assert_eq!(decibels(f64::NAN), Value::Null);
    }
}
