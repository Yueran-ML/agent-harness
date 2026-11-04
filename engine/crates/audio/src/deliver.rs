//! The sound of an export (plan T6): the same [`Graph`] as playback, fed by a reader that waits
//! for the sound of its clips instead of reading ahead, and the bus that is rendered taken from
//! it as it is; then what an export of a standard of loudness puts after it, and the rate of the
//! file.
//!
//! # The path of an export
//!
//! 1. [`Graph::start`] at the first sample of the range, with the preroll of a playback that
//!    starts there ([`Graph::preroll`]: [`Graph::settle`], at most
//!    [`crate::constants::PREROLL_MAX`]), so that the export of a range and a playback from the
//!    same sample are the same to the bit, and an export from the start of the timeline is the
//!    one reference that both come near after their preroll (T8's review).
//! 2. The output of the bus that is rendered ([`Graph::output_of`]): a main output or the bus
//!    that is monitored, in its own format; nothing clipped.
//! 3. [`Finish`]: a gain, and a limiter of true peaks (T10's [`Limiter`], 64 samples ahead,
//!    made up so that sample `n` of the timeline is still sample `n` of the file), for Audio
//!    Normalization of the Deliver page (p. 4055; [`normalization`]). Nothing where it is not
//!    asked for: the output of the graph passes to the bit.
//! 4. The rate of the file ([`Rerate`]): where it is not the rate of the timeline, the windowed
//!    sinc of the clips' speeds, as the device's rate is made (T5's `stream`), on every channel
//!    and not clipped. Where it is, nothing is done.
//!
//! What is clipped, for a file of fixed point (and AAC, as the exports before it were), and how
//! the channels are laid in the file, the caller does (the server's `render.rs`).
//!
//! # Audio Normalization (p. 4055)
//!
//! [`normalization`] measures the bus (T9's [`Analyzer`], BS.1770 with the true peak), and finds
//! the [`Finish`] that brings it to a standard: Normalize scales the gain only, to whichever of
//! the target loudness and the highest true peak it meets first; Optimize brings it to the target
//! loudness and, where its true peak would go over, puts the limiter after the gain with its
//! ceiling at the standard's highest true peak, then measures again: the loudness that the
//! limiter took off is given back once where it is more than 0.1 LU, and the ceiling is lowered
//! by what the true peak still goes over, twice at the most (T10's review). It changes the
//! export only, never the project. The agent's `video_loudness` (T24) measures through another
//! closure (the project with a fader and a Limiter insert) and finds its settings by the same
//! code.

use crate::analysis::{Analysis, Analyzer, Level};
use crate::constants::{
    EXPORT_LIMITER_RELEASE_MS, LIMITER_LOOKAHEAD, NORMALIZE_CEILING_LOWERINGS,
    NORMALIZE_LOUDNESS_TOLERANCE,
};
use crate::feed::{ClipFeed, FeedError};
use crate::fx::limiter::{self, Limiter};
use crate::graph::{radius, resampled, Graph};
use crate::node::{Block, Layout, Node, Planar, Spec};

/// What an export puts after the bus that it renders.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Finish {
    /// The gain, in dB.
    pub gain_db: f64,
    /// The ceiling of a limiter of true peaks after the gain, in dBTP (−24 to 0); none for no
    /// limiter.
    pub ceiling_db: Option<f64>,
}

/// The bus of an export, taken from its graph block after block, with its [`Finish`] after it.
#[derive(Debug)]
pub struct Bounce {
    output: usize,
    layout: Layout,
    rate: u32,
    gain: f32,
    limiter: Option<Box<Limiter>>,
    /// The samples given to the limiter since it was reset.
    fed: u64,
    /// The bus after the gain.
    scratch: Planar,
    out: Planar,
}

