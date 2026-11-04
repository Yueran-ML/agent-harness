//! Noise Reduction (plan T12; Reference Manual 20, p. 3970; the interaction reference §10.5 and
//! C-X4): a spectral gate of our own, its noise followed as it goes (Auto Speech) or learned from
//! a stretch of noise (Manual), and a voice mode by `RNNoise` ([`crate::fx::rnnoise`]).
//!
//! # The transform
//!
//! A short-time transform of [`FFT`] points every [`HOP`] samples (75 % overlap) on a grid laid
//! on the timeline ([`Node::align`]). Each frame is weighed by a periodic Hann window
//! `wa(n) = sin²(πn/N)` before the transform and by a synthesis window `ws` after it, and added
//! to what the frames before it left:
//!
//! ```text
//! ws(n) = 0                                           for n < R
//! ws(n) = 2R/(N − R) · sin²(π(n − R)/(N − R)) / wa(n)   for R ≤ n < N
//! ```
//!
//! with N = 2048 and R = 512. The product `wa·ws` is a Hann window of N − R samples on the end
//! of the frame, three of which overlap to 1 at every sample (Σ over the hops = 1), so that a
//! frame left as it is gives the input back. Since `ws` is 0 on the first hop of a frame, a
//! sample is complete once the frame that ends N − R samples after it is done: the delay is
//! [`LATENCY`] = N − R = 1536 samples, whatever the blocks, to the sample (the plan; the
//! symmetric windows of the reference's C-X4 would need 2048).
//!
//! # The gate
//!
//! For each frame, channel by channel, and each bin `k` of power `P = |X|²`:
//!
//! 1. `Q` is the mean of `P` over the bins `k − H … k + H`, H = Frequency Smoothing · 8 bins
//!    ([`DENOISE_FREQUENCY_SMOOTHING_BINS`]), clamped within [`DENOISE_POWER_FLOOR`] and
//!    [`DENOISE_POWER_CEILING`]: the smoothing widens what a strong partial keeps open and
//!    steadies the noise, against "harmonic ringing" (p. 3970).
//! 2. The noise `N`. Auto Speech follows it: the signal profile `S` is `ln Q` smoothed with the
//!    time constant `attack / ratio`, and the noise profile `ln N` follows the lowest `S`: down at
//!    once, up with the time constant `attack` (Attack sets how fast the noise profile updates,
//!    Ratio the attack of the signal profile relative to it, p. 3970). Manual takes the power
//!    learned ([`learn`]) and follows nothing; with nothing learned it follows as Auto Speech.
//! 3. The gain of the bin is `g = f + (1 − f)·r²/(1 + r²)` with `r = Q / (β·θ·N)`, θ the
//!    Threshold in power, β = 1 + 3·Sensitivity the overestimation of the noise (1 to 4, C-X4),
//!    and `f` the floor of [`DENOISE_FLOOR_DB`]: open (1) well above the mask, at `f` well
//!    below. De-Rumble and De-Hiss gate only below 250 Hz and above 4 kHz ([`Band`]).
//! 4. With Time Smoothing on, the gain follows `g` over the frames with the time constant of
//!    [`DENOISE_TIME_SMOOTHING`].
//!
//! The first frame after a reset that holds N samples of input starts the profiles at what it
//! hears; the frames before it, partly before the start, pass as they are.
//!
//! # After the gate
//!
//! The dry sound is the input delayed by [`LATENCY`], so that every mix of the two is aligned.
//! Level is a gain on the processed sound before the mix (p. 3970); Dry/Wet mixes; Listen to
//! Noise Only lets out the dry sound less the processed one (what is taken away); On is the
//! bypass, the delay kept. These four move sample by sample, ramped over 10 ms; a mix of 0 or 1
//! takes one side to the bit. The numbers of the gate take effect frame by frame, at the last
//! sample of the frame, and the overlap of the frames spreads a change over 1536 samples.
//!
//! # Changes of the structure
//!
//! The mode, and the profile learned, are not parameters: switching between the gate and the
//! voice changes the latency, and the profile is not one number. The graph builds the node again
//! with them ([`NoiseReduction::new`]) and swaps it in (T4a, T5).

use crate::constants::{
    samples, DENOISE_FFT, DENOISE_FLOOR_DB, DENOISE_FREQUENCY_SMOOTHING_BINS, DENOISE_HISS_HZ,
    DENOISE_HOP, DENOISE_INPUT_LIMIT, DENOISE_LATENCY, DENOISE_POWER_CEILING, DENOISE_POWER_FLOOR,
    DENOISE_RUMBLE_HZ, DENOISE_SENSITIVITY_FACTOR, DENOISE_SETTLE_TIME_CONSTANTS,
    DENOISE_TIME_SMOOTHING, SMOOTHING,
};
use crate::fft::{Complex32, RealFft};
use crate::fx::rnnoise::Voice;
use crate::node::{Block, Layout, Node, ParamId, Planar, Spec, Unsupported};
use crate::rt::{Domain, Smoothed};

/// The points of the transform.
pub const FFT: usize = DENOISE_FFT;

/// The hop between frames.
pub const HOP: usize = DENOISE_HOP;

/// The delay of the spectral gate, in samples: [`FFT`] − [`HOP`].
pub const LATENCY: usize = DENOISE_LATENCY;

/// The bins of a spectrum: [`FFT`] / 2 + 1.
pub const BINS: usize = FFT / 2 + 1;

// --- Modes, bands, presets -------------------------------------------------------------------

/// Where the noise comes from (the model's `NoiseMode`; the interaction reference §10.5).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Mode {
    /// The noise followed as it goes (p. 3970).
    #[default]
    AutoSpeech,
    /// The noise learned from a stretch of it.
    Manual,
    /// `RNNoise`: only Dry/Wet, Level, Listen and On apply.
    Voice,
}

