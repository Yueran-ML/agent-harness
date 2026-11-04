//! The compensation of delays (ADC, p. 3706; plan T4b): a path's latency is the sum of the
//! latencies of its nodes, and the paths that meet at a bus are lined up by delaying the faster
//! ones to the slowest, on each connection that feeds it (an output or a send). A track whose ADC
//! is off is not delayed: it arrives early by what it lacks. What is left of the latency at a
//! main output is the latency of the graph, `L`, taken off where the sound leaves it: the sound
//! of sample `n` of the timeline is always output sample `n` (see `Graph::start`).
//!
//! The latencies are worked out when the graph is built (`build.rs`); here is the line that
//! delays, which runs on the thread of the mix.

use crate::node::Planar;

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
