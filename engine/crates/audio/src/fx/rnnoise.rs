//! The voice mode of Noise Reduction (plan T12; the interaction reference §10.5): `RNNoise`
//! (J.-M. Valin, "A Hybrid DSP/Deep Learning Approach to Real-Time Full-Band Speech
//! Enhancement", 2018) by the crate `nnnoiseless` 0.5.2, which keeps the voice and takes the rest
//! away. Of the parameters only Dry/Wet, Level, Listen to Noise Only and On apply.
//!
//! # Frames and delay
//!
//! `RNNoise` takes frames of [`RNNOISE_FRAME`] samples at 48 kHz, in the range of an i16
//! ([`RNNOISE_SCALE`]), and puts out the frame before the one it takes ([`RNNOISE_DELAY`]). The
//! frames are laid on the timeline ([`Node::align`]), and a frame's output comes out while the
//! next frame comes in: at 48 kHz the delay is 960 samples, whatever the blocks. The output of
//! the first frame after a reset, the plan's "first frame thrown away", is the silence before the
//! start.
//!
//! # Other rates
//!
//! At another rate the node takes the sound to 48 kHz and back inside itself ([`Resampler`]): a
//! windowed sinc of [`RNNOISE_RESAMPLE_ZEROS`] zero crossings on each side, its band
//! [`RNNOISE_RESAMPLE_BAND`] of the lower Nyquist frequency, a table of taps for each place
//! between samples, each table summing to 1. The samples of 48 kHz sit on a grid laid on the
//! timeline, the sample `n` at the time `n / 48000` s. The delays of the two conversions are
//! chosen so that the whole delay is a whole count of samples (the way back waits until the
//! delay at 48 kHz is a multiple of the grid's period, 48000 / gcd(rate, 48000)): at 44.1 kHz
//! 17 + (160 + 960) · 147 / 160 = 1046 samples ([`Voice::latency_for`]). Rates whose grid is longer
//! than [`RNNOISE_LONGEST_PERIOD`], or out of [`RNNOISE_RATES`], are refused by `prepare`.
//!
//! # The thread of the mix
//!
//! `nnnoiseless` plans its transforms on first use on each thread and keeps them there (in the
//! crate `easyfft`, leaking one plan for each thread): the first frame on a thread takes memory,
//! none after. The thread of the mix, which lives as long as the playback, calls [`warm_thread`]
//! once before it runs (T5); `prepare` does not, so that the threads that build graphs do not
//! each keep a plan.
//!
//! # Fresh states
//!
//! The state of `RNNoise` has no reset of its own, and copying a fresh state into a used one
//! allocates (`clone_from` takes 9 allocations, measured): so the node keeps
//! [`RNNOISE_SPARE_STATES`] fresh states for each channel, taken in `prepare`, and swaps a used
//! state for a fresh one on a reset and after [`RNNOISE_FORGET`] of digital silence (every
//! sample of 100 frames in a row below −140 dBFS). `RNNoise` keeps its state through silence, so
//! that without the latter the state after a gap would be that of the clip before it, and a
//! playback started inside the gap would not hear what an export hears. When the spares are
//! spent (more resets and gaps than that without a `prepare`), the used state is kept.

use nnnoiseless::DenoiseState;

use crate::constants::{
    samples, DENOISE_INPUT_LIMIT, RNNOISE_DELAY, RNNOISE_FORGET, RNNOISE_FRAME,
    RNNOISE_LONGEST_PERIOD, RNNOISE_RATE, RNNOISE_RATES, RNNOISE_RESAMPLE_BAND,
    RNNOISE_RESAMPLE_ZEROS, RNNOISE_SCALE, RNNOISE_SETTLE, RNNOISE_SPARE_STATES, SMOOTHING,
};
use crate::fx::denoise::{Mixer, Param, Settings};
use crate::node::{Block, Layout, Node, ParamId, Planar, Spec, Unsupported};

/// Plans the transforms of `RNNoise` on the calling thread, so that it takes no memory there
/// after: the thread of the mix calls it once before it runs the graph. Allocates.
pub fn warm_thread() {
    let mut state = DenoiseState::new();
    let input = [RNNOISE_SCALE / 8.0; RNNOISE_FRAME];
    let mut output = [0.0; RNNOISE_FRAME];
    state.process_frame(&mut output, &input);
    state.process_frame(&mut output, &input);
}

// --- Resampling ------------------------------------------------------------------------------

/// The greatest common divisor.
fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

