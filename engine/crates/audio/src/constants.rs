//! Every number of the sound that the manual of `DaVinci Resolve` 20 does not give and the plan
//! of phase 3 decides, in one place, so that the check against the real program at the end of the
//! phase (T26) changes them here alone.
//!
//! Where the plan (`2026-11-03-studio3.md`, "本计划替作者定的事" and the tasks) and the interaction
//! reference (§15, whose numbers C-… are cited) differ, the plan holds. Times are in seconds and
//! become samples by [`samples`] at the rate of the timeline. The ranges and defaults of the
//! parameters of each effect are with the effect's parameters in the model (`fx.rs` of
//! `columbina-model`), not here: these are the numbers of how the sound is made.

/// The samples in `seconds` at `rate`, to the nearest.
#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the times here are short and not negative"
)]
pub fn samples(seconds: f64, rate: u32) -> usize {
    (seconds * f64::from(rate)).round() as usize
}

// --- Channels and buses ----------------------------------------------------------------------

/// The most channels of anything in the graph: an Adaptive track has 1 to 24 (p. 3724), and the
/// widest bus, 7.1, has 8.
pub const MAX_CHANNELS: usize = 24;

/// The deepest nesting of buses into buses (p. 3721; C-B2).
pub const MAX_BUS_DEPTH: usize = 6;

/// The most buses that a track or a bus outputs to, and the most sends besides (p. 3721; C-B1).
pub const MAX_OUTPUTS: usize = 10;

/// The most sends of a track or a bus (C-B1).
pub const MAX_SENDS: usize = 10;

/// The insert slots of a track or a bus (p. 3855; C-X12).
pub const MAX_INSERTS: usize = 6;

/// The most buses of a timeline: the manual has "up to the limitations of your system" (p. 3730;
/// C-B3).
pub const MAX_BUSES: usize = 64;

/// The most VCAs of a timeline: the manual has both 64 and 128, and the smaller is taken (p. 3687,
/// p. 3883; C-B5).
pub const MAX_VCAS: usize = 64;

// --- Faders, pan and the laws of the channels ------------------------------------------------

/// The top of a fader, a send and a VCA, in dB (C-F1).
pub const FADER_MAX_DB: f32 = 10.0;

/// A level in dB at or below which the sound is silent (−∞): the fader's bottom in the model.
pub const SILENCE_DB: f32 = -100.0;

/// The gain into each side of a stereo bus of a mono source panned to the centre: −3.01 dB, the
/// law of equal power, as libswresample makes stereo of mono (C-F5, C-F6).
pub const MONO_TO_STEREO_GAIN: f32 = std::f32::consts::FRAC_1_SQRT_2;

/// The gain of the near side of a stereo source panned all the way: +3 dB, the balance that the
/// engine has had since phase 1 (`mix.rs::pan`), so that the sound of old projects is the same
/// (C-F5).
pub const BALANCE_EDGE_GAIN: f32 = std::f32::consts::SQRT_2;

/// The gain of the centre into each side when 5.1 is mixed down to stereo (ITU-R BS.775; C-F7).
pub const DOWNMIX_CENTRE_GAIN: f32 = std::f32::consts::FRAC_1_SQRT_2;

/// The gain of each surround into its side when 5.1 is mixed down to stereo (ITU-R BS.775; C-F7).
pub const DOWNMIX_SURROUND_GAIN: f32 = std::f32::consts::FRAC_1_SQRT_2;

/// The gain of the LFE when 5.1 is mixed down to stereo: dropped (ITU-R BS.775; C-F7).
pub const DOWNMIX_LFE_GAIN: f32 = 0.0;

/// The gain of each side when two channels are made one by their sum (C-F16).
pub const MONO_SUM_GAIN: f32 = 0.5;

// --- The thread of the mix -------------------------------------------------------------------

/// The ramp of a parameter changed while the sound plays: linear, the gain in the linear domain
/// and the frequency in the logarithmic one (plan; C-T1).
pub const SMOOTHING: f64 = 0.010;

/// The crossfade between the old graph and the new one when the structure changes, at the edge of
/// a block (plan T5; the reference's C-T3 has 5 ms).
pub const GRAPH_SWAP_CROSSFADE: f64 = 0.010;

/// The sound the mix thread keeps ahead of the card (plan; C-T2).
pub const OUTPUT_AHEAD: f64 = 0.030;

/// The fewest periods of the device the mix thread keeps ahead of the card (plan).
pub const OUTPUT_AHEAD_PERIODS: usize = 2;