/// The frequencies that the gate works on: all, or those of the presets De-Rumble and De-Hiss
/// (C-X4).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Band {
    /// Every frequency.
    #[default]
    Full,
    /// Below [`DENOISE_RUMBLE_HZ`] (De-Rumble).
    Rumble,
    /// Above [`DENOISE_HISS_HZ`] (De-Hiss).
    Hiss,
    /// Both (De-Rumble and Hiss).
    RumbleAndHiss,
}

impl Band {
    /// Every band, in the order of their codes.
    pub const ALL: [Band; 4] = [Band::Full, Band::Rumble, Band::Hiss, Band::RumbleAndHiss];

    /// Its code for [`Param::Band`]: its place in [`Band::ALL`].
    #[must_use]
    pub const fn code(self) -> u8 {
        self as u8
    }

    /// The band of `code`, a whole number from 0 to 3.
    #[must_use]
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a whole number from 0 to 3"
    )]
    pub fn from_code(code: f32) -> Option<Band> {
        if code.fract() != 0.0 || !(0.0..=3.0).contains(&code) {
            return None;
        }
        Some(Band::ALL[code as usize])
    }

    /// Whether the gate works on `hz`.
    #[must_use]
    pub fn gates(self, hz: f32) -> bool {
        match self {
            Band::Full => true,
            Band::Rumble => hz < DENOISE_RUMBLE_HZ,
            Band::Hiss => hz > DENOISE_HISS_HZ,
            Band::RumbleAndHiss => hz < DENOISE_RUMBLE_HZ || hz > DENOISE_HISS_HZ,
        }
    }
}

/// The presets of Noise Reduction (p. 3970): the defaults, with the gate on a band.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Preset {
    /// The defaults.
    Default,
    /// The gate above 4 kHz.
    DeHiss,
    /// The gate below 250 Hz.
    DeRumble,
    /// Both.
    DeRumbleAndHiss,
}

impl Preset {
    /// Every preset, in the order of the menu.
    pub const ALL: [Preset; 4] = [
        Preset::Default,
        Preset::DeHiss,
        Preset::DeRumble,
        Preset::DeRumbleAndHiss,
    ];

    /// Its name in the menu.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Preset::Default => "Default",
            Preset::DeHiss => "De-Hiss",
            Preset::DeRumble => "De-Rumble",
            Preset::DeRumbleAndHiss => "De-Rumble and Hiss",
        }
    }

    /// Its band.
    #[must_use]
    pub const fn band(self) -> Band {
        match self {
            Preset::Default => Band::Full,
            Preset::DeHiss => Band::Hiss,
            Preset::DeRumble => Band::Rumble,
            Preset::DeRumbleAndHiss => Band::RumbleAndHiss,
        }
    }
}

// --- Parameters ------------------------------------------------------------------------------

/// A parameter of Noise Reduction, set by [`Node::set`] with its [`Param::id`], in the units of
/// the model's `NoiseReductionFx` (`columbina-model`, `audio/fx.rs`): decibels, milliseconds,
/// percent, and switches as 0 or 1. The mode and the profile learned are not parameters (see
/// the module).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Param {
    /// On: off is the bypass, the delay kept (the insert's `on`).
    On,
    /// Threshold, dB, 0 to 40: how far above the noise a bin must be to pass.
    ThresholdDb,
    /// Attack, ms, 50 to 5000: the time constant of the noise profile.
    AttackMs,
    /// Sensitivity, percent, 0 to 100: the noise overestimated 1 to 4 times.
    Sensitivity,
    /// Ratio, 1 to 20: the attack of the signal profile is Attack divided by it.
    Ratio,
    /// Frequency Smoothing, percent, 0 to 100.
    FrequencySmoothing,
    /// Time Smoothing on.
    TimeSmoothing,
    /// Dry/Wet, percent, 0 (dry) to 100 (wet).
    DryWet,
    /// Level, dB, −6 to 18: on the processed sound before the mix.
    LevelDb,
    /// Listen to Noise Only.
    Listen,
    /// The band of the gate ([`Band::code`]): set by the presets.
    Band,
}

/// The count of the parameters.
pub const PARAMS: usize = 11;

impl Param {
    /// Every parameter, in the order of their ids.
    pub const ALL: [Param; PARAMS] = [
        Param::On,
        Param::ThresholdDb,
        Param::AttackMs,
        Param::Sensitivity,
        Param::Ratio,
        Param::FrequencySmoothing,
        Param::TimeSmoothing,
        Param::DryWet,
        Param::LevelDb,
        Param::Listen,
        Param::Band,
    ];

    /// Its id for [`Node::set`] and the lanes of automation: its place in [`Param::ALL`].
    #[must_use]
    #[allow(clippy::cast_possible_truncation, reason = "11 parameters")]
    pub const fn id(self) -> ParamId {
        ParamId(self as u32)
    }

    /// The parameter of `id`.
    #[must_use]
    pub fn from_id(id: ParamId) -> Option<Param> {
        Param::ALL.get(usize::try_from(id.0).ok()?).copied()
    }

