//! What the graph does on the thread of the mix: [`Graph::process`], [`Graph::start`],
//! [`Graph::set`] and [`Graph::write_device`]. Nothing here locks, allocates or waits
//! (convention 3; `tests/rt_audit.rs`, `tests/no_alloc.rs`).

use crate::db_to_gain;
use crate::feed::{ClipFeed, FeedError};
use crate::node::Node;

use super::{Graph, StripParam, Target};

impl Graph {
    /// Processes the next `frames` samples (at most [`Graph::max_block`]; more are cut to it):
    /// reads the clips that sound in them through `feed`, runs every strip in its order, and
    /// leaves each output's sound in [`Graph::output`]. The samples out are those of the timeline
    /// from [`Graph::at`] on.
    ///
    /// # Errors
    ///
    /// The feed could not give the sound of a clip: the block is processed all the same, that
    /// clip silent in it, and the first such clip is told.
    pub fn process(&mut self, feed: &mut dyn ClipFeed, frames: usize) -> Result<(), FeedError> {
        let frames = frames.min(self.max_block);
        let start = self.position;
        let rate = self.rate;
        let mut result = Ok(());
        for input in &mut self.inputs {
            for channel in 0..input.channels() {
                input.channel_mut(channel)[..frames].fill(0.0);
            }
        }
        for vca in &mut self.vcas {
            vca.advance(frames);
        }
        for at in 0..self.order.len() {
            let strip = self.order[at];
            for clip in self.clips_of[strip].clone() {
                let mixed = self.clips[clip].mix(
                    clip,
                    feed,
                    (start, frames, rate),
                    &mut self.scratch,
                    &mut self.inputs[strip],
                );
                if let (Err(error), Ok(())) = (mixed, &result) {
                    result = Err(error);
                }
            }
            let this = &mut self.strips[strip];
            this.load(&self.inputs[strip], start, rate, frames);
            this.run(rate, frames, &self.pres, &self.vcas);
            if self.keys[strip] {
                for channel in 0..this.channels() {
                    self.pres[strip].channel_mut(channel)[..frames]
                        .copy_from_slice(&this.work.channel(channel)[..frames]);
                }
            }
            this.route(frames, &mut self.edge, &mut self.inputs);
        }
        for out in &mut self.outputs {
            let post = &self.strips[out.strip].post;
            let channels = out.buffer.channels();
            for channel in 0..channels {
                out.buffer.channel_mut(channel)[..frames]
                    .copy_from_slice(&post.channel(channel)[..frames]);
            }
            out.delay.process(&mut out.buffer, channels, frames);
        }
        self.position = start.saturating_add(i64::try_from(frames).unwrap_or(0));
        self.frames = frames;
        result
    }

    /// Starts a run at sample `at` of the timeline: every node, delay and ramp back to silence
    /// and lined up with the timeline, then [`Graph::preroll`] samples before `at` and the
    /// [`Graph::latency`] after it processed and thrown away, so that the next block out starts
    /// at `at` and sounds as a run from earlier would (the plan's "预滚"; T6).
    ///
    /// # Errors
    ///
    /// As [`Graph::process`], for the samples thrown away.
    pub fn start(&mut self, at: i64, feed: &mut dyn ClipFeed) -> Result<(), FeedError> {
        let begin = at.saturating_sub(i64::from(self.preroll()));
        self.reset(begin);
        let mut left = u64::from(self.preroll()) + u64::from(self.latency);
        let mut result = Ok(());
        // The stretched clips under way at `begin` brought there now, not on the thread of the mix.
        for (index, clip) in self.clips.iter_mut().enumerate() {
            let primed = clip.prime(index, feed, begin, &mut self.scratch);
            if let (Err(error), Ok(())) = (primed, &result) {
                result = Err(error);
            }
        }
        while left > 0 {
            let frames = usize::try_from(left)
                .unwrap_or(usize::MAX)
                .min(self.max_block);
            if let (Err(error), Ok(())) = (self.process(feed, frames), &result) {
                result = Err(error);
            }
            left -= u64::try_from(frames).unwrap_or(left);
        }
        self.frames = 0;
        result
    }