/// A conversion of a stream from one rate to another by a windowed sinc (see the module): the
/// output sample `n` is the input at the place `n·p/q − delay`, `p/q` the ratio of the rates in
/// lowest terms, and every place falls on one of `q` tables of taps.
#[derive(Clone, Debug)]
pub struct Resampler {
    p: u64,
    q: u64,
    radius: usize,
    delay: u64,
    /// `q` tables of `2·radius` taps: the table of `n·p mod q` weighs the input samples
    /// `floor(n·p/q) − delay − radius + 1` on.
    taps: Vec<f32>,
}

impl Resampler {
    /// The taps on each side of the conversion from `from` to `to` samples a second.
    #[must_use]
    pub fn radius(from: u32, to: u32) -> usize {
        let cutoff = Self::cutoff(from, to);
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a few dozen taps"
        )]
        let radius = (RNNOISE_RESAMPLE_ZEROS / cutoff).ceil() as usize;
        radius
    }

    /// The band kept, as a part of the Nyquist frequency of `from`.
    fn cutoff(from: u32, to: u32) -> f64 {
        RNNOISE_RESAMPLE_BAND * (f64::from(to) / f64::from(from)).min(1.0)
    }

    /// The conversion from `from` to `to`, its output `delay` samples of `from` late (at least
    /// [`Resampler::radius`], so that it reads no input from after the place). Allocates.
    ///
    /// # Panics
    ///
    /// When a rate is 0 or the delay is less than the radius.
    #[must_use]
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        reason = "taps in f32, and a few hundred tables"
    )]
    pub fn new(from: u32, to: u32, delay: u64) -> Self {
        assert!(from > 0 && to > 0, "a rate of 0");
        let common = gcd(u64::from(from), u64::from(to));
        let (p, q) = (u64::from(from) / common, u64::from(to) / common);
        let radius = Self::radius(from, to);
        assert!(delay >= radius as u64, "a delay shorter than the radius");
        let cutoff = Self::cutoff(from, to);
        let reach = radius as f64;
        let mut taps = Vec::with_capacity(q as usize * 2 * radius); // rt-ok: when the graph is built
        for phase in 0..q {
            let fraction = phase as f64 / q as f64;
            let start = taps.len();
            for tap in 0..2 * radius {
                // The place of the output less the place of the input under this tap.
                let distance = fraction + reach - 1.0 - tap as f64;
                let angle = std::f64::consts::PI * cutoff * distance;
                let sinc = if angle.abs() < 1e-12 {
                    1.0
                } else {
                    angle.sin() / angle
                };
                let window = if distance.abs() < reach {
                    0.5 + 0.5 * (std::f64::consts::PI * distance / reach).cos()
                } else {
                    0.0
                };
                taps.push(sinc * window);
            }
            let table = &mut taps[start..];
            let sum: f64 = table.iter().sum();
            for tap in table {
                *tap /= sum;
            }
        }
        Resampler {
            p,
            q,
            radius,
            delay,
            taps: taps.into_iter().map(|tap| tap as f32).collect(), // rt-ok: when the graph is built
        }
    }

    /// The input samples to the output's one: `p/q` in lowest terms.
    #[must_use]
    pub fn ratio(&self) -> (u64, u64) {
        (self.p, self.q)
    }

    /// The delay, in samples of the input.
    #[must_use]
    pub fn delay(&self) -> u64 {
        self.delay
    }

    /// The taps on each side.
    #[must_use]
    pub fn reach(&self) -> usize {
        self.radius
    }

    /// The output samples whose last input sample is `input`: from ⌈input·q/p⌉ to before
    /// ⌈(input + 1)·q/p⌉.
    #[must_use]
    pub fn outputs_at(&self, input: u64) -> std::ops::Range<u64> {
        (input * self.q).div_ceil(self.p)..((input + 1) * self.q).div_ceil(self.p)
    }

    /// The output sample `n`, of the input in `history`.
    #[must_use]
    #[allow(clippy::cast_possible_truncation, reason = "a phase below q")]
    pub fn output(&self, n: u64, history: &History) -> f32 {
        let place = n * self.p;
        let phase = (place % self.q) as usize;
        let first = (place / self.q).cast_signed()
            - self.delay.cast_signed()
            - (self.radius as u64).cast_signed()
            + 1;
        let taps = &self.taps[phase * 2 * self.radius..(phase + 1) * 2 * self.radius];
        let mut sum = 0.0_f32;
        for (offset, tap) in (0_i64..).zip(taps) {
            sum += tap * history.get(first + offset);
        }
        sum
    }
}