impl Bounce {
    /// The output `output` of `graph` (see [`Graph::output_of`]) with `finish` after it.
    ///
    /// # Panics
    ///
    /// When `graph` has no such output.
    #[must_use]
    pub fn new(graph: &Graph, output: usize, finish: Finish) -> Self {
        let layout = graph.outputs()[output].layout;
        let channels = layout.channels();
        let max_block = graph.max_block();
        #[allow(clippy::cast_possible_truncation, reason = "a gain of a few decibels")]
        let gain = 10.0_f64.powf(finish.gain_db / 20.0) as f32;
        let limiter = finish.ceiling_db.map(|ceiling| {
            #[allow(clippy::cast_possible_truncation, reason = "−24 to 0 dB")]
            let settings = limiter::Settings {
                ceiling_db: ceiling.clamp(-24.0, 0.0) as f32,
                release_ms: EXPORT_LIMITER_RELEASE_MS,
                ..limiter::Settings::default()
            };
            let mut node = Box::new(Limiter::new(&settings));
            node.prepare(&Spec::new(graph.rate(), max_block, layout))
                .expect("a limiter takes the channels of every bus");
            node
        });
        Bounce {
            output,
            layout,
            rate: graph.rate(),
            gain,
            limiter,
            fed: 0,
            scratch: Planar::new(channels, max_block),
            out: Planar::new(channels, max_block),
        }
    }

    /// The channels of the bus.
    #[must_use]
    pub fn layout(&self) -> Layout {
        self.layout
    }

    /// Starts at sample `at` of the timeline: the graph started there ([`Graph::start`]), and
    /// the lookahead of the limiter, if there is one, run and thrown away, so that the first
    /// sample of [`Bounce::next`] is the sound of sample `at`.
    ///
    /// # Errors
    ///
    /// The feed could not give the sound of a clip (as [`Graph::process`]).
    pub fn start(
        &mut self,
        graph: &mut Graph,
        feed: &mut dyn ClipFeed,
        at: i64,
    ) -> Result<(), FeedError> {
        graph.start(at, feed)?;
        self.fed = 0;
        if let Some(limiter) = &mut self.limiter {
            limiter.reset();
            let mut left = usize::try_from(limiter.latency()).unwrap_or(LIMITER_LOOKAHEAD);
            while left > 0 {
                let frames = left.min(graph.max_block());
                self.next(graph, feed, frames)?;
                left -= frames;
            }
        }
        Ok(())
    }

    /// The next `frames` samples (at most [`Graph::max_block`]): the bus, its gain and its
    /// limiter; the first `frames` of each channel of what it gives.
    ///
    /// # Errors
    ///
    /// The feed could not give the sound of a clip (as [`Graph::process`]).
    pub fn next(
        &mut self,
        graph: &mut Graph,
        feed: &mut dyn ClipFeed,
        frames: usize,
    ) -> Result<&Planar, FeedError> {
        let frames = frames.min(graph.max_block());
        graph.process(feed, frames)?;
        let bus = graph.output(self.output);
        let target = if self.limiter.is_some() {
            &mut self.scratch
        } else {
            &mut self.out
        };
        for channel in 0..bus.channels() {
            let from = &bus.channel(channel)[..frames];
            let into = &mut target.channel_mut(channel)[..frames];
            if self.gain.to_bits() == 1.0_f32.to_bits() {
                into.copy_from_slice(from);
            } else {
                for (into, sample) in into.iter_mut().zip(from) {
                    *into = sample * self.gain;
                }
            }
        }
        if let Some(limiter) = &mut self.limiter {
            let block = Block::new(self.fed, self.rate, frames);
            let scratch = &self.scratch;
            scratch.read(frames, |input| {
                self.out
                    .write(frames, |output| limiter.process(&block, input, output));
            });
            self.fed += u64::try_from(frames).unwrap_or(0);
        }
        Ok(&self.out)
    }
}