    /// Where it is in the model: the insert's `on`, the fields of `NoiseReductionFx`, and `band`,
    /// which the model has yet to take (T1).
    #[must_use]
    pub const fn path(self) -> &'static str {
        match self {
            Param::On => "on",
            Param::ThresholdDb => "threshold_db",
            Param::AttackMs => "attack_ms",
            Param::Sensitivity => "sensitivity",
            Param::Ratio => "ratio",
            Param::FrequencySmoothing => "frequency_smoothing",
            Param::TimeSmoothing => "time_smoothing",
            Param::DryWet => "dry_wet",
            Param::LevelDb => "level_db",
            Param::Listen => "listen",
            Param::Band => "band",
        }
    }

    /// Whether a lane of automation may set it: not Listen, which is for the monitor, nor the
    /// band of a preset.
    #[must_use]
    pub const fn is_automatable(self) -> bool {
        !matches!(self, Param::Listen | Param::Band)
    }

    /// Whether it is a switch, on at 0.5 and above.
    #[must_use]
    pub const fn is_switch(self) -> bool {
        matches!(self, Param::On | Param::TimeSmoothing | Param::Listen)
    }

    /// Whether the gate reads it once a frame (the rest move sample by sample).
    #[must_use]
    pub const fn is_spectral(self) -> bool {
        matches!(
            self,
            Param::ThresholdDb
                | Param::AttackMs
                | Param::Sensitivity
                | Param::Ratio
                | Param::FrequencySmoothing
                | Param::TimeSmoothing
                | Param::Band
        )
    }

    /// Its range (C-X4; Level from p. 3970).
    #[must_use]
    pub const fn range(self) -> (f32, f32) {
        match self {
            Param::ThresholdDb => (0.0, 40.0),
            Param::AttackMs => (50.0, 5000.0),
            Param::Sensitivity | Param::FrequencySmoothing | Param::DryWet => (0.0, 100.0),
            Param::Ratio => (1.0, 20.0),
            Param::LevelDb => (-6.0, 18.0),
            Param::Band => (0.0, 3.0),
            Param::On | Param::TimeSmoothing | Param::Listen => (0.0, 1.0),
        }
    }

    /// `value` clamped to the range; a switch as 0 or 1, a band to the nearest code.
    #[must_use]
    pub fn clamp(self, value: f32) -> f32 {
        let (low, high) = self.range();
        if self.is_switch() {
            if value >= 0.5 {
                1.0
            } else {
                0.0
            }
        } else if self == Param::Band {
            value.round().clamp(low, high)
        } else {
            value.clamp(low, high)
        }
    }
}

/// Noise Reduction as set: the model's `NoiseReductionFx`, with the insert's `on` and the band of
/// the presets.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// On (not bypassed).
    pub on: bool,
    /// Where the noise comes from.
    pub mode: Mode,
    /// dB, 0 to 40: 10.
    pub threshold_db: f32,
    /// ms, 50 to 5000: 1000.
    pub attack_ms: f32,
    /// Percent: 50.
    pub sensitivity: f32,
    /// 1 to 20: 4.
    pub ratio: f32,
    /// Percent: 25.
    pub frequency_smoothing: f32,
    /// On.
    pub time_smoothing: bool,
    /// Percent: 100, all processed.
    pub dry_wet: f32,
    /// dB, −6 to 18: 0.
    pub level_db: f32,
    /// Listen to Noise Only: off.
    pub listen: bool,
    /// The band of the gate.
    pub band: Band,
    /// The noise learned, as the power of each bin ([`learn`]), for [`Mode::Manual`]. Of any
    /// length: it is spread over the [`BINS`] by the frequency of its bins.
    pub profile: Option<Vec<f32>>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            on: true,
            mode: Mode::AutoSpeech,
            threshold_db: 10.0,
            attack_ms: 1000.0,
            sensitivity: 50.0,
            ratio: 4.0,
            frequency_smoothing: 25.0,
            time_smoothing: true,
            dry_wet: 100.0,
            level_db: 0.0,
            listen: false,
            band: Band::Full,
            profile: None,
        }
    }
}

/// A switch as 0 or 1.
fn switch(on: bool) -> f32 {
    if on {
        1.0
    } else {
        0.0
    }
}

impl Settings {
    /// The settings of `preset`.
    #[must_use]
    pub fn preset(preset: Preset) -> Self {
        Settings {
            band: preset.band(),
            ..Settings::default()
        }
    }

    /// The value of `param`, in the units of [`Node::set`].
    #[must_use]
    pub fn value(&self, param: Param) -> f32 {
        match param {
            Param::On => switch(self.on),
            Param::ThresholdDb => self.threshold_db,
            Param::AttackMs => self.attack_ms,
            Param::Sensitivity => self.sensitivity,
            Param::Ratio => self.ratio,
            Param::FrequencySmoothing => self.frequency_smoothing,
            Param::TimeSmoothing => switch(self.time_smoothing),
            Param::DryWet => self.dry_wet,
            Param::LevelDb => self.level_db,
            Param::Listen => switch(self.listen),
            Param::Band => f32::from(self.band.code()),
        }
    }

    /// Sets `param` to `value`, clamped; NaN changes nothing.
    pub fn set(&mut self, param: Param, value: f32) {
        if value.is_nan() {
            return;
        }
        let value = param.clamp(value);
        match param {
            Param::On => self.on = value >= 0.5,
            Param::ThresholdDb => self.threshold_db = value,
            Param::AttackMs => self.attack_ms = value,
            Param::Sensitivity => self.sensitivity = value,
            Param::Ratio => self.ratio = value,
            Param::FrequencySmoothing => self.frequency_smoothing = value,
            Param::TimeSmoothing => self.time_smoothing = value >= 0.5,
            Param::DryWet => self.dry_wet = value,
            Param::LevelDb => self.level_db = value,
            Param::Listen => self.listen = value >= 0.5,
            Param::Band => {
                if let Some(band) = Band::from_code(value) {
                    self.band = band;
                }
            }
        }
    }

    /// Every parameter with its value: set one by one into a node of the same mode and profile,
    /// they make it as [`NoiseReduction::new`] of these settings.
    #[must_use]
    pub fn params(&self) -> [(ParamId, f32); PARAMS] {
        Param::ALL.map(|param| (param.id(), self.value(param)))
    }

    /// The same, every number within its range (NaN as the default).
    #[must_use]
    pub fn clamped(&self) -> Settings {
        let mut clamped = Settings {
            profile: self.profile.clone(),
            mode: self.mode,
            ..Settings::default()
        };
        for param in Param::ALL {
            clamped.set(param, self.value(param));
        }
        clamped
    }
}

