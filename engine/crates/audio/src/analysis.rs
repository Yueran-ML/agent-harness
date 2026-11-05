//! Offline analysis (plan T9; Analyze Audio Level, p. 3986–3987), Normalize Audio Levels
//! (p. 3840–3841) and Level Matcher (p. 3804–3805): the loudness and the peaks of a whole sound,
//! and the gain that brings it to a level.
//!
//! [`Analyzer`] runs the same [`LoudnessTap`] and [`Loudness`] as the meter of the monitored bus,
//! fed as fast as the sound is rendered (T6's blocking read) instead of by the card. What it
//! measures is pure analysis: the change of a clip's Volume that Normalize and Level Matcher
//! make is an operation of the model (T2), one transaction for all the clips, the whole curve
//! of a clip with keyframes moved by the same amount (C-C5, C-C7).
//!
//! # What is measured of a timeline (T9b)
//!
//! A graph built for analysis (`Purpose::Analysis`, as an export: never a Listen) is run over a
//! range by [`run_strip`], and the sound at a point of one of its strips (`Graph::sound_at`) given
//! block by block to an [`Analyzer`]:
//!
//! - a track or a bus: after its fader (its mute, solo and VCAs too: what an export of it would
//!   carry), over the range asked for, the whole timeline where none is;
//! - a clip (Normalize, Level Matcher, Analyze Audio Level, p. 3986): its sound after its Volume
//!   (keys and all) and before its pan, after the configuration of its channels: the clip's chain
//!   is source → speed and stretch → clip EQ → clip effects → Volume → pan, so its equalizer and
//!   its effects are heard (the coordinator's decision for T9b, from the review of T4c), and so is
//!   its Volume, so that the gain that brings it to a level is what is added to its Volume and a
//!   second Normalize adds nothing (the review of T9b). Over the part of it on the timeline.
//!   [`alone`] makes the project in which a graph gives that before the fader of its track: the
//!   clip alone on its track, centred, without its fades and crossfades; the track without Trim,
//!   polarity, Ducker, inserts, Dynamics, equalizer and lanes; the other tracks of sound off, so
//!   that they are not read.

use std::ops::Range;

use columbina_model::{AudioParam, Project, TrackKind};

use crate::constants::{CLIP_VOLUME_RANGE, LOUDNESS_CELLS_KEPT};
use crate::feed::{ClipFeed, FeedError};
use crate::graph::{Graph, Point};
use crate::loudness::{loudness_tap, Loudness, LoudnessReader, LoudnessTap, Reading};
use crate::node::Layout;
use crate::standards::{Measure, Standard};

/// What an analysis found, in LUFS, LU, dBTP and dBFS (see [`Reading`]).
pub type Analysis = Reading;

/// The analysis of a sound, fed block by block.
#[derive(Debug)]
pub struct Analyzer {
    tap: LoudnessTap,
    reader: LoudnessReader,
    loudness: Loudness,
    /// The most samples fed to the tap before its cells are read: half of what it keeps.
    chunk: usize,
    lost: u64,
}

impl Analyzer {
    /// An analysis of sound of `layout` at `rate`, reading the true peak if `true_peak` (it costs
    /// the most: 336 products a sample and a channel).
    #[must_use]
    pub fn new(rate: u32, layout: Layout, true_peak: bool) -> Self {
        let (tap, reader) = loudness_tap(rate, layout, true_peak);
        let chunk = tap.cell() * LOUDNESS_CELLS_KEPT / 2;
        Analyzer {
            tap,
            reader,
            loudness: Loudness::new(),
            chunk,
            lost: 0,
        }
    }