/// The last samples of a stream, by their place in it from the start of the timeline.
#[derive(Clone, Debug, Default)]
pub struct History {
    ring: Vec<f32>,
}

impl History {
    /// A history of at least `length` samples, silent. Allocates.
    #[must_use]
    pub fn new(length: usize) -> Self {
        History {
            ring: vec![0.0; length.next_power_of_two()], // rt-ok: when the graph is built
        }
    }

    /// Silence.
    pub fn clear(&mut self) {
        self.ring.fill(0.0);
    }

    /// Puts the sample of the place `at`.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "a place modulo the length of the ring"
    )]
    pub fn put(&mut self, at: u64, sample: f32) {
        let mask = self.ring.len() as u64 - 1;
        self.ring[(at & mask) as usize] = sample;
    }

    /// The sample of the place `at`: silence before the start.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a place modulo the length of the ring"
    )]
    #[must_use]
    pub fn get(&self, at: i64) -> f32 {
        if at < 0 {
            0.0
        } else {
            let mask = self.ring.len() as u64 - 1;
            self.ring[(at as u64 & mask) as usize]
        }
    }
}

// --- The node --------------------------------------------------------------------------------

/// How the rate of the timeline meets 48 kHz: not at all, or by two conversions.
#[derive(Debug)]
struct Conversion {
    up: Resampler,
    down: Resampler,
}

/// How the voice meets `rate`: its delay, and the delays of the way up and of the way back
/// (none at 48 kHz). Worked out without taking memory. `None` when the rate is refused.
#[allow(clippy::cast_possible_truncation, reason = "a few dozen taps")]
fn plan(rate: u32) -> Option<(u64, Option<(u64, u64)>)> {
    let (low, high) = RNNOISE_RATES;
    if !(low..=high).contains(&rate) {
        return None;
    }
    let inner = 2 * RNNOISE_DELAY as u64;
    if rate == RNNOISE_RATE {
        return Some((inner, None));
    }
    let g = gcd(u64::from(rate), u64::from(RNNOISE_RATE));
    let (p, q) = (u64::from(rate) / g, u64::from(RNNOISE_RATE) / g);
    if q > RNNOISE_LONGEST_PERIOD {
        return None;
    }
    let up = Resampler::radius(rate, RNNOISE_RATE) as u64;
    let radius = Resampler::radius(RNNOISE_RATE, rate) as u64;
    // The delay back, at 48 kHz: at least the radius, and with the delay of RNNoise a multiple
    // of q, so that the whole delay is a whole count of samples of the rate.
    let back = (radius + inner).div_ceil(q) * q - inner;
    Some((up + (back + inner) * p / q, Some((up, back))))
}

/// The two conversions at `rate`, by its [`plan`]. Allocates.
fn conversion(rate: u32, delays: Option<(u64, u64)>) -> Option<Conversion> {
    delays.map(|(up, back)| Conversion {
        up: Resampler::new(rate, RNNOISE_RATE, up),
        down: Resampler::new(RNNOISE_RATE, rate, back),
    })
}

/// `RNNoise` on one channel, at 48 kHz.
struct Channel {
    state: Box<DenoiseState<'static>>,
    /// The state has taken a frame since it was fresh.
    used: bool,
    /// The frames of digital silence in a row.
    quiet: u32,
    /// The frame coming in, scaled.
    frame: [f32; RNNOISE_FRAME],
    /// The frame going out: the output of the last frame taken.
    out: [f32; RNNOISE_FRAME],
    /// The input at the rate of the timeline, for the way to 48 kHz.
    input: History,
    /// The output at 48 kHz, for the way back.
    voice: History,
    /// The dry sound, for its delay.
    dry: History,
}

/// The fresh states of `RNNoise`, and the used ones taken out of the channels, kept to be freed
/// off the thread of the mix (on the next `prepare`, or when the node is dropped).
struct Pool {
    spares: Vec<Box<DenoiseState<'static>>>,
    spent: Vec<Box<DenoiseState<'static>>>,
}

impl Pool {
    /// A fresh state for `channel`, if one is left; the used one is kept in `spent`.
    fn refresh(&mut self, channel: &mut Channel) {
        if !channel.used {
            return;
        }
        if let Some(fresh) = self.spares.pop() {
            let used = std::mem::replace(&mut channel.state, fresh);
            // Within its capacity: as many can be spent as were spare.
            debug_assert!(self.spent.len() < self.spent.capacity());
            self.spent.push(used);
            channel.used = false;
        }
    }
}