/// The sound of the timeline made into another rate, on every channel, not clipped: output
/// sample `k` of a run from sample `at` of the timeline is the timeline at `at + k · rate / to`
/// (exactly: `at · to + k · rate` in units of `1 / to` of a sample), by the windowed sinc of the
/// clips' speeds at the cutoff `min(to / rate, 1)`, as T5's `stream` makes the device's rate.
#[derive(Debug)]
pub struct Rerate {
    rate: u32,
    to: u32,
    cutoff: f64,
    reach: i64,
    origin: i64,
    given: i64,
    /// The sample of the timeline of the first sample kept.
    base: i64,
    kept: Vec<Vec<f32>>,
}

impl Rerate {
    /// From a timeline of `rate` to `to` samples a second, `channels` channels.
    ///
    /// # Panics
    ///
    /// When a rate is 0.
    #[must_use]
    pub fn new(rate: u32, to: u32, channels: usize) -> Self {
        assert!(rate > 0 && to > 0, "a rate of nothing");
        let cutoff = (f64::from(to) / f64::from(rate)).min(1.0);
        let reach = if rate == to { 0 } else { radius(cutoff) };
        Rerate {
            rate,
            to,
            cutoff,
            reach,
            origin: 0,
            given: 0,
            base: 0,
            kept: vec![Vec::new(); channels],
        }
    }

    /// Starts again, the first sample out at sample `at` of the timeline; the sound is to be
    /// given from [`Rerate::first_input`] on.
    pub fn start(&mut self, at: i64) {
        self.origin = at;
        self.given = 0;
        self.base = self.first_input();
        for kept in &mut self.kept {
            kept.clear();
        }
    }

    /// The sample of the timeline that the sound is to be given from.
    #[must_use]
    pub fn first_input(&self) -> i64 {
        self.origin - self.reach
    }

    /// The sample of the timeline, and the share of the way to the next, of output sample
    /// `frame`.
    fn position(&self, frame: i64) -> (i64, u32) {
        let position = i128::from(self.origin) * i128::from(self.to)
            + i128::from(frame) * i128::from(self.rate);
        let to = i128::from(self.to);
        (
            i64::try_from(position.div_euclid(to)).unwrap_or(i64::MAX),
            u32::try_from(position.rem_euclid(to)).unwrap_or(0),
        )
    }

    /// Takes the next samples of the timeline, a slice for each channel, all of one length.
    pub fn push(&mut self, channels: &[&[f32]]) {
        for (kept, channel) in self.kept.iter_mut().zip(channels) {
            kept.extend_from_slice(channel);
        }
    }

    /// Puts out as many samples as what was given makes, at most `most`, onto the end of each
    /// channel of `out`; how many.
    #[allow(
        clippy::cast_precision_loss,
        reason = "the share of the way to the next sample is a fraction in [0, 1)"
    )]
    pub fn pull(&mut self, most: usize, out: &mut [Vec<f32>]) -> usize {
        let held = i64::try_from(self.kept.first().map_or(0, Vec::len)).unwrap_or(i64::MAX);
        let mut made = 0;
        let mut frame = vec![0.0_f32; self.kept.len()];
        while made < most {
            let (sample, remainder) = self.position(self.given);
            let lowest = if self.reach == 0 {
                sample
            } else {
                sample - self.reach + 1
            };
            if sample + self.reach >= self.base + held || lowest < self.base {
                break;
            }
            let index = usize::try_from(sample - self.base).unwrap_or(0);
            let window: Vec<&[f32]> = self.kept.iter().map(Vec::as_slice).collect();
            resampled(
                &window,
                index,
                f64::from(remainder) / f64::from(self.to),
                self.cutoff,
                &mut frame,
            );
            for (out, sample) in out.iter_mut().zip(&frame) {
                out.push(*sample);
            }
            self.given += 1;
            made += 1;
        }
        // What the next sample out no longer reaches is let go.
        let (next, _) = self.position(self.given);
        let lowest = if self.reach == 0 {
            next
        } else {
            next - self.reach + 1
        };
        let gone = usize::try_from(lowest - self.base)
            .unwrap_or(0)
            .min(usize::try_from(held).unwrap_or(0));
        if gone > 0 {
            for kept in &mut self.kept {
                kept.drain(..gone);
            }
            self.base += i64::try_from(gone).unwrap_or(0);
        }
        made
    }
}