    /// Hears a block, planar, all channels of one length; channels past the layout's are not
    /// heard.
    pub fn take(&mut self, input: &[&[f32]]) {
        let input = &input[..input.len().min(self.tap.channels())];
        let frames = input.first().map_or(0, |channel| channel.len());
        let mut parts = [&[][..]; crate::constants::MAX_CHANNELS];
        let mut at = 0;
        while at < frames {
            let end = frames.min(at + self.chunk);
            for (part, channel) in parts.iter_mut().zip(input) {
                *part = &channel[at..end];
            }
            self.tap.take(&parts[..input.len()]);
            let loudness = &mut self.loudness;
            self.lost += self.reader.drain(|cell| loudness.push(cell));
            at = end;
        }
    }

    /// What it has found so far.
    #[must_use]
    pub fn reading(&self) -> Reading {
        self.loudness.reading()
    }

    /// The end of the sound: what it found, the peaks of the last samples counted.
    #[must_use]
    pub fn finish(mut self) -> Analysis {
        debug_assert_eq!(self.lost, 0, "an analysis reads every cell");
        let (true_peak, sample_peak) = self.tap.finish();
        self.loudness.push_peaks(true_peak, sample_peak);
        self.loudness.reading()
    }
}

/// The analysis of `channels` (planar) of `layout` at `rate`.
#[must_use]
pub fn analyze(rate: u32, layout: Layout, true_peak: bool, channels: &[&[f32]]) -> Analysis {
    let mut analyzer = Analyzer::new(rate, layout, true_peak);
    analyzer.take(channels);
    analyzer.finish()
}

/// What Normalize Audio Levels brings to the Reference Level (Normalization Mode, p. 3840).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    /// Sample Peak Program: the largest sample, in dBFS.
    SamplePeak,
    /// True Peak Program: the largest size between the samples, in dBTP.
    TruePeak,
    /// The integrated loudness of a standard, in LUFS.
    Loudness(Measure),
}

impl Level {
    /// The level of a loudness standard.
    #[must_use]
    pub fn of(standard: &Standard) -> Self {
        Level::Loudness(standard.measure)
    }

    /// Whether its analysis must read the true peak.
    #[must_use]
    pub fn needs_true_peak(self) -> bool {
        self == Level::TruePeak
    }

    /// The level of `analysis`, if it has one: none for silence, nor a loudness for a sound
    /// shorter than a block of 400 ms.
    #[must_use]
    pub fn of_analysis(self, analysis: &Analysis) -> Option<f64> {
        let level = match self {
            Level::SamplePeak => Some(analysis.sample_peak),
            Level::TruePeak => Some(analysis.true_peak),
            Level::Loudness(Measure::Gated) => analysis.integrated,
            Level::Loudness(Measure::Ungated) => analysis.integrated_ungated,
        };
        level.filter(|level| level.is_finite())
    }
}

/// Set Level (p. 3841): how the gain of several clips is found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetLevel {
    /// All the clips as one: one gain for all, from the level of them together (for a peak, the
    /// highest of them; for a loudness, the clips heard one after another in the order of the
    /// timeline, C-C6). The default (C-C5).
    Relative,
    /// Each clip by its own level.
    Independent,
}

/// The gain in dB that brings `analysis` to `target` by `level`; `None` when it has no level
/// (silence, or too short for its loudness), and its Volume is left as it is.
#[must_use]
pub fn normalize(level: Level, target: f64, analysis: &Analysis) -> Option<f64> {
    level
        .of_analysis(analysis)
        .map(|measured| target - measured)
}

/// The Volume of a clip before Normalize, in dB: its lowest and its highest value (its one value,
/// or those of its keyframes), to tell whether a gain takes it out of the range of a Volume.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Volume {
    /// The lowest.
    pub lowest: f64,
    /// The highest.
    pub highest: f64,
}

impl Volume {
    /// One value over all the clip.
    #[must_use]
    pub fn fixed(db: f64) -> Self {
        Volume {
            lowest: db,
            highest: db,
        }
    }