    /// Back to silence at input sample `begin`, with nothing processed.
    pub(crate) fn reset(&mut self, begin: i64) {
        self.position = begin;
        self.frames = 0;
        for strip in &mut self.strips {
            strip.reset(begin);
        }
        for clip in &mut self.clips {
            clip.reset();
            if let Some(eq) = &mut clip.eq {
                eq.align(begin.cast_unsigned());
            }
        }
        self.scratch.reset();
        for vca in &mut self.vcas {
            vca.fader.finish();
            vca.mute.finish();
        }
        for out in &mut self.outputs {
            out.delay.reset();
        }
    }

    /// Sets `control` to `value`, in the units of the model (decibels, −1 to 1, percent, and
    /// 0 or 1 for a switch, on at 0.5 and above): ramped over 10 ms from the next sample. A lane
    /// of the same setting, where there is one, still takes its place. A control of another
    /// graph, and a value that is not a number (NaN, ±∞), change nothing.
    pub fn set(&mut self, control: super::Control, value: f32) {
        let switch = |value: f32| if value >= 0.5 { 1.0 } else { 0.0 };
        if control.0 != self.id || !value.is_finite() {
            return;
        }
        match control.1 {
            Target::VcaFader(vca) => {
                if let Some(vca) = self.vcas.get_mut(vca) {
                    vca.fader.set(db_to_gain(value));
                }
            }
            Target::VcaMute(vca) => {
                if let Some(vca) = self.vcas.get_mut(vca) {
                    vca.mute.set(1.0 - switch(value));
                }
            }
            Target::Strip(strip, param) => {
                let Some(strip) = self.strips.get_mut(strip) else {
                    return;
                };
                match param {
                    StripParam::Trim => strip.trim.set(db_to_gain(value)),
                    StripParam::Invert => strip.sign.set(1.0 - 2.0 * switch(value)),
                    StripParam::Fader => strip.fader.set(db_to_gain(value)),
                    StripParam::Mute => strip.mute.set(1.0 - switch(value)),
                    StripParam::Audible => strip.audible.set(switch(value)),
                    StripParam::PanX | StripParam::PanY | StripParam::PanDivergence => {
                        for edge in strip.edges.iter_mut().filter(|edge| !edge.send) {
                            match param {
                                StripParam::PanX => edge.pan.x.set(value),
                                StripParam::PanY => edge.pan.y.set(value),
                                _ => edge.pan.divergence.set(value / 100.0),
                            }
                        }
                    }
                    StripParam::Level(edge) => {
                        if let Some(edge) = strip.edges.get_mut(edge) {
                            edge.level.set(db_to_gain(value));
                        }
                    }
                    StripParam::On(edge) => {
                        if let Some(edge) = strip.edges.get_mut(edge) {
                            edge.on.set(switch(value));
                        }
                    }
                    StripParam::EdgePanX(edge) => {
                        if let Some(edge) = strip.edges.get_mut(edge) {
                            edge.pan.x.set(value);
                        }
                    }
                    StripParam::EdgePanY(edge) => {
                        if let Some(edge) = strip.edges.get_mut(edge) {
                            edge.pan.y.set(value);
                        }
                    }
                    StripParam::Node(stage, param) => {
                        if let Some(stage) = strip.stages.get_mut(stage) {
                            stage.node.set(param, value);
                        }
                    }
                }
            }
        }
    }

    /// Writes the first `frames` samples of output `output` of the last block into `into`,
    /// interleaved, each clipped to −1…1: the copy for a device or a fixed-point encoder, the
    /// only place where the graph's sound is clipped (plan T4b, "削波只在这里做").
    pub fn write_device(&self, output: usize, frames: usize, into: &mut [f32]) {
        let buffer = &self.outputs[output].buffer;
        let channels = buffer.channels();
        for (n, frame) in into.chunks_exact_mut(channels).take(frames).enumerate() {
            for (channel, sample) in frame.iter_mut().enumerate() {
                *sample = buffer.channel(channel)[n].clamp(-1.0, 1.0);
            }
        }
    }
}
