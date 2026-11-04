//! The strip of a track or a bus (plan T4b; pp. 3855, 3857–3860, 3869–3874, 3882–3887): the sum
//! of what feeds it → Trim and polarity → the inserts, Dynamics and equalizer in their Order
//! (the nodes) → the point before the fader → the mute and the solo → the fader, with its VCAs →
//! the point after the fader → its outputs and sends, each at its level and on its pan, delayed to
//! line up with the other paths into its bus. A send before the fader is taken after the mute
//! (C-B4).
//!
//! What is set once is a parameter ramped over 10 ms ([`Smoothed`], C-T1); where a lane of
//! automation is, its value takes the place of the parameter, sample by sample (T14).

use columbina_model::{Lane, Time};

use crate::constants::MAX_CHANNELS;
use crate::db_to_gain;
use crate::node::{Automated, Automation, Block, Layout, Node, ParamId, Planar};
use crate::rt::Smoothed;

use super::pdc::DelayLine;
use super::route::{panned, Matrix, PanAt};
use super::taps::Tap;

/// The most lanes of one node that a block hands it.
pub(crate) const MAX_NODE_LANES: usize = 64;

/// A lane of automation of a strip, read `delay` samples behind the input of the graph: the
/// latency of the graph before the point where it acts, so that it acts on the sound of the
/// time it is at.
#[derive(Debug)]
pub(crate) struct LaneOf {
    pub(crate) lane: Lane,
    pub(crate) delay: i64,
}

/// A node of a strip: an insert, its Dynamics or its equalizer.
pub(crate) struct Stage {
    pub(crate) node: Box<dyn Node>,
    /// Its parameters that lanes set, and the lanes.
    pub(crate) lanes: Box<[(ParamId, usize)]>,
    /// Where its sidechain comes from, if it has one; the sidechain of the block; and the
    /// delay that lines the sidechain up with the sound it acts on (T4b's review, P3-6).
    pub(crate) key: Option<Key>,
    pub(crate) keyed: Planar,
    pub(crate) side: DelayLine,
    /// The latency of the graph before it: its sample 0 is that of the timeline this far back.
    pub(crate) offset: i64,
    /// The samples it has processed since it was reset ([`Block::start`]).
    pub(crate) processed: u64,
}

/// Where the sidechain of a strip comes from: the sound before the fader of another strip, its
/// channels but the LFE (T8's review).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Key {
    pub(crate) source: usize,
    pub(crate) channels: [usize; MAX_CHANNELS],
    pub(crate) count: usize,
}

/// A VCA that a strip is a member of, with the lanes of its fader and mute as the strip reads
/// them.
#[derive(Clone, Copy, Debug)]
pub(crate) struct VcaTerm {
    pub(crate) vca: usize,
    pub(crate) fader: Option<usize>,
    pub(crate) mute: Option<usize>,
}

/// A VCA, through a block: its gain and whether it lets its members be heard (1) or mutes them
/// (0), at each sample.
#[derive(Debug)]
pub(crate) struct VcaState {
    pub(crate) fader: Smoothed,
    pub(crate) mute: Smoothed,
    pub(crate) gain: Box<[f32]>,
    pub(crate) heard: Box<[f32]>,
}

impl VcaState {
    /// Its values through the next `frames` samples.
    pub(crate) fn advance(&mut self, frames: usize) {
        self.fader.fill(&mut self.gain[..frames]);
        self.mute.fill(&mut self.heard[..frames]);
    }
}

/// The pan of an output or a send: as it is set, ramped, and its lanes (x, y, divergence).
#[derive(Debug)]
pub(crate) struct Pan {
    pub(crate) base: PanAt,
    pub(crate) x: Smoothed,
    pub(crate) y: Smoothed,
    pub(crate) divergence: Smoothed,
    pub(crate) lanes: [Option<usize>; 3],
    /// What the matrix of its edge was last worked out for, while it moves.
    pub(crate) last: PanAt,
}

impl Pan {
    /// Whether it changes within a block: a lane, or a ramp under way.
    fn moves(&self) -> bool {
        self.lanes.iter().any(Option::is_some)
            || self.x.is_ramping()
            || self.y.is_ramping()
            || self.divergence.is_ramping()
    }

    /// As it is set now.
    fn now(&self) -> PanAt {
        PanAt {
            x: self.x.current(),
            y: self.y.current(),
            divergence: self.divergence.current(),
            ..self.base
        }
    }
}