    /// The Volume of a clip: its one value, or the lowest and the highest of its keyframes.
    #[must_use]
    pub fn of(gain: &AudioParam) -> Self {
        match gain {
            AudioParam::Fixed(db) => Volume::fixed(*db),
            AudioParam::Keys(keys) => Volume {
                lowest: keys
                    .iter()
                    .map(|key| key.value)
                    .fold(f64::INFINITY, f64::min),
                highest: keys
                    .iter()
                    .map(|key| key.value)
                    .fold(f64::NEG_INFINITY, f64::max),
            },
        }
    }
}

/// What Normalize does to a clip.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Normalized {
    /// The gain in dB added to its Volume (the whole curve moved by it); `None` when it has no
    /// level and is left as it is.
    pub gain: Option<f64>,
    /// Whether the Volume, or a keyframe of it, moved by the gain would go out of
    /// [`CLIP_VOLUME_RANGE`] and is clamped there, so that the clip does not come to the target:
    /// the dialog tells the user which (Relative most of all, where one gain is for all).
    pub clamped: bool,
}

/// The gains of Normalize Audio Levels for clips: Independent, the gain of each by its own
/// analysis (`each`); Relative, the one gain of their analysis `together` for every clip; and
/// for each, whether its Volume (`volumes`, one for each clip) moved by it would be clamped.
///
/// # Panics
///
/// When `volumes` and `each` are not of one length.
#[must_use]
pub fn normalize_clips(
    level: Level,
    set: SetLevel,
    target: f64,
    each: &[Analysis],
    together: &Analysis,
    volumes: &[Volume],
) -> Vec<Normalized> {
    assert_eq!(each.len(), volumes.len(), "a Volume for each clip");
    let relative = normalize(level, target, together);
    each.iter()
        .zip(volumes)
        .map(|(analysis, volume)| {
            let gain = match set {
                SetLevel::Independent => normalize(level, target, analysis),
                SetLevel::Relative => relative,
            };
            let (low, high) = CLIP_VOLUME_RANGE;
            let clamped =
                gain.is_some_and(|gain| volume.lowest + gain < low || volume.highest + gain > high);
            Normalized { gain, clamped }
        })
        .collect()
}

/// The gain in dB that Level Matcher adds to the Volume of `clip` to match `reference` (C-C7):
/// the difference of their loudness, each by its integrated loudness, gated, or by its ungated
/// mean where it has none (shorter than a block of 400 ms, or under the gates), so that a
/// reference with pauses is not taken lower for a short clip; `None` where either is silent.
#[must_use]
pub fn level_match(reference: &Analysis, clip: &Analysis) -> Option<f64> {
    let loudness = |analysis: &Analysis| analysis.integrated.or(analysis.mean);
    let difference = loudness(reference).zip(loudness(clip)).map(|(r, c)| r - c);
    difference.filter(|difference| difference.is_finite())
}

/// Runs `graph` over `range` of the timeline (samples) and gives `sink` the sound at `point` of
/// strip `strip`, block by block (planar, a slice for each channel of the strip), until `sink`
/// says no more (`false`: the analysis is cancelled). Whether it ran to the end of the range.
/// [`run_strips`] for one point.
///
/// # Errors
///
/// The feed could not give the sound of a clip (as [`Graph::process`]).
///
/// # Panics
///
/// When the graph has no such strip.
pub fn run_strip(
    graph: &mut Graph,
    feed: &mut dyn ClipFeed,
    point: (usize, Point),
    range: Range<i64>,
    sink: &mut dyn FnMut(&[&[f32]]) -> bool,
) -> Result<bool, FeedError> {
    run_strips(graph, feed, &[point], range, &mut |_, sound| sink(sound))
}