/// How far ahead of the playhead a clip's reader decodes (plan T5).
pub const PREFETCH_AHEAD: f64 = 0.5;

/// The sound a clip's reader keeps on either side of the playhead (plan T5, as in phase 2).
pub const PREFETCH_WINDOW: f64 = 0.25;

/// The level under which two outputs are taken as the same: a node has settled when its output
/// after a reset differs from the output of the same node started earlier by less than this
/// (plan T4a, `Node::settle`; and the zero test from the middle, T6).
pub const SETTLE_FLOOR_DBFS: f32 = -90.0;

// --- Change of speed -------------------------------------------------------------------------

/// The time of the source between two points where the stretch that keeps the pitch starts again
/// (plan T13).
pub const STRETCH_RESET_INTERVAL: f64 = 4.0;

/// The crossfade on either side of a point where the stretch starts again (plan T13).
pub const STRETCH_RESET_CROSSFADE: f64 = 0.010;

/// The sound of each move of the playhead while scrubbing (C-U7).
pub const SCRUB_GRAIN: f64 = 0.060;

// --- Meters and loudness ---------------------------------------------------------------------

/// The integration of the PPM, IEC 60268-18 (plan).
pub const PPM_INTEGRATION: f64 = 0.005;

/// The fall of the PPM: this many dB ...
pub const PPM_FALL_DB: f32 = 20.0;

/// ... in this many seconds (plan; C-L2).
pub const PPM_FALL_TIME: f64 = 1.7;

/// The rise and the fall of the VU (plan).
pub const VU_TIME: f64 = 0.300;

/// The fall of the peak segment of the Digital VU, per 20 dB (C-L2).
pub const DIGITAL_VU_PEAK_FALL_TIME: f64 = 0.5;

/// How long a peak is held: Medium, the default (plan; C-L4) ...
pub const PEAK_HOLD: f64 = 2.0;

/// ... Short ...
pub const PEAK_HOLD_SHORT: f64 = 1.0;

/// ... and Long (C-L4).
pub const PEAK_HOLD_LONG: f64 = 5.0;

/// The window of the RMS of the meters (C-L3).
pub const RMS_WINDOW: f64 = 0.300;

/// How often the meters are read and sent (C-L5).
pub const METER_RATE_HZ: u32 = 30;

/// The oversampling of the true peak (plan T9; C-L1).
pub const TRUE_PEAK_OVERSAMPLING: usize = 4;

/// The window of Momentary loudness, and how often it is given (BS.1770; p. 3985).
pub const LOUDNESS_MOMENTARY: f64 = 0.400;

/// How often Momentary loudness is given (p. 3985; C-R1).
pub const LOUDNESS_MOMENTARY_STEP: f64 = 0.100;

/// The window of Short Term loudness: the manual has 30 s and 3 s, EBU has 3 s (C-R2).
pub const LOUDNESS_SHORT_TERM: f64 = 3.0;

/// The absolute gate of integrated loudness and of the loudness range, in LUFS (BS.1770; C-R5).
pub const LOUDNESS_ABSOLUTE_GATE: f32 = -70.0;

/// The gate relative to the ungated loudness, for the integrated loudness, in LU (C-R5).
pub const LOUDNESS_RELATIVE_GATE: f32 = -10.0;

/// The gate relative to the ungated loudness, for the loudness range, in LU (Tech 3342; C-R5).
pub const LOUDNESS_RANGE_GATE: f32 = -20.0;

/// The low and the high percentile of the short-term loudness that make the loudness range
/// (Tech 3342; C-R5).
pub const LOUDNESS_RANGE_PERCENTILES: (f32, f32) = (0.10, 0.95);

/// The cut of the monitor by DIM, in dB (C-M1).
pub const DIM_DB: f32 = -20.0;

// --- Dynamics and EQ -------------------------------------------------------------------------

/// The part of a step that the gain of Dynamics has gone by its attack or release time: the time
/// constant of a one-pole smoother, 1 − 1/e (plan T8).
pub const DYNAMICS_TIME_CONSTANT_FRACTION: f64 = 1.0 - 1.0 / std::f64::consts::E;

/// The level, in dBFS, that Dynamics hears in silence: the floor of its detector (T8).
pub const DYNAMICS_LEVEL_FLOOR_DB: f64 = -200.0;

/// The loudest level, in dBFS, that Dynamics hears: an infinite sample is heard as this (T8).
pub const DYNAMICS_LEVEL_CEILING_DB: f64 = 200.0;