// --- The windows and the transform -----------------------------------------------------------

/// The analysis window: a periodic Hann window of [`FFT`] samples, sin²(πn/N).
#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    reason = "a window of 2048 samples"
)]
pub fn analysis_window() -> Vec<f32> {
    (0..FFT)
        .map(|n| hann(n as f64 / FFT as f64) as f32)
        .collect() // rt-ok: made when the node is built
}

/// The synthesis window: 0 on the first [`HOP`], then a periodic Hann window of
/// [`FFT`] − [`HOP`] samples scaled by 2R/(N − R), divided by the analysis window (see the
/// module).
#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    reason = "a window of 2048 samples"
)]
pub fn synthesis_window() -> Vec<f32> {
    let span = (FFT - HOP) as f64;
    let scale = 2.0 * HOP as f64 / span;
    (0..FFT)
        .map(|n| {
            if n < HOP {
                0.0
            } else {
                let analysis = hann(n as f64 / FFT as f64);
                (scale * hann((n - HOP) as f64 / span) / analysis) as f32
            }
        })
        .collect() // rt-ok: made when the node is built
}

/// sin²(πx).
fn hann(x: f64) -> f64 {
    let s = (std::f64::consts::PI * x).sin();
    s * s
}

/// A frame of [`Stft::process`]: the channel, the sample of the block on which it ends, and
/// whether every sample of it was heard since the reset (else part of it is the silence before
/// the start).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    /// The channel.
    pub channel: usize,
    /// The sample of the block at which the frame ends.
    pub at: usize,
    /// Every sample of it heard.
    pub complete: bool,
}

/// The short-time transform of the gate, channel by channel, with its overlap-add (see the
/// module): each frame's spectrum is handed to a function that may change it, and the sound comes
/// out [`LATENCY`] samples later. The frames end where the samples of the timeline, from the
/// origin ([`Stft::align`]), reach a multiple of [`HOP`].
pub struct Stft {
    fft: RealFft,
    analysis: Vec<f32>,
    synthesis: Vec<f32>,
    channels: usize,
    /// The last [`FFT`] samples of each channel, by the sample since the reset modulo FFT.
    input: Vec<f32>,
    /// What the frames add up to, for each channel, likewise.
    sum: Vec<f32>,
    frame: Vec<f32>,
    spectrum: Vec<Complex32>,
    /// The samples since the reset.
    count: u64,
    /// The sample of the timeline at which this run started.
    origin: u64,
}

impl std::fmt::Debug for Stft {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Stft")
            .field("channels", &self.channels)
            .field("count", &self.count)
            .field("origin", &self.origin)
            .finish_non_exhaustive()
    }
}

impl Stft {
    /// The transform of `channels`. Off the thread of the mix: it allocates.
    #[must_use]
    pub fn new(channels: usize) -> Self {
        let fft = RealFft::new(FFT);
        let frame = fft.signal();
        let spectrum = fft.spectrum();
        Stft {
            fft,
            analysis: analysis_window(),
            synthesis: synthesis_window(),
            channels,
            input: vec![0.0; channels * FFT], // rt-ok: made when the node is built
            sum: vec![0.0; channels * FFT],   // rt-ok: made when the node is built
            frame,
            spectrum,
            count: 0,
            origin: 0,
        }
    }

    /// The channels.
    #[must_use]
    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Back to silence, the origin kept.
    pub fn reset(&mut self) {
        self.input.fill(0.0);
        self.sum.fill(0.0);
        self.count = 0;
    }

    /// Lays the frames on the timeline from `origin`, the sample at which the run starts.
    pub fn align(&mut self, origin: u64) {
        self.origin = origin;
    }

    /// Processes `frames` samples of `input` into `wet` (the frames as `shape` left them) and
    /// `dry` (the input delayed alike), calling `shape` on the spectrum of every frame that ends
    /// in the block, channel by channel. A sample that is not finite is taken as 0. Takes no
    /// memory.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "a place in a ring of 2048 samples"
    )]
    pub fn process(
        &mut self,
        frames: usize,
        input: &[&[f32]],
        wet: &mut [&mut [f32]],
        dry: &mut [&mut [f32]],
        mut shape: impl FnMut(Frame, &mut [Complex32]),
    ) {
        let size = FFT as u64;
        let mut n = 0;
        while n < frames {
            let phase = (self.origin.wrapping_add(self.count) % HOP as u64) as usize;
            let to_end = HOP - phase;
            let len = to_end.min(frames - n);
            for channel in 0..self.channels {
                let ring = &mut self.input[channel * FFT..(channel + 1) * FFT];
                let sum = &mut self.sum[channel * FFT..(channel + 1) * FFT];
                let (x, wet, dry) = (&input[channel], &mut wet[channel], &mut dry[channel]);
                for i in 0..len {
                    let at = ((self.count + i as u64) % size) as usize;
                    let sample = x[n + i];
                    ring[at] = if sample.is_finite() { sample } else { 0.0 };
                    let back = (at + FFT - LATENCY) % FFT;
                    dry[n + i] = ring[back];
                    wet[n + i] = sum[back];
                    sum[back] = 0.0;
                }
            }
            n += len;
            self.count += len as u64;
            if len == to_end {
                let complete = self.count >= size;
                let first = (self.count % size) as usize;
                for channel in 0..self.channels {
                    let ring = &self.input[channel * FFT..(channel + 1) * FFT];
                    for (i, (sample, window)) in
                        self.frame.iter_mut().zip(&self.analysis).enumerate()
                    {
                        let x = ring[(first + i) % FFT];
                        *sample = x.clamp(-DENOISE_INPUT_LIMIT, DENOISE_INPUT_LIMIT) * window;
                    }
                    self.fft.forward(&mut self.frame, &mut self.spectrum);
                    shape(
                        Frame {
                            channel,
                            at: n - 1,
                            complete,
                        },
                        &mut self.spectrum,
                    );
                    self.fft.inverse(&mut self.spectrum, &mut self.frame);
                    let sum = &mut self.sum[channel * FFT..(channel + 1) * FFT];
                    for i in HOP..FFT {
                        sum[(first + i) % FFT] += self.frame[i] * self.synthesis[i];
                    }
                }
            }
        }
    }
}