/// Runs `graph` once over `range` of the timeline (samples) and gives `sink` the sound at each
/// of `points` (a strip and a point of it), with the place of the point among them, block by
/// block, until `sink` says no more (`false`: the analysis is cancelled); the same range of the
/// timeline of each, however far ahead of the outputs its strip is ([`Graph::lead`]): the graph
/// starts as far before the range as the strip furthest ahead is, and runs on as far after it as
/// the one least ahead is behind that. Whether it ran to the end.
///
/// # Errors
///
/// The feed could not give the sound of a clip (as [`Graph::process`]).
///
/// # Panics
///
/// When the graph has no such strip.
pub fn run_strips(
    graph: &mut Graph,
    feed: &mut dyn ClipFeed,
    points: &[(usize, Point)],
    range: Range<i64>,
    sink: &mut dyn FnMut(usize, &[&[f32]]) -> bool,
) -> Result<bool, FeedError> {
    let leads: Vec<i64> = points
        .iter()
        .map(|&(strip, _)| i64::from(graph.lead(strip)))
        .collect();
    let most = leads.iter().copied().max().unwrap_or(0);
    let least = leads.iter().copied().min().unwrap_or(0);
    let length = range.end - range.start;
    graph.start(range.start - most, feed)?;
    // Sample `p` of the run is sample `range.start + p − most + lead` of a strip's timeline.
    let mut at = 0;
    while at < length + most - least {
        let frames = usize::try_from(length + most - least - at)
            .unwrap_or(usize::MAX)
            .min(graph.max_block());
        graph.process(feed, frames)?;
        let end = at + i64::try_from(frames).unwrap_or(i64::MAX);
        for (index, (&(strip, point), lead)) in points.iter().zip(&leads).enumerate() {
            // The part of the block within the range, for this strip.
            let first = most - lead;
            let (from, to) = (at.max(first), end.min(first + length));
            if from < to {
                let offset = usize::try_from(from - at).unwrap_or(0);
                let count = usize::try_from(to - from).unwrap_or(0);
                let sound = graph.sound_at(strip, point);
                if !sound.read_range(offset, count, |channels| sink(index, channels)) {
                    return Ok(false);
                }
            }
        }
        at = end;
    }
    Ok(true)
}

/// The project in which the strip of the track of clip `clip` of timeline `timeline` gives the
/// clip's sound before its fader as an analysis measures a clip (see the module), the track's id,
/// and the clip's samples of the timeline; none where there is no such clip on a track of sound.
#[must_use]
pub fn alone(
    project: &Project,
    timeline: usize,
    clip: &str,
) -> Option<(Project, String, Range<i64>)> {
    let mut project = project.clone();
    let t = project.timelines.get_mut(timeline)?;
    let at = t.tracks.iter().position(|track| {
        track.kind == TrackKind::Audio && track.clips.iter().any(|c| c.id == clip)
    })?;
    let (num, den, rate) = (t.rate.num, t.rate.den, t.sample_rate);
    for (index, track) in t.tracks.iter_mut().enumerate() {
        if track.kind == TrackKind::Audio && index != at {
            track.enabled = false;
        }
    }
    let track = &mut t.tracks[at];
    track.enabled = true;
    track.clips.retain(|c| c.id == clip);
    track.transitions.clear();
    let audio = &mut track.audio;
    audio.mute = false;
    audio.solo = false;
    audio.lanes.clear();
    let strip = &mut audio.strip;
    strip.trim_db = 0.0;
    strip.invert = false;
    strip.track_fx = None;
    strip.inserts.clear();
    strip.eq.on = false;
    strip.dynamics.on = false;
    let c = &mut track.clips[0];
    c.enabled = true;
    // Its Volume stays, keys and all: a gain to bring it to a level is added to it (the review
    // of T9b), so it is measured after it.
    c.pan = AudioParam::Fixed(0.0);
    c.fade_in = 0;
    c.fade_out = 0;
    c.audio_fade_in = None;
    c.audio_fade_out = None;
    // Its equalizer and its effects stay: they come before its Volume.
    // The sample of the timeline at the start of a frame, to the sample below (as an export).
    let sample = |frame: i64| {
        i64::try_from(i128::from(frame) * i128::from(rate) * i128::from(den) / i128::from(num))
            .unwrap_or(i64::MAX)
    };
    let range = sample(c.start)..sample(c.end());
    let id = track.id.clone();
    Some((project, id, range))
}