/// A sample of a frame below this, in the scale of `RNNoise` (−140 dBFS), is digital silence.
const SILENT: f32 = 1e-7 * RNNOISE_SCALE;

impl Channel {
    /// One sample at 48 kHz, at the place `n` of the grid: the voice of 960 samples before.
    /// After [`RNNOISE_FORGET`] of frames of digital silence in a row (`forget` frames) the
    /// channel takes a fresh state from `pool`.
    fn step(&mut self, n: u64, sample: f32, forget: u32, pool: &mut Pool) -> f32 {
        #[allow(clippy::cast_possible_truncation, reason = "a place in a frame")]
        let at = (n % RNNOISE_FRAME as u64) as usize;
        let out = self.out[at];
        self.frame[at] = sample.clamp(-DENOISE_INPUT_LIMIT, DENOISE_INPUT_LIMIT) * RNNOISE_SCALE;
        if at == RNNOISE_FRAME - 1 {
            if self.frame.iter().all(|x| x.abs() < SILENT) {
                self.quiet = self.quiet.saturating_add(1);
                if self.quiet == forget {
                    pool.refresh(self);
                }
            } else {
                self.quiet = 0;
            }
            self.state.process_frame(&mut self.out, &self.frame);
            self.used = true;
        }
        out / RNNOISE_SCALE
    }

    fn clear(&mut self) {
        self.frame.fill(0.0);
        self.out.fill(0.0);
        self.input.clear();
        self.voice.clear();
        self.dry.clear();
    }
}

/// The voice mode of Noise Reduction (see the module): a [`Node`] on any layout but
/// [`Layout::Empty`], the same out as in, no sidechain, at the rates that [`Voice::latency_for`]
/// takes.
pub struct Voice {
    set: Settings,
    rate: u32,
    latency: u64,
    conversion: Option<Conversion>,
    channels: Vec<Channel>,
    pool: Pool,
    /// The frames of [`RNNOISE_FORGET`].
    forget: u32,
    count: u64,
    origin: u64,
    dry: Planar,
    mixer: Mixer,
}

impl std::fmt::Debug for Voice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Voice")
            .field("set", &self.set)
            .field("rate", &self.rate)
            .field("latency", &self.latency)
            .field("channels", &self.channels.len())
            .field("spares", &self.pool.spares.len())
            .finish_non_exhaustive()
    }
}

impl Voice {
    /// The voice set as `settings` (its mode taken as the voice).
    #[must_use]
    pub fn new(settings: &Settings) -> Self {
        let set = settings.clamped();
        Voice {
            mixer: Mixer::new(&set),
            set,
            rate: 0,
            latency: 0,
            conversion: None,
            channels: Vec::new(), // rt-ok: empty, before the graph is built
            pool: Pool {
                spares: Vec::new(), // rt-ok: empty, before the graph is built
                spent: Vec::new(),  // rt-ok: empty, before the graph is built
            },
            forget: 0,
            count: 0,
            origin: 0,
            dry: Planar::new(0, 0),
        }
    }

    /// The delay of the voice at `rate`, or `None` when the rate is refused.
    #[must_use]
    pub fn latency_for(rate: u32) -> Option<u32> {
        plan(rate).and_then(|(latency, _)| u32::try_from(latency).ok())
    }

    /// The settling ([`Node::settle`]) of the voice at `rate`: [`RNNOISE_SETTLE`], the delay and
    /// the ramp of 10 ms. The settings do not change it.
    #[must_use]
    pub fn settle_for(settings: &Settings, rate: u32) -> u32 {
        let _ = settings;
        let latency = Voice::latency_for(rate).map_or(0, |latency| latency as usize);
        let total = samples(RNNOISE_SETTLE + SMOOTHING, rate) + latency + 2 * RNNOISE_FRAME;
        u32::try_from(total).unwrap_or(u32::MAX)
    }

    /// The fresh states left for resets.
    #[must_use]
    pub fn spares(&self) -> usize {
        self.pool.spares.len()
    }

    /// The parameters as set (their targets).
    #[must_use]
    pub fn settings(&self) -> &Settings {
        &self.set
    }
}