/// The noise of `channels`, a stretch of it, learned for [`Mode::Manual`]: the mean power of
/// each bin over every whole frame in it (from its first sample, a frame every [`HOP`]) and over
/// the channels, weighed as the gate weighs them. `None` when it is shorter than a frame.
/// Off the thread of the mix.
#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    reason = "a mean of powers"
)]
pub fn learn(channels: &[&[f32]]) -> Option<Vec<f32>> {
    let length = channels.iter().map(|channel| channel.len()).min()?;
    if length < FFT {
        return None;
    }
    let mut fft = RealFft::new(FFT);
    let analysis = analysis_window();
    let mut frame = fft.signal();
    let mut spectrum = fft.spectrum();
    let mut sums = vec![0.0_f64; BINS]; // rt-ok: learned off the thread of the mix
    let mut count = 0_u64;
    for channel in channels {
        let mut start = 0;
        while start + FFT <= length {
            for ((sample, window), x) in frame.iter_mut().zip(&analysis).zip(&channel[start..]) {
                let x = if x.is_finite() { *x } else { 0.0 };
                *sample = x.clamp(-DENOISE_INPUT_LIMIT, DENOISE_INPUT_LIMIT) * window;
            }
            fft.forward(&mut frame, &mut spectrum);
            for (sum, bin) in sums.iter_mut().zip(&spectrum) {
                *sum += f64::from(bin.norm_sqr());
            }
            count += 1;
            start += HOP;
        }
    }
    let mean = |sum: &f64| (sum / count as f64) as f32;
    let profile: Vec<f32> = sums.iter().map(mean).collect(); // rt-ok: off the thread of the mix
    Some(profile)
}

// --- The node of the gate --------------------------------------------------------------------

/// A switch that cross-fades over the ramp of 10 ms: its weight is `count / ramp`, so that it
/// is 0 or 1 to the bit at its ends.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Switch {
    pub(crate) on: bool,
    count: u32,
    ramp: u32,
}

impl Switch {
    pub(crate) fn new(on: bool) -> Self {
        Switch {
            on,
            count: u32::from(on),
            ramp: 1,
        }
    }

    pub(crate) fn prepare(&mut self, rate: u32) {
        self.ramp = u32::try_from(samples(SMOOTHING, rate))
            .unwrap_or(u32::MAX)
            .max(1);
        self.finish();
    }

    pub(crate) fn finish(&mut self) {
        self.count = if self.on { self.ramp } else { 0 };
    }

    /// The weight of the next sample, going to `lane` when it is a finite value, else to the
    /// switch as set.
    #[allow(
        clippy::cast_precision_loss,
        reason = "a ramp of a few thousand samples"
    )]
    pub(crate) fn next(&mut self, lane: Option<f32>) -> f32 {
        let on = match lane {
            Some(value) if value.is_finite() => value >= 0.5,
            _ => self.on,
        };
        if on {
            self.count = (self.count + 1).min(self.ramp);
        } else {
            self.count = self.count.saturating_sub(1);
        }
        self.count as f32 / self.ramp as f32
    }
}

/// The mix after the processing, shared by the gate and the voice: Dry/Wet and Level ramped,
/// Listen and On cross-faded.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Mixer {
    dry_wet: Smoothed,
    level: Smoothed,
    pub(crate) listen: Switch,
    pub(crate) on: Switch,
}

/// `a` mixed with `b` by `t`: `a` or `b` to the bit at 0 and 1.
#[allow(clippy::float_cmp, reason = "the ends of a mix are taken to the bit")]
fn mix(a: f32, b: f32, t: f32) -> f32 {
    if t == 0.0 {
        a
    } else if t == 1.0 {
        b
    } else {
        a + t * (b - a)
    }
}

/// A lane's value at `n`, when it is there and finite.
fn lane_at(lane: Option<&[f32]>, n: usize) -> Option<f32> {
    lane.map(|values| values[n])
        .filter(|value| value.is_finite())
}

impl Mixer {
    pub(crate) fn new(settings: &Settings) -> Self {
        Mixer {
            dry_wet: Smoothed::new(settings.dry_wet / 100.0, Domain::Linear),
            level: Smoothed::new(crate::db_to_gain(settings.level_db), Domain::Linear),
            listen: Switch::new(settings.listen),
            on: Switch::new(settings.on),
        }
    }

    pub(crate) fn prepare(&mut self, rate: u32) {
        self.dry_wet.prepare(rate);
        self.level.prepare(rate);
        self.listen.prepare(rate);
        self.on.prepare(rate);
    }

    pub(crate) fn finish(&mut self) {
        self.dry_wet.finish();
        self.level.finish();
        self.listen.finish();
        self.on.finish();
    }

    /// Sets a parameter of the mix (the others are ignored); the value is clamped, not NaN.
    pub(crate) fn set(&mut self, param: Param, value: f32) {
        match param {
            Param::DryWet => self.dry_wet.set(value / 100.0),
            Param::LevelDb => self.level.set(crate::db_to_gain(value)),
            Param::Listen => self.listen.on = value >= 0.5,
            Param::On => self.on.on = value >= 0.5,
            _ => {}
        }
    }