/// The samples of a bin of the Hold of Dynamics: the window of a hold of H samples is H + 1 to
/// H + 32 samples long, kept as the peaks of its bins (T8). 32 rather than 64, for half the
/// spread of the length of the window (0.67 ms at 48 kHz) at 48 kB a window for 4 s.
pub const DYNAMICS_HOLD_BIN: u32 = 32;

/// How fast the peak that a module of Dynamics hears when it is turned on falls, in dB a second:
/// it bridges the dips of a wave (4 dB in the 10 ms of half a wave of 50 Hz) and forgets the
/// 400 dB between the floor and the ceiling in a second (T8).
pub const DYNAMICS_WAKE_FALL_DB_PER_SECOND: f64 = 400.0;

/// How many of its time constants (attack plus release, and its hold) a module of Dynamics
/// takes to settle (`Node::settle`, T8): a difference of the gain of 200 dB, at an output up to
/// +12 dBFS, comes under −90 dBFS in ln(200 / 6.9e-5) ≈ 15 time constants, and 3 more cover the
/// two stages of its detector (the hold of the peak, then the attack).
pub const DYNAMICS_SETTLE_TIME_CONSTANTS: f64 = 18.0;

/// The zones of the frequency of the bands 2 to 5 of the track EQ, in Hz: L, ML, MH, H (plan; the
/// reference's C-E3 has wider ones).
pub const EQ_ZONES: [(f32, f32); 4] = [
    (20.0, 200.0),
    (100.0, 1_000.0),
    (500.0, 5_000.0),
    (2_000.0, 20_000.0),
];

/// The slope of a shelf of the EQ, the S of the RBJ cookbook (C-E4).
pub const EQ_SHELF_SLOPE: f32 = 1.0;

/// The crossfade between the old and the new structure of a band of the EQ when what is not a
/// number on a line changes: a band or the EQ switched, a type, a slope (plan T7). The
/// frequency, the gain and the Q ramp over [`SMOOTHING`] instead, the filters designed again
/// at each sample (the review of T7).
pub const EQ_CROSSFADE: f64 = 0.010;

/// The highest frequency of a band of the EQ, as a part of the rate: 20 kHz, or less where the
/// rate is below 44.4 kHz (the reference, §9.2).
pub const EQ_MAX_FREQUENCY_FRACTION: f64 = 0.45;

// --- Effects ---------------------------------------------------------------------------------

/// The window of the RMS by which the Ducker hears its sources (plan; C-X2).
pub const DUCKER_RMS_WINDOW: f64 = 0.010;

/// The level of the sources above which the Ducker ducks, in dBFS (plan; C-X2).
pub const DUCKER_THRESHOLD_DBFS: f32 = -45.0;

/// The hysteresis of the Ducker between ducking and not, in dB (C-X2).
pub const DUCKER_HYSTERESIS_DB: f32 = 3.0;

/// The lookahead of the Limiter, in samples (the manual, p. 3964; C-X5).
pub const LIMITER_LOOKAHEAD: usize = 64;

/// The points of the transform of Noise Reduction (plan T12; C-X4).
pub const DENOISE_FFT: usize = 2048;

/// The hop of Noise Reduction: an overlap of 75 % (plan T12; C-X4).
pub const DENOISE_HOP: usize = 512;

/// The delay of Noise Reduction, in samples (plan T12; the reference's C-X4 has 2048).
pub const DENOISE_LATENCY: usize = DENOISE_FFT - DENOISE_HOP;

/// The frame of the voice mode of Noise Reduction (`RNNoise`, plan T12), at 48 kHz.
pub const RNNOISE_FRAME: usize = 480;

/// The highest frequency that De-Rumble works on, in Hz (C-X4).
pub const DENOISE_RUMBLE_HZ: f32 = 250.0;

/// The lowest frequency that De-Hiss works on, in Hz (C-X4).
pub const DENOISE_HISS_HZ: f32 = 4_000.0;

/// The attack and release of De-Esser's Reaction, Slow, Medium and Fast, in seconds (C-X7).
pub const DEESSER_REACTION: [(f64, f64); 3] = [(0.010, 0.080), (0.002, 0.040), (0.0005, 0.020)];

/// The notches of De-Hummer: the fundamental and its harmonics (C-X8) ...
pub const DEHUMMER_NOTCHES: usize = 4;

/// ... each of this Q (C-X8).
pub const DEHUMMER_Q: f32 = 30.0;

/// The point at which Soft Clipper clips, in dBFS (C-X10).
pub const SOFT_CLIP_DBFS: f32 = 0.0;

// --- Automation ------------------------------------------------------------------------------