/// The sound of an export: its bus with its [`Finish`], at the rate of the file, given a run at a
/// time ([`Render::take`]).
#[derive(Debug)]
pub struct Render {
    bounce: Bounce,
    rerate: Option<Rerate>,
    /// What is made and not yet taken, each channel.
    made: Vec<Vec<f32>>,
}

impl Render {
    /// The output `output` of `graph` with `finish` after it, at `rate` samples a second, from
    /// sample `at` of the timeline: the graph started there (and as far before as the filter of
    /// another rate reaches).
    ///
    /// # Errors
    ///
    /// The feed could not give the sound of a clip (as [`Graph::process`]).
    ///
    /// # Panics
    ///
    /// When `graph` has no such output, or `rate` is 0.
    pub fn start(
        graph: &mut Graph,
        feed: &mut dyn ClipFeed,
        (output, finish): (usize, Finish),
        at: i64,
        rate: u32,
    ) -> Result<Self, FeedError> {
        let mut bounce = Bounce::new(graph, output, finish);
        let channels = bounce.layout().channels();
        let rerate = (rate != graph.rate()).then(|| {
            let mut rerate = Rerate::new(graph.rate(), rate, channels);
            rerate.start(at);
            rerate
        });
        let from = rerate.as_ref().map_or(at, Rerate::first_input);
        bounce.start(graph, feed, from)?;
        Ok(Render {
            bounce,
            rerate,
            made: vec![Vec::new(); channels],
        })
    }

    /// The channels of the bus.
    #[must_use]
    pub fn layout(&self) -> Layout {
        self.bounce.layout()
    }

    /// The next `frames` samples of the file, a vector for each channel.
    ///
    /// # Errors
    ///
    /// The feed could not give the sound of a clip (as [`Graph::process`]).
    pub fn take(
        &mut self,
        graph: &mut Graph,
        feed: &mut dyn ClipFeed,
        frames: usize,
    ) -> Result<Vec<Vec<f32>>, FeedError> {
        while self.made.first().map_or(0, Vec::len) < frames {
            let wanted = frames - self.made[0].len();
            match &mut self.rerate {
                None => {
                    let count = wanted.min(graph.max_block());
                    let bus = self.bounce.next(graph, feed, count)?;
                    for (channel, made) in self.made.iter_mut().enumerate() {
                        made.extend_from_slice(&bus.channel(channel)[..count]);
                    }
                }
                Some(rerate) => {
                    if rerate.pull(wanted, &mut self.made) == 0 {
                        let count = graph.max_block();
                        let bus = self.bounce.next(graph, feed, count)?;
                        bus.read(count, |channels| rerate.push(channels));
                    }
                }
            }
        }
        Ok(self
            .made
            .iter_mut()
            .map(|made| made.drain(..frames).collect())
            .collect())
    }
}

/// The analysis of `frames` samples of the file of an export from sample `at` of the timeline:
/// output `output` of `graph` with `finish` after it, at `rate` (BS.1770 loudness, its true peak
/// and its sample peak).
///
/// # Errors
///
/// The feed could not give the sound of a clip (as [`Graph::process`]).
pub fn measure(
    graph: &mut Graph,
    feed: &mut dyn ClipFeed,
    (output, finish): (usize, Finish),
    (at, frames): (i64, usize),
    rate: u32,
) -> Result<Analysis, FeedError> {
    let mut render = Render::start(graph, feed, (output, finish), at, rate)?;
    let mut analyzer = Analyzer::new(rate, render.layout(), true);
    let mut left = frames;
    while left > 0 {
        let count = left.min(graph.max_block());
        let run = render.take(graph, feed, count)?;
        let channels: Vec<&[f32]> = run.iter().map(Vec::as_slice).collect();
        analyzer.take(&channels);
        left -= count;
    }
    Ok(analyzer.finish())
}