    /// Mixes `frames` samples: `out` holds the processed sound and becomes the output, `dry` the
    /// input delayed alike. A processed sample `y` and a dry one `d` give
    /// `mix(d, mix(mix(d, level·y, wet), d − y, listen), on)`.
    pub(crate) fn mix(
        &mut self,
        frames: usize,
        automation: &crate::node::Automation,
        dry: &[&[f32]],
        out: &mut [&mut [f32]],
    ) {
        let lane = |param: Param| automation.get(param.id());
        let (dry_wet, level, on) = (lane(Param::DryWet), lane(Param::LevelDb), lane(Param::On));
        for n in 0..frames {
            let set_wet = self.dry_wet.next_value();
            let wet =
                lane_at(dry_wet, n).map_or(set_wet, |value| Param::DryWet.clamp(value) / 100.0);
            let set_level = self.level.next_value();
            let gain = lane_at(level, n).map_or(set_level, |value| {
                crate::db_to_gain(Param::LevelDb.clamp(value))
            });
            let listen = self.listen.next(None);
            let bypass = self.on.next(lane_at(on, n));
            for (out, dry) in out.iter_mut().zip(dry) {
                let (y, d) = (out[n], dry[n]);
                let processed = mix(d, gain * y, wet);
                let heard = mix(processed, d - y, listen);
                out[n] = mix(d, heard, bypass);
            }
        }
    }
}

/// The state of the gate on one channel.
#[derive(Clone, Debug, Default)]
struct Track {
    /// The signal profile, ln of a power, for each bin.
    signal: Vec<f32>,
    /// The noise profile, likewise.
    noise: Vec<f32>,
    /// The gain of each bin, smoothed over time.
    gain: Vec<f32>,
    /// A whole frame has been heard since the reset.
    started: bool,
}

impl Track {
    fn new() -> Self {
        Track {
            signal: vec![0.0; BINS], // rt-ok: when the graph is built
            noise: vec![0.0; BINS],  // rt-ok: when the graph is built
            gain: vec![1.0; BINS],   // rt-ok: when the graph is built
            started: false,
        }
    }

    fn reset(&mut self) {
        self.signal.fill(0.0);
        self.noise.fill(0.0);
        self.gain.fill(1.0);
        self.started = false;
    }
}

/// What the gate reads once a frame.
#[derive(Clone, Copy, Debug)]
struct Gate {
    /// β·θ: the noise times this is the mask.
    mask: f64,
    /// The steps of the profiles and of the gain over a hop: 1 − exp(−hop / (τ·rate)).
    signal_step: f32,
    noise_step: f32,
    gain_step: f32,
    time_smoothing: bool,
    /// The bins on each side of the frequency smoothing.
    half: usize,
    band: Band,
    /// The frequency of a bin.
    bin_hz: f32,
}

/// 1 − exp(−hop / (τ·rate)): the step of a one-pole follower of time constant `tau` seconds
/// over a hop.
#[allow(clippy::cast_possible_truncation, reason = "a step from 0 to 1")]
fn step(tau: f64, rate: u32) -> f32 {
    #[allow(clippy::cast_precision_loss, reason = "the hop")]
    let hop = HOP as f64;
    (1.0 - (-hop / (tau * f64::from(rate))).exp()) as f32
}

impl Gate {
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        reason = "a few bins, and the frequency of a bin"
    )]
    fn of(values: &[f32; PARAMS], rate: u32) -> Gate {
        let value = |param: Param| values[param as usize];
        let (low, high) = DENOISE_SENSITIVITY_FACTOR;
        let beta = low + (high - low) * value(Param::Sensitivity) / 100.0;
        let theta = 10.0_f64.powf(f64::from(value(Param::ThresholdDb)) / 10.0);
        let attack = f64::from(value(Param::AttackMs)) / 1000.0;
        let ratio = f64::from(value(Param::Ratio));
        #[allow(clippy::cast_precision_loss, reason = "8 bins")]
        let widest = DENOISE_FREQUENCY_SMOOTHING_BINS as f32;
        Gate {
            mask: f64::from(beta) * theta,
            signal_step: step(attack / ratio, rate),
            noise_step: step(attack, rate),
            gain_step: step(DENOISE_TIME_SMOOTHING, rate),
            time_smoothing: value(Param::TimeSmoothing) >= 0.5,
            half: (widest * value(Param::FrequencySmoothing) / 100.0).round() as usize,
            band: Band::from_code(value(Param::Band)).unwrap_or_default(),
            bin_hz: rate as f32 / FFT as f32,
        }
    }
}

/// The spectral gate of Noise Reduction, Auto Speech or Manual (see the module): a [`Node`] on
/// any layout but [`Layout::Empty`], the same out as in, no sidechain, with a latency of
/// [`LATENCY`] at every rate.
pub struct Spectral {
    /// The parameters as set, clamped (the profile and the mode beside).
    set: Settings,
    /// The noise learned, as ln of the power of each of the [`BINS`], for Manual.
    learned: Option<Vec<f32>>,
    rate: u32,
    channels: usize,
    stft: Stft,
    tracks: Vec<Track>,
    power: Vec<f32>,
    smoothed: Vec<f32>,
    dry: Planar,
    mixer: Mixer,
}

impl std::fmt::Debug for Spectral {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Spectral")
            .field("set", &self.set)
            .field("rate", &self.rate)
            .field("channels", &self.channels)
            .finish_non_exhaustive()
    }
}

/// The learned power `profile`, of any length, spread over the [`BINS`] by the frequency of its
/// bins (linearly between them), as ln of a power within the floor and the ceiling. `None` when
/// it is empty.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    reason = "places in spectra of at most a few thousand bins"
)]
fn learned(profile: Option<&[f32]>) -> Option<Vec<f32>> {
    let profile = profile.filter(|profile| !profile.is_empty())?;
    let last = (profile.len() - 1) as f64;
    let power = |index: usize| {
        let value = profile[index];
        if value.is_finite() {
            value.max(0.0)
        } else {
            0.0
        }
    };
    Some(
        (0..BINS)
            .map(|bin| {
                let at = bin as f64 * last / (BINS - 1) as f64;
                let below = at.floor() as usize;
                let above = (below + 1).min(profile.len() - 1);
                let t = (at - below as f64) as f32;
                let value = power(below) + t * (power(above) - power(below));
                value.clamp(DENOISE_POWER_FLOOR, DENOISE_POWER_CEILING).ln()
            })
            .collect(), // rt-ok: made when the node is built
    )
}