/// The time of Snap's return, Glide Time (the manual, p. 3905; C-A1).
pub const AUTOMATION_GLIDE: f64 = 0.250;

/// The ramp of Return when the transport stops (C-A2).
pub const AUTOMATION_RETURN: f64 = 0.250;

/// The time after the last step of the mouse wheel at which Touch lets a control go (C-A3).
pub const AUTOMATION_WHEEL_RELEASE: f64 = 0.500;

/// The error by which the points drawn or recorded on a fader's lane are thinned, in dB (plan
/// T14; C-A4).
pub const AUTOMATION_FADER_TOLERANCE_DB: f32 = 0.1;

/// The error of the thinning of a pan's lane, as a part of its whole range (C-A4).
pub const AUTOMATION_PAN_TOLERANCE: f32 = 0.005;

/// The error of the thinning of the other lanes, as a part of the range shown (C-A4).
pub const AUTOMATION_OTHER_TOLERANCE: f32 = 0.001;

/// The most points recorded in a second (C-A4).
pub const AUTOMATION_MAX_POINTS_PER_SECOND: u32 = 100;

/// The least time between two points recorded (C-A4).
pub const AUTOMATION_MIN_SPACING: f64 = 0.001;

/// Where a range of a lane raised or lowered puts its inner points, inside its ends (C-A6).
pub const AUTOMATION_BOUNDARY_RAMP: f64 = 0.010;

// --- The curves of fades and crossfades (convention 11) --------------------------------------

/// The level of one side of a crossfade (p. 3831; `fade_in_level_db` and `fade_out_level_db`
/// of a transition, T1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CrossfadeLevel {
    /// −3 dB: g(t) = t^1.5, 3.01 dB under linear half way (C-C2; chosen over the plan's
    /// 1 − cos(πt/2), which is 4.64 dB under, by the review of T4a).
    Minus3,
    /// 0 dB: g(t) = t, linear; the default of a new crossfade (Q11).
    Zero,
    /// +3 dB: g(t) = sin(πt/2), equal power, 3.01 dB over linear half way; the crossfades of
    /// projects of schema 3, to the bit as phase 2 made them (`model/src/resolve.rs`).
    Plus3,
}

impl CrossfadeLevel {
    /// The level of `db`: −3, 0 or 3.
    #[must_use]
    pub fn from_db(db: i32) -> Option<Self> {
        match db {
            -3 => Some(CrossfadeLevel::Minus3),
            0 => Some(CrossfadeLevel::Zero),
            3 => Some(CrossfadeLevel::Plus3),
            _ => None,
        }
    }
}

/// The gain of one side of a crossfade of `level`, `t` of the way through it (0 to 1, clamped):
/// g(t) for the side that comes in, g(1 − t) for the side that goes (`falling`). The equal-power
/// side that goes is cos(πt/2), which is g(1 − t) and is what phase 2 computes, to the bit.
#[must_use]
pub fn crossfade_gain(level: CrossfadeLevel, t: f64, falling: bool) -> f64 {
    use std::f64::consts::FRAC_PI_2;
    let t = t.clamp(0.0, 1.0);
    match (level, falling) {
        (CrossfadeLevel::Zero, false) => t,
        (CrossfadeLevel::Zero, true) => 1.0 - t,
        (CrossfadeLevel::Plus3, false) => (t * FRAC_PI_2).sin(),
        (CrossfadeLevel::Plus3, true) => (t * FRAC_PI_2).cos(),
        (CrossfadeLevel::Minus3, false) => t.powf(1.5),
        (CrossfadeLevel::Minus3, true) => (1.0 - t).powf(1.5),
    }
}

/// The gain of a clip's fade, `t` of the way through it (0 to 1, clamped): g(t) for a fade in,
/// g(1 − t) for a fade out (`falling`). `curve` from −1 to 1 bends it: 0 is linear, +1 is
/// sin(πt/2), −1 is 1 − cos(πt/2), and between them the curve is the linear one mixed with the
/// one at the end by |`curve`|. `skew` from −1 to 1 moves its middle in time: t' = t^(2^(−skew))
/// (the plan's table; dragging the handle up and down and sideways, pp. 972–973).
#[must_use]
pub fn fade_gain(curve: f64, skew: f64, t: f64, falling: bool) -> f64 {
    use std::f64::consts::FRAC_PI_2;
    let t = t.clamp(0.0, 1.0);
    let t = if falling { 1.0 - t } else { t };
    let curve = curve.clamp(-1.0, 1.0);
    let skew = skew.clamp(-1.0, 1.0);
    let t = if skew == 0.0 {
        t
    } else {
        t.powf((-skew).exp2())
    };
    let end = if curve >= 0.0 {
        (t * FRAC_PI_2).sin()
    } else {
        1.0 - (t * FRAC_PI_2).cos()
    };
    t + curve.abs() * (end - t)
}