/// An output or a send of a strip to a bus.
pub(crate) struct Edge {
    pub(crate) dest: usize,
    /// A send (not an output).
    pub(crate) send: bool,
    /// A send taken before the fader.
    pub(crate) pre: bool,
    /// Its level, a gain (1 for an output), and its lane, of decibels.
    pub(crate) level: Smoothed,
    pub(crate) level_lane: Option<usize>,
    /// Whether it is on (1) or not (0), and its lane, a switch.
    pub(crate) on: Smoothed,
    pub(crate) on_lane: Option<usize>,
    pub(crate) pan: Pan,
    pub(crate) from: Layout,
    pub(crate) to: Layout,
    /// The matrix of its pan as it is set.
    pub(crate) matrix: Matrix,
    /// The delay that lines it up with the other paths into its bus.
    pub(crate) delay: DelayLine,
    /// How many more samples of silence its delay line takes before it holds nothing: what went
    /// in before it was switched off still comes out, and nothing of it is left for when it is
    /// switched on again.
    pub(crate) stale: usize,
}

impl Edge {
    /// Whether it sends nothing through the block: off, and staying so.
    fn silent(&self) -> bool {
        self.on_lane.is_none() && !self.on.is_ramping() && self.on.current() == 0.0
    }
}

/// The strip of a track or a bus in the graph.
pub(crate) struct Strip {
    pub(crate) layout: Layout,
    pub(crate) trim: Smoothed,
    pub(crate) trim_lane: Option<usize>,
    /// The polarity, 1 or −1.
    pub(crate) sign: Smoothed,
    pub(crate) sign_lane: Option<usize>,
    pub(crate) stages: Vec<Stage>,
    pub(crate) fader: Smoothed,
    pub(crate) fader_lane: Option<usize>,
    /// 1 where it is heard, 0 where it is muted.
    pub(crate) mute: Smoothed,
    pub(crate) mute_lane: Option<usize>,
    /// 1 where it is heard, 0 where a solo elsewhere silences it.
    pub(crate) audible: Smoothed,
    pub(crate) vcas: Vec<VcaTerm>,
    pub(crate) edges: Vec<Edge>,
    pub(crate) lanes: Vec<LaneOf>,
    /// The values of the lanes through the block, a lane after another, each `max_block` long.
    pub(crate) values: Box<[f32]>,
    pub(crate) max_block: usize,
    /// Through the block: the mute and solo, the fader and VCAs, and an edge's level.
    pub(crate) heard: Box<[f32]>,
    pub(crate) gains: Box<[f32]>,
    pub(crate) levels: Box<[f32]>,
    /// The sound through the strip, a spare for the nodes, and the sound after the fader.
    pub(crate) work: Planar,
    pub(crate) spare: Planar,
    pub(crate) post: Planar,
    pub(crate) pre_tap: Tap,
    pub(crate) post_tap: Tap,
}

/// A switch read from a lane: on at 0.5 and above.
fn on(value: f32) -> bool {
    value >= 0.5
}

impl Strip {
    /// The channels.
    pub(crate) fn channels(&self) -> usize {
        self.layout.channels()
    }

    /// The value of lane `lane` at sample `n` of the block.
    fn value(&self, lane: usize, n: usize) -> f32 {
        self.values[lane * self.max_block + n]
    }

    /// Takes the sum that feeds it, and the values of its lanes, for a block of `frames` whose
    /// input is at sample `start` of the timeline.
    pub(crate) fn load(&mut self, input: &Planar, start: i64, rate: u32, frames: usize) {
        for channel in 0..self.channels() {
            self.work.channel_mut(channel)[..frames]
                .copy_from_slice(&input.channel(channel)[..frames]);
        }
        let scale = rate;
        for (index, lane) in self.lanes.iter().enumerate() {
            let values = &mut self.values[index * self.max_block..][..frames];
            for (ticks, value) in (start - lane.delay..).zip(values) {
                #[allow(clippy::cast_possible_truncation, reason = "a value of a parameter")]
                let at = lane.lane.value_at(Time { ticks, scale }) as f32;
                *value = at;
            }
        }
    }