impl Node for Voice {
    fn prepare(&mut self, spec: &Spec) -> Result<(), Unsupported> {
        let refused = || Unsupported::from(spec);
        if spec.layout == Layout::Empty || spec.output != spec.layout || spec.sidechain != 0 {
            return Err(refused());
        }
        let (latency, delays) = plan(spec.rate).ok_or_else(refused)?;
        let conversion = conversion(spec.rate, delays);
        self.rate = spec.rate;
        self.latency = latency;
        self.origin = 0;
        let (input, voice) = conversion.as_ref().map_or((1, 1), |c| {
            let (p, q) = c.up.ratio();
            #[allow(clippy::cast_possible_truncation, reason = "a few thousand samples")]
            let voice = (c.down.delay() + c.down.reach() as u64 + q.div_ceil(p) + 4) as usize;
            (2 * c.up.reach() + 2, voice)
        });
        self.conversion = conversion;
        #[allow(
            clippy::cast_possible_truncation,
            reason = "a delay of a few thousand samples"
        )]
        let dry = latency as usize + 1;
        let channels = spec.inputs();
        self.channels = (0..channels)
            .map(|_| Channel {
                state: DenoiseState::new(),
                used: false,
                quiet: 0,
                frame: [0.0; RNNOISE_FRAME],
                out: [0.0; RNNOISE_FRAME],
                input: History::new(input),
                voice: History::new(voice),
                dry: History::new(dry),
            })
            .collect(); // rt-ok: when the graph is built
        let spares: Vec<_> = (0..channels * RNNOISE_SPARE_STATES)
            .map(|_| DenoiseState::new())
            .collect(); // rt-ok: when the graph is built
        let spent = Vec::with_capacity(spares.len()); // rt-ok: when the graph is built
        self.pool = Pool { spares, spent };
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss,
            reason = "a hundred frames"
        )]
        let forget =
            (RNNOISE_FORGET * f64::from(RNNOISE_RATE) / RNNOISE_FRAME as f64).ceil() as u32;
        self.forget = forget;
        self.dry = Planar::new(channels, spec.max_block);
        self.mixer.prepare(spec.rate);
        self.reset();
        Ok(())
    }

    fn process(&mut self, ctx: &Block, input: &[&[f32]], output: &mut [&mut [f32]]) {
        let frames = ctx.frames;
        if frames == 0 {
            return;
        }
        let Voice {
            conversion,
            channels,
            pool,
            forget,
            count,
            origin,
            latency,
            dry,
            mixer,
            ..
        } = self;
        let first = origin.wrapping_add(*count);
        dry.write(frames, |dry| {
            for (c, channel) in channels.iter_mut().enumerate() {
                for i in 0..frames {
                    let at = first + i as u64;
                    let sample = input[c][i];
                    let sample = if sample.is_finite() { sample } else { 0.0 };
                    channel.dry.put(at, sample);
                    #[allow(clippy::cast_possible_wrap, reason = "places on a timeline")]
                    let back = at as i64 - *latency as i64;
                    dry[c][i] = channel.dry.get(back);
                    output[c][i] = match conversion {
                        None => channel.step(at, sample, *forget, pool),
                        Some(conversion) => {
                            channel.input.put(at, sample);
                            for n in conversion.up.outputs_at(at) {
                                let x = conversion.up.output(n, &channel.input);
                                let y = channel.step(n, x, *forget, pool);
                                channel.voice.put(n, y);
                            }
                            conversion.down.output(at, &channel.voice)
                        }
                    };
                }
            }
        });
        *count += frames as u64;
        dry.read(frames, |dry| {
            mixer.mix(frames, &ctx.automation, dry, output);
        });
    }

    fn latency(&self) -> u32 {
        u32::try_from(self.latency).unwrap_or(u32::MAX)
    }

    /// [`Voice::settle_for`] at the rate prepared.
    fn settle(&self) -> u32 {
        Voice::settle_for(&self.set, self.rate)
    }

    fn reset(&mut self) {
        for channel in &mut self.channels {
            channel.clear();
            channel.quiet = 0;
            self.pool.refresh(channel);
        }
        self.count = 0;
        self.mixer.finish();
    }

    /// Lays the frames of 48 kHz, and the grid on which another rate meets it, on the timeline:
    /// the sample of 48 kHz `n` is at `n / 48000` s from its start.
    fn align(&mut self, origin: u64) {
        self.origin = origin;
    }

    fn set(&mut self, param: ParamId, value: f32) {
        let Some(param) = Param::from_id(param) else {
            return;
        };
        if !value.is_finite() {
            return;
        }
        self.set.set(param, value);
        self.mixer.set(param, param.clamp(value));
    }
}

/// Whether the voice reads `param`: those of the mix, not those of the gate.
#[must_use]
pub fn applies(param: Param) -> bool {
    !param.is_spectral()
}