impl Spectral {
    /// The gate set as `settings` (its mode taken as Auto Speech unless it is Manual).
    #[must_use]
    pub fn new(settings: &Settings) -> Self {
        let set = settings.clamped();
        let learned = if set.mode == Mode::Manual {
            learned(set.profile.as_deref())
        } else {
            None
        };
        Spectral {
            mixer: Mixer::new(&set),
            set,
            learned,
            rate: 0,
            channels: 0,
            stft: Stft::new(0),
            tracks: Vec::new(),        // rt-ok: empty, before the graph is built
            power: vec![0.0; BINS],    // rt-ok: when the node is built
            smoothed: vec![0.0; BINS], // rt-ok: when the node is built
            dry: Planar::new(0, 0),
        }
    }

    /// The parameters as set (their targets).
    #[must_use]
    pub fn settings(&self) -> &Settings {
        &self.set
    }

    /// Whether it follows the noise (Auto Speech, or Manual with nothing learned).
    #[must_use]
    pub fn follows(&self) -> bool {
        self.learned.is_none()
    }

    /// The settling ([`Node::settle`]) of the gate set as `settings` at `rate`: two frames (the
    /// input heard whole, then the overlap emptied), the ramp of 10 ms, and
    /// [`DENOISE_SETTLE_TIME_CONSTANTS`] of the time constants of the gain and, unless a profile
    /// is learned in Manual, of the signal profile (Attack / Ratio) and of the noise profile
    /// (Attack), one after the other. Every switch counts as on. The graph (T4b) asks it for the
    /// slowest settings that the lanes take in the pre-roll (the longest Attack, the lowest
    /// Ratio).
    #[must_use]
    pub fn settle_for(settings: &Settings, rate: u32) -> u32 {
        let learns = settings.mode == Mode::Manual
            && settings
                .profile
                .as_ref()
                .is_some_and(|profile| !profile.is_empty());
        settle_of(!learns, settings.attack_ms, settings.ratio, rate)
    }
}

/// The settling of the gate (see [`Spectral::settle_for`]), whether it `follows` the noise, of
/// `attack` ms and `ratio` (clamped; NaN as the default). Takes no memory.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "a count of samples"
)]
fn settle_of(follows: bool, attack: f32, ratio: f32, rate: u32) -> u32 {
    let number = |param: Param, value: f32| {
        f64::from(if value.is_nan() {
            Settings::default().value(param)
        } else {
            param.clamp(value)
        })
    };
    let attack = number(Param::AttackMs, attack) / 1000.0;
    let ratio = number(Param::Ratio, ratio);
    let taus = DENOISE_TIME_SMOOTHING
        + if follows {
            attack + attack / ratio
        } else {
            0.0
        };
    let rate = f64::from(rate);
    let total =
        (2 * FFT) as f64 + (SMOOTHING * rate).round() + DENOISE_SETTLE_TIME_CONSTANTS * taus * rate;
    total.ceil().min(f64::from(u32::MAX)) as u32
}

/// Gates one frame of one channel (see the module, "The gate").
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    reason = "powers in f32 and the frequencies of bins"
)]
fn gate(
    spectrum: &mut [Complex32],
    track: &mut Track,
    gate: &Gate,
    learned: Option<&[f32]>,
    complete: bool,
    power: &mut [f32],
    smoothed: &mut [f32],
) {
    if !complete {
        return;
    }
    for (power, bin) in power.iter_mut().zip(spectrum.iter()) {
        *power = bin.norm_sqr();
    }
    let mut sum = 0.0_f64;
    let mut low = 0;
    let mut high = 0;
    for (k, smoothed) in smoothed.iter_mut().enumerate() {
        let (from, to) = (k.saturating_sub(gate.half), (k + gate.half).min(BINS - 1));
        while high <= to {
            sum += f64::from(power[high]);
            high += 1;
        }
        while low < from {
            sum -= f64::from(power[low]);
            low += 1;
        }
        *smoothed = (sum.max(0.0) / (high - low) as f64) as f32;
    }
    let floor = crate::db_to_gain(DENOISE_FLOOR_DB);
    for k in 0..BINS {
        let q = smoothed[k].clamp(DENOISE_POWER_FLOOR, DENOISE_POWER_CEILING);
        let noise = if let Some(learned) = learned {
            learned[k]
        } else {
            let s = q.ln();
            if track.started {
                let signal = track.signal[k] + gate.signal_step * (s - track.signal[k]);
                let noise = track.noise[k];
                track.signal[k] = signal;
                track.noise[k] = if signal < noise {
                    signal
                } else {
                    noise + gate.noise_step * (signal - noise)
                };
            } else {
                track.signal[k] = s;
                track.noise[k] = s;
            }
            track.noise[k]
        };
        let g = if gate.band.gates(k as f32 * gate.bin_hz) {
            let r = f64::from(q) / (gate.mask * f64::from(noise).exp());
            let open = 1.0 - 1.0 / (1.0 + r * r);
            (f64::from(floor) + (1.0 - f64::from(floor)) * open) as f32
        } else {
            1.0
        };
        let gain = if track.started && gate.time_smoothing {
            track.gain[k] + gate.gain_step * (g - track.gain[k])
        } else {
            g
        };
        track.gain[k] = gain;
        spectrum[k] *= gain;
    }
    track.started = true;
}