// --- Noise Reduction (T12) -------------------------------------------------------------------

/// The most that the spectral gate of Noise Reduction lowers a bin, in dB: the floor of its gain,
/// which leaves a little of the noise under the voice instead of the "musical" tones of bins let
/// through alone (C-X4 has no depth; we decide).
pub const DENOISE_FLOOR_DB: f32 = -30.0;

/// The power of a bin of Noise Reduction below which it is heard as this: so that the profiles
/// in their logarithms stay finite after silence, and climb back from silence in a bounded time
/// (about −149 dBFS of white noise in a bin of the 2048-point transform; we decide).
pub const DENOISE_POWER_FLOOR: f32 = 1e-12;

/// The power of a bin above which it is heard as this (we decide).
pub const DENOISE_POWER_CEILING: f32 = 1e12;

/// The largest size of a sample that Noise Reduction analyses: a larger one is analysed as this
/// (the dry sound is let through as it is), so that the power of a bin stays finite (we decide).
pub const DENOISE_INPUT_LIMIT: f32 = 1e4;

/// The time constant of the gains of Noise Reduction over time with Time Smoothing on, in
/// seconds (p. 3970 gives no number; we decide).
pub const DENOISE_TIME_SMOOTHING: f64 = 0.020;

/// The bins on each side over which Frequency Smoothing at 100 % averages the power that Noise
/// Reduction weighs (p. 3970 gives no number; we decide: ±188 Hz at 48 kHz).
pub const DENOISE_FREQUENCY_SMOOTHING_BINS: usize = 8;

/// The factor by which Sensitivity at 0 % and at 100 % overestimates the noise (C-X4).
pub const DENOISE_SENSITIVITY_FACTOR: (f32, f32) = (1.0, 4.0);

/// The time constants of its profiles after which Noise Reduction counts itself settled:
/// ⌈ln(55 / 3.2e-5)⌉ = 15, the logarithm of a power spanning 1e-12 to 1e12 (55) brought below
/// −90 dBFS (we decide).
pub const DENOISE_SETTLE_TIME_CONSTANTS: f64 = 15.0;

/// The rate at which `RNNoise` works (plan T12).
pub const RNNOISE_RATE: u32 = 48_000;

/// The delay of `RNNoise` itself, at 48 kHz: its window of two frames puts out the frame before
/// the one it takes (measured; the plan's "the first frame is thrown away").
pub const RNNOISE_DELAY: usize = 480;

/// The scale of the samples that `RNNoise` takes: the range of an i16 (plan T12).
pub const RNNOISE_SCALE: f32 = 32_768.0;

/// The settling of the voice mode of Noise Reduction, in seconds: its network forgets slowly.
/// Measured on a voice over noise from five starts: within −60 dBFS of an earlier start after
/// 10 s (the pre-roll), mostly within −90 dBFS after 20 to 35 s, a difference of −94 dBFS as late
/// as 70 s, and below −130 dBFS after 80 s (we decide on 60).
pub const RNNOISE_SETTLE: f64 = 60.0;

/// The fresh states of `RNNoise` that the voice mode keeps for each channel, to be as new after
/// a reset without taking memory on the thread of the mix (we decide).
pub const RNNOISE_SPARE_STATES: usize = 4;

/// The zero crossings on each side of the windowed sinc by which the voice mode takes another
/// rate to 48 kHz and back (as the mixer of phase 2, `server/src/play/mix.rs`).
pub const RNNOISE_RESAMPLE_ZEROS: f64 = 16.0;

/// The band that the resampling of the voice mode keeps, as a part of the lower Nyquist
/// frequency (we decide).
pub const RNNOISE_RESAMPLE_BAND: f64 = 0.95;

/// The longest period, in samples of 48 kHz, of the grid on which another rate meets 48 kHz,
/// 48000 / gcd(rate, 48000), that the voice mode takes: 640 takes every rate of 8 kHz and of the
/// families of 44.1 and 48 kHz from 11.025 kHz up (we decide).
pub const RNNOISE_LONGEST_PERIOD: u64 = 640;

/// The lowest and the highest rate that the voice mode takes (we decide).
pub const RNNOISE_RATES: (u32, u32) = (8_000, 384_000);
