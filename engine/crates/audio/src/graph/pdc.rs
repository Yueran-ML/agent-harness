//! The compensation of delays (ADC, p. 3706; plan T4b): a path's latency is the sum of the
//! latencies of its nodes, and the paths that meet at a bus are lined up by delaying the faster
//! ones to the slowest, on each connection that feeds it (an output or a send). A track whose ADC
//! is off is not delayed: it arrives early by what it lacks. What is left of the latency at a
//! main output is the latency of the graph, `L`, taken off where the sound leaves it: the sound
//! of sample `n` of the timeline is always output sample `n` (see `Graph::start`).
//!
//! The latencies are worked out when the graph is built (`build.rs`); here is the line that
//! delays, which runs on the thread of the mix.

use crate::node::{Block, Node, ParamId, Planar, Spec, Unsupported};

/// A delay of a fixed count of samples on each of some channels.
#[derive(Debug)]
pub(crate) struct DelayLine {
    /// A line of `length` samples for each channel, one after another.
    pub(crate) lines: Box<[f32]>,
    pub(crate) length: usize,
    pub(crate) at: usize,
}

impl DelayLine {
    /// The samples it delays by.
    pub(crate) fn length(&self) -> usize {
        self.length
    }

    /// Delays the first `frames` samples of the first `channels` channels of `buffer` where they
    /// are.
    pub(crate) fn process(&mut self, buffer: &mut Planar, channels: usize, frames: usize) {
        if self.length == 0 {
            return;
        }
        for (channel, line) in self
            .lines
            .chunks_exact_mut(self.length)
            .take(channels)
            .enumerate()
        {
            let mut at = self.at;
            for sample in &mut buffer.channel_mut(channel)[..frames] {
                std::mem::swap(sample, &mut line[at]);
                at += 1;
                if at == self.length {
                    at = 0;
                }
            }
        }
        self.at = (self.at + frames) % self.length;
    }

    /// Back to silence.
    pub(crate) fn reset(&mut self) {
        self.lines.fill(0.0);
        self.at = 0;
    }
}

/// A node of no sound of its own that delays by a fixed count of samples: a bypassed insert of a
/// node that has no switch of its own, which "still delays its sound as it does when it is on"
/// (the model's `Insert`); and the delay that lines a strip up with its sidechain where the
/// sidechain is the later (T4b's review, P3-6).
#[derive(Debug)]
pub(crate) struct Delayer {
    pub(crate) line: DelayLine,
}

impl Delayer {
    /// A delay of `length` samples, of no channels until it is prepared.
    pub(crate) fn new(length: usize) -> Self {
        Delayer {
            line: DelayLine {
                lines: Box::default(),
                length,
                at: 0,
            },
        }
    }
}

impl Node for Delayer {
    fn prepare(&mut self, spec: &Spec) -> Result<(), Unsupported> {
        if spec.layout != spec.output {
            return Err(Unsupported::from(spec));
        }
        let length = self.line.length;
        self.line.lines = std::iter::repeat_n(0.0, spec.inputs() * length).collect(); // rt-ok: when the graph is built
        self.line.at = 0;
        Ok(())
    }

    fn process(&mut self, ctx: &Block, input: &[&[f32]], output: &mut [&mut [f32]]) {
        for (output, input) in output.iter_mut().zip(input) {
            output.copy_from_slice(input);
        }
        let length = self.line.length;
        if length == 0 {
            return;
        }
        for (output, line) in output
            .iter_mut()
            .zip(self.line.lines.chunks_exact_mut(length))
        {
            let mut at = self.line.at;
            for sample in output.iter_mut() {
                std::mem::swap(sample, &mut line[at]);
                at += 1;
                if at == length {
                    at = 0;
                }
            }
        }
        self.line.at = (self.line.at + ctx.frames) % length;
    }

    fn latency(&self) -> u32 {
        u32::try_from(self.line.length).unwrap_or(u32::MAX)
    }

    fn settle(&self) -> u32 {
        0
    }

    fn reset(&mut self) {
        self.line.reset();
    }

    fn set(&mut self, _: ParamId, _: f32) {}
}