/// What Audio Normalization brings the bus to (p. 4055).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aim {
    /// How the loudness is measured: [`Level::Loudness`] of the standard.
    pub level: Level,
    /// The target, in LUFS.
    pub target: f64,
    /// The highest true peak, in dBTP, if the standard has one.
    pub true_peak: Option<f64>,
}

/// How Audio Normalization gets there (p. 4055).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Normalize to Standard: the gain only, to whichever of the target and the highest true peak
    /// is met first.
    Normalize,
    /// Optimize to Standard: the target, and a limiter of true peaks where the gain would take
    /// the true peak over the highest.
    Optimize,
}

/// What Audio Normalization found.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Normalization {
    /// What the export puts after its bus.
    pub finish: Finish,
    /// The bus as it is.
    pub before: Analysis,
    /// The bus with `finish` after it, measured; none where nothing was put after it but a gain
    /// (whose loudness and peaks are those of `before` raised by it).
    pub after: Option<Analysis>,
    /// Whether it meets the target within [`NORMALIZE_LOUDNESS_TOLERANCE`] and the highest true
    /// peak; false too where the bus has no loudness (silence, or shorter than a block of
    /// 400 ms), which is left as it is.
    pub met: bool,
    /// How many times the bus was measured.
    pub measured: u32,
}

/// Finds what brings the bus to `aim` by `mode` (see the module), measuring it as it would be
/// with a [`Finish`] by `measure`.
///
/// # Errors
///
/// What `measure` fails with.
pub fn normalization<E>(
    mode: Mode,
    aim: &Aim,
    mut measure: impl FnMut(Finish) -> Result<Analysis, E>,
) -> Result<Normalization, E> {
    let before = measure(Finish::default())?;
    let mut measured = 1;
    let Some(level) = aim.level.of_analysis(&before) else {
        return Ok(Normalization {
            finish: Finish::default(),
            before,
            after: None,
            met: false,
            measured,
        });
    };
    let mut gain = aim.target - level;
    let peak = aim.true_peak;
    let within = |loudness: Option<f64>| {
        loudness
            .is_some_and(|loudness| (aim.target - loudness).abs() <= NORMALIZE_LOUDNESS_TOLERANCE)
    };
    let gain_only = |gain: f64| {
        let met =
            within(Some(level + gain)) && peak.is_none_or(|peak| before.true_peak + gain <= peak);
        Normalization {
            finish: Finish {
                gain_db: gain,
                ceiling_db: None,
            },
            before,
            after: None,
            met,
            measured: 1,
        }
    };
    match (mode, peak) {
        (Mode::Normalize, Some(peak)) => return Ok(gain_only(gain.min(peak - before.true_peak))),
        (_, None) => return Ok(gain_only(gain)),
        (Mode::Optimize, Some(peak)) if before.true_peak + gain <= peak => {
            return Ok(gain_only(gain))
        }
        (Mode::Optimize, Some(_)) => {}
    }
    let peak = peak.unwrap_or(0.0);
    let mut ceiling = peak;
    let mut given_back = false;
    let mut lowered = 0;
    loop {
        let finish = Finish {
            gain_db: gain,
            ceiling_db: Some(ceiling),
        };
        let after = measure(finish)?;
        measured += 1;
        let loudness = aim.level.of_analysis(&after);
        if let Some(loudness) = loudness {
            if !given_back && !within(Some(loudness)) {
                gain += aim.target - loudness;
                given_back = true;
                continue;
            }
        }
        if after.true_peak > peak && lowered < NORMALIZE_CEILING_LOWERINGS {
            ceiling -= after.true_peak - peak;
            lowered += 1;
            continue;
        }
        return Ok(Normalization {
            finish,
            before,
            after: Some(after),
            met: within(loudness) && after.true_peak <= peak,
            measured,
        });
    }
}