    /// Trim and polarity, the nodes, the point before the fader, and the mute, solo, fader and
    /// VCAs into the sound after the fader, for a block of `frames` at `rate`, the sidechain taken
    /// from `pres` (the sound before the fader of every strip).
    pub(crate) fn run(&mut self, rate: u32, frames: usize, pres: &[Planar], vcas: &[VcaState]) {
        let channels = self.channels();
        self.trim(frames);
        self.nodes(rate, frames, pres);
        self.pre_tap.take(&self.work, channels, frames);
        self.fade(frames, vcas);
        for channel in 0..channels {
            let post = &mut self.post.channel_mut(channel)[..frames];
            let work = &self.work.channel(channel)[..frames];
            for (n, (post, sample)) in post.iter_mut().zip(work).enumerate() {
                *post = sample * self.heard[n] * self.gains[n];
            }
        }
        self.post_tap.take(&self.post, channels, frames);
    }

    /// Trim and polarity; nothing at all where they are at unity and stay there.
    #[allow(
        clippy::float_cmp,
        reason = "unity, exactly, leaves the sound to the bit"
    )]
    fn trim(&mut self, frames: usize) {
        let channels = self.channels();
        if self.trim_lane.is_some()
            || self.sign_lane.is_some()
            || self.trim.is_ramping()
            || self.sign.is_ramping()
            || self.trim.current() != 1.0
            || self.sign.current() != 1.0
        {
            for n in 0..frames {
                let trim = match self.trim_lane {
                    Some(lane) => db_to_gain(self.value(lane, n)),
                    None => self.trim.next_value(),
                };
                let sign = match self.sign_lane {
                    Some(lane) => {
                        if on(self.value(lane, n)) {
                            -1.0
                        } else {
                            1.0
                        }
                    }
                    None => self.sign.next_value(),
                };
                self.gains[n] = trim * sign;
            }
            for channel in 0..channels {
                for (sample, gain) in self.work.channel_mut(channel)[..frames]
                    .iter_mut()
                    .zip(&self.gains[..frames])
                {
                    *sample *= gain;
                }
            }
        }
    }

    /// The nodes, in their order, the sidechain taken from `pres`.
    fn nodes(&mut self, rate: u32, frames: usize, pres: &[Planar]) {
        let Strip {
            stages,
            values,
            max_block,
            work,
            spare,
            ..
        } = self;
        for stage in stages.iter_mut() {
            if let Some(key) = &stage.key {
                let source = &pres[key.source];
                for (into, &from) in key.channels[..key.count].iter().enumerate() {
                    stage.keyed.channel_mut(into)[..frames]
                        .copy_from_slice(&source.channel(from)[..frames]);
                }
                stage.side.process(&mut stage.keyed, key.count, frames);
            }
            let mut list = [Automated {
                param: ParamId(0),
                values: &[],
            }; MAX_NODE_LANES];
            let mut count = 0;
            for &(param, lane) in stage.lanes.iter().take(MAX_NODE_LANES) {
                list[count] = Automated {
                    param,
                    values: &values[lane * *max_block..][..frames],
                };
                count += 1;
            }
            let mut side: [&[f32]; MAX_CHANNELS] = [&[]; MAX_CHANNELS];
            let sides = match &stage.key {
                Some(key) => {
                    for (side, channel) in side.iter_mut().zip(0..key.count) {
                        *side = &stage.keyed.channel(channel)[..frames];
                    }
                    key.count
                }
                None => 0,
            };
            let block = Block {
                start: stage.processed,
                rate,
                frames,
                sidechain: &side[..sides],
                automation: Automation::new(&list[..count]),
            };
            work.read(frames, |input| {
                spare.write(frames, |output| stage.node.process(&block, input, output));
            });
            std::mem::swap(work, spare);
            stage.processed += u64::try_from(frames).unwrap_or(0);
        }
    }

    /// The mute, solo, fader and VCAs at each sample: whether it is heard, and its gain.
    fn fade(&mut self, frames: usize, vcas: &[VcaState]) {
        for n in 0..frames {
            let mut heard = match self.mute_lane {
                Some(lane) => {
                    if on(self.value(lane, n)) {
                        0.0
                    } else {
                        1.0
                    }
                }
                None => self.mute.next_value(),
            };
            heard *= self.audible.next_value();
            let mut gain = match self.fader_lane {
                Some(lane) => db_to_gain(self.value(lane, n)),
                None => self.fader.next_value(),
            };
            for term in &self.vcas {
                let vca = &vcas[term.vca];
                gain *= match term.fader {
                    Some(lane) => db_to_gain(self.value(lane, n)),
                    None => vca.gain[n],
                };
                heard *= match term.mute {
                    Some(lane) => {
                        if on(self.value(lane, n)) {
                            0.0
                        } else {
                            1.0
                        }
                    }
                    None => vca.heard[n],
                };
            }
            self.heard[n] = heard;
            self.gains[n] = gain;
        }
    }

    /// Adds what it outputs and sends through the block to the sums of the buses, `inputs`, by
    /// way of `scratch`.
    pub(crate) fn route(&mut self, frames: usize, scratch: &mut Planar, inputs: &mut [Planar]) {
        let channels = self.channels();
        let Strip {
            edges,
            values,
            max_block,
            levels,
            heard,
            work,
            post,
            ..
        } = self;
        let value = |lane: usize, n: usize| values[lane * *max_block + n];
        for edge in edges.iter_mut() {
            let quiet = edge.silent();
            if quiet && edge.stale == 0 {
                continue;
            }
            for (n, level) in levels[..frames].iter_mut().enumerate() {
                let gain = match edge.level_lane {
                    Some(lane) => db_to_gain(value(lane, n)),
                    None => edge.level.next_value(),
                };
                let switched = match edge.on_lane {
                    Some(lane) => {
                        if on(value(lane, n)) {
                            1.0
                        } else {
                            0.0
                        }
                    }
                    None => edge.on.next_value(),
                };
                *level = gain * switched;
            }
            for channel in 0..channels {
                let into = &mut scratch.channel_mut(channel)[..frames];
                if edge.pre {
                    let from = &work.channel(channel)[..frames];
                    for (n, (into, sample)) in into.iter_mut().zip(from).enumerate() {
                        *into = sample * heard[n] * levels[n];
                    }
                } else {
                    let from = &post.channel(channel)[..frames];
                    for (n, (into, sample)) in into.iter_mut().zip(from).enumerate() {
                        *into = sample * levels[n];
                    }
                }
            }
            edge.delay.process(scratch, channels, frames);
            let into = &mut inputs[edge.dest];
            if edge.pan.moves() {
                for n in 0..frames {
                    let pan = &mut edge.pan;
                    let mut at = pan.base;
                    at.x = match pan.lanes[0] {
                        Some(lane) => value(lane, n),
                        None => pan.x.next_value(),
                    };
                    at.y = match pan.lanes[1] {
                        Some(lane) => value(lane, n),
                        None => pan.y.next_value(),
                    };
                    at.divergence = match pan.lanes[2] {
                        Some(lane) => value(lane, n) / 100.0,
                        None => pan.divergence.next_value(),
                    };
                    if at != edge.pan.last {
                        edge.matrix = panned(edge.from, edge.to, &at);
                        edge.pan.last = at;
                    }
                    edge.matrix.mix_sample(scratch, into, n);
                }
                if !edge.pan.moves() {
                    edge.pan.last = edge.pan.now();
                    edge.matrix = panned(edge.from, edge.to, &edge.pan.last);
                }
            } else {
                edge.matrix.mix(scratch, into, frames);
            }
            edge.stale = if quiet {
                edge.stale.saturating_sub(frames)
            } else {
                edge.delay.length()
            };
        }
    }

    /// Back to silence, its ramps ended, its nodes reset and lined up with the sample of the
    /// timeline `origin` at which the run starts at the input of the graph.
    pub(crate) fn reset(&mut self, origin: i64) {
        for smoothed in [
            &mut self.trim,
            &mut self.sign,
            &mut self.fader,
            &mut self.mute,
            &mut self.audible,
        ] {
            smoothed.finish();
        }
        for stage in &mut self.stages {
            stage.side.reset();
            stage.node.reset();
            stage.processed = 0;
            // The grid of a node is that of the timeline: two's complement keeps the remainder
            // of any power of two for a run that starts before the timeline does.
            stage
                .node
                .align(origin.wrapping_sub(stage.offset).cast_unsigned());
        }
        for edge in &mut self.edges {
            for smoothed in [
                &mut edge.level,
                &mut edge.on,
                &mut edge.pan.x,
                &mut edge.pan.y,
                &mut edge.pan.divergence,
            ] {
                smoothed.finish();
            }
            edge.pan.last = edge.pan.now();
            edge.matrix = panned(edge.from, edge.to, &edge.pan.last);
            edge.delay.reset();
            edge.stale = 0;
        }
        self.pre_tap.reset();
        self.post_tap.reset();
    }
}