impl Node for Spectral {
    fn prepare(&mut self, spec: &Spec) -> Result<(), Unsupported> {
        if spec.layout == Layout::Empty
            || spec.output != spec.layout
            || spec.sidechain != 0
            || spec.rate == 0
        {
            return Err(Unsupported::from(spec));
        }
        self.rate = spec.rate;
        self.channels = spec.inputs();
        self.stft = Stft::new(self.channels);
        self.tracks = (0..self.channels).map(|_| Track::new()).collect(); // rt-ok: when the graph is built
        self.dry = Planar::new(self.channels, spec.max_block);
        self.mixer.prepare(spec.rate);
        self.reset();
        Ok(())
    }

    fn process(&mut self, ctx: &Block, input: &[&[f32]], output: &mut [&mut [f32]]) {
        let frames = ctx.frames;
        if frames == 0 {
            return;
        }
        let lanes: [Option<&[f32]>; PARAMS] = std::array::from_fn(|index| {
            let param = Param::ALL[index];
            if param.is_automatable() && param.is_spectral() {
                ctx.automation.get(param.id())
            } else {
                None
            }
        });
        let mut current: Option<(usize, Gate)> = None;
        let rate = self.rate;
        let Spectral {
            set,
            learned,
            stft,
            tracks,
            power,
            smoothed,
            dry,
            mixer,
            ..
        } = self;
        let values = |n: usize| -> [f32; PARAMS] {
            std::array::from_fn(|index| {
                let param = Param::ALL[index];
                lane_at(lanes[index], n).map_or(set.value(param), |value| param.clamp(value))
            })
        };
        dry.write(frames, |dry| {
            stft.process(frames, input, output, dry, |frame, spectrum| {
                let gate_now = match current {
                    Some((at, gate_now)) if at == frame.at => gate_now,
                    _ => {
                        let gate_now = Gate::of(&values(frame.at), rate);
                        current = Some((frame.at, gate_now));
                        gate_now
                    }
                };
                gate(
                    spectrum,
                    &mut tracks[frame.channel],
                    &gate_now,
                    learned.as_deref(),
                    frame.complete,
                    power,
                    smoothed,
                );
            });
        });
        dry.read(frames, |dry| {
            mixer.mix(frames, &ctx.automation, dry, output);
        });
    }

    fn latency(&self) -> u32 {
        #[allow(clippy::cast_possible_truncation, reason = "1536")]
        let latency = LATENCY as u32;
        latency
    }

    /// [`Spectral::settle_for`] the parameters as set.
    fn settle(&self) -> u32 {
        settle_of(
            self.follows(),
            self.set.attack_ms,
            self.set.ratio,
            self.rate,
        )
    }

    fn reset(&mut self) {
        self.stft.reset();
        for track in &mut self.tracks {
            track.reset();
        }
        self.mixer.finish();
    }

    /// Lays the frames on the timeline: a frame ends where `origin` plus the samples processed,
    /// plus one, is a multiple of [`HOP`].
    fn align(&mut self, origin: u64) {
        self.stft.align(origin);
    }

    fn set(&mut self, param: ParamId, value: f32) {
        let Some(param) = Param::from_id(param) else {
            return;
        };
        if value.is_nan() {
            return;
        }
        self.set.set(param, value);
        self.mixer.set(param, param.clamp(value));
    }
}

// --- Either mode -----------------------------------------------------------------------------

/// Noise Reduction in the mode of its settings: the spectral gate ([`Spectral`]) or the voice
/// ([`Voice`]). The mode is fixed from [`NoiseReduction::new`] on: a change of it builds a new
/// node (see the module).
#[derive(Debug)]
pub enum NoiseReduction {
    /// Auto Speech or Manual.
    Spectral(Box<Spectral>),
    /// The voice, by `RNNoise`.
    Voice(Box<Voice>),
}

impl NoiseReduction {
    /// Noise Reduction set as `settings`, in its mode.
    #[must_use]
    pub fn new(settings: &Settings) -> Self {
        match settings.mode {
            Mode::Voice => NoiseReduction::Voice(Box::new(Voice::new(settings))), // rt-ok: when the graph is built
            Mode::AutoSpeech | Mode::Manual => {
                NoiseReduction::Spectral(Box::new(Spectral::new(settings))) // rt-ok: when the graph is built
            }
        }
    }

    /// The settling of Noise Reduction set as `settings` at `rate`, in its mode.
    #[must_use]
    pub fn settle_for(settings: &Settings, rate: u32) -> u32 {
        match settings.mode {
            Mode::Voice => Voice::settle_for(settings, rate),
            Mode::AutoSpeech | Mode::Manual => Spectral::settle_for(settings, rate),
        }
    }

    fn node(&mut self) -> &mut dyn Node {
        match self {
            NoiseReduction::Spectral(node) => node.as_mut(),
            NoiseReduction::Voice(node) => node.as_mut(),
        }
    }

    fn node_ref(&self) -> &dyn Node {
        match self {
            NoiseReduction::Spectral(node) => node.as_ref(),
            NoiseReduction::Voice(node) => node.as_ref(),
        }
    }
}

impl Node for NoiseReduction {
    fn prepare(&mut self, spec: &Spec) -> Result<(), Unsupported> {
        self.node().prepare(spec)
    }

    fn process(&mut self, ctx: &Block, input: &[&[f32]], output: &mut [&mut [f32]]) {
        self.node().process(ctx, input, output);
    }

    fn latency(&self) -> u32 {
        self.node_ref().latency()
    }

    fn settle(&self) -> u32 {
        self.node_ref().settle()
    }

    fn reset(&mut self) {
        self.node().reset();
    }

    fn align(&mut self, origin: u64) {
        self.node().align(origin);
    }

    fn set(&mut self, param: ParamId, value: f32) {
        self.node().set(param, value);
    }
}
