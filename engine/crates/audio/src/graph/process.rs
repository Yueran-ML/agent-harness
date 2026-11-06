//! What the graph does on the thread of the mix: [`Graph::process`], [`Graph::start`],
//! [`Graph::set`], [`Graph::glide`], [`Graph::live`] and [`Graph::write_device`]. Nothing here locks, allocates or waits
//! (convention 3; `tests/rt_audit.rs`, `tests/no_alloc.rs`).

use crate::db_to_gain;
use crate::feed::{ClipFeed, FeedError};
use crate::node::Node;

use super::route::balance;
use super::{ClipParam, Graph, StripParam, Target};

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
        // Where the clips are on the timeline at a speed of 1 (T18's review: a warp of speed 1
        // is a shift, under which the clips and lanes read the timeline as they are built).
        let timeline = start.saturating_add(self.shift());
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
                    (timeline, frames, rate),
                    &mut self.scratch,
                    &mut self.inputs[strip],
                );
                if let (Err(error), Ok(())) = (mixed, &result) {
                    result = Err(error);
                }
            }
            let this = &mut self.strips[strip];
            this.load(&self.inputs[strip], (start, self.warp), rate, frames);
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
        if let (Some(loudness), Some(monitor)) = (&mut self.loudness, self.monitor) {
            loudness.take(&self.outputs[monitor].buffer, frames);
        }
        self.position = start.saturating_add(i64::try_from(frames).unwrap_or(0));
        self.frames = frames;
        if self.metering && frames > 0 { self.fx_blocks = self.fx_blocks.wrapping_add(1); self.fx_state.write((true, self.fx_blocks)); }
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
        self.start_within(at, u32::MAX, feed)
    }

    /// [`Graph::start`] with at most `most` samples of preroll: a graph swapped in as the sound
    /// plays (T5), which a crossfade brings in, needs not settle as long as a run that starts
    /// does.
    ///
    /// # Errors
    ///
    /// As [`Graph::process`], for the samples thrown away.
    pub fn start_within(
        &mut self,
        at: i64,
        most: u32,
        feed: &mut dyn ClipFeed,
    ) -> Result<(), FeedError> {
        // However short, as long again as the effects of a clip are late (T4c's review): they
        // start a lead after the run does, and are silent for as long.
        let preroll = self.preroll_at(at).min(most.saturating_add(self.lead));
        let begin = at.saturating_sub(i64::from(preroll));
        // What is thrown away is not metered (`taps.rs`).
        let metering = self.metering;
        self.set_metering(false);
        // Reset and the stretched clips brought to `begin` (T13): on this path too.
        let mut result = self.begin(begin, feed);
        let mut left = u64::from(preroll) + u64::from(self.latency);
        while left > 0 {
            let frames = usize::try_from(left)
                .unwrap_or(usize::MAX)
                .min(self.max_block);
            if let (Err(error), Ok(())) = (self.process(feed, frames), &result) {
                result = Err(error);
            }
            left -= u64::try_from(frames).unwrap_or(left);
        }
        self.set_metering(metering);
        self.frames = 0;
        result
    }

    /// The first step of every run that starts at input sample `begin` (`Graph::start`, and any
    /// other way in, such as a graph swapped in by the thread that builds them): back to silence
    /// there ([`Graph::reset`]), then every stretched clip under way there brought there
    /// (`ClipChain::prime`, T13), so that the work of catching its stretch up is done here, before
    /// the sound plays, and never in a block of the thread of the mix.
    ///
    /// # Errors
    ///
    /// The feed could not give the sound of a clip while it was caught up.
    pub(crate) fn begin(&mut self, begin: i64, feed: &mut dyn ClipFeed) -> Result<(), FeedError> {
        self.reset(begin);
        let mut result = Ok(());
        let begin = begin.saturating_add(self.shift());
        for (index, clip) in self.clips.iter_mut().enumerate() {
            let primed = clip.prime(index, feed, begin, &mut self.scratch);
            if let (Err(error), Ok(())) = (primed, &result) {
                result = Err(error);
            }
        }
        result
    }

    /// Back to silence at input sample `begin`, with nothing processed.
    pub(crate) fn reset(&mut self, begin: i64) {
        self.position = begin;
        self.frames = 0;
        let begin = begin.saturating_add(self.shift());
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
        if let Some(loudness) = &mut self.loudness {
            loudness.reset();
        }
    }

    /// Sets `control` to `value`, in the units of the model (decibels, −1 to 1, percent, and
    /// 0 or 1 for a switch, on at 0.5 and above): ramped over 10 ms from the next sample. A lane
    /// of the same setting, where there is one, still takes its place. A control of another
    /// graph, and a value that is not a number (NaN, ±∞), change nothing.
    pub fn set(&mut self, control: super::Control, value: f32) {
        self.change(control, value, None);
    }

    /// Sets `control` to `value` as [`Graph::set`] does, but on a straight line over `samples`
    /// samples (at once for 0): a setting that no lane is of, let go after it was touched while
    /// automation is recorded, gliding back to where it is set (Snap, p. 3905; T16). A gain glides
    /// straight in the gain, a pan in its value; a switch, and a parameter of a node (Dynamics,
    /// the equalizer, an insert), ramp over 10 ms as [`Graph::set`] ramps them.
    pub fn glide(&mut self, control: super::Control, value: f32, samples: u32) {
        self.change(control, value, Some(samples));
    }

    /// Holds `live` in place of the lane `key` of the graph, at every place that reads it, from
    /// the next sample (T16): a value of a control that is touched while automation is recorded,
    /// an offset of a fader trimmed, or the lane let take its place back. A key of another graph
    /// changes nothing. A run started again ([`Graph::start`]) holds nothing.
    pub fn live(&mut self, key: super::LaneKey, live: super::Live) {
        if key.0 != self.id {
            return;
        }
        for strip in &mut self.strips {
            for lane in strip.lanes.iter_mut().filter(|lane| lane.key == key.1) {
                lane.held.take(live, lane.lane.kind);
            }
        }
    }

    /// [`Graph::set`] (`glide` none) or [`Graph::glide`].
    fn change(&mut self, control: super::Control, value: f32, glide: Option<u32>) {
        let switch = |value: f32| if value >= 0.5 { 1.0 } else { 0.0 };
        if control.0 != self.id || !value.is_finite() {
            return;
        }
        // A number goes to its value over 10 ms, or over the glide.
        let to = |smoothed: &mut crate::rt::Smoothed, value: f32| match glide {
            Some(samples) => smoothed.glide(value, samples),
            None => smoothed.set(value),
        };
        match control.1 {
            Target::VcaFader(vca) => {
                if let Some(vca) = self.vcas.get_mut(vca) {
                    to(&mut vca.fader, db_to_gain(value));
                }
            }
            Target::VcaMute(vca) => {
                if let Some(vca) = self.vcas.get_mut(vca) {
                    vca.mute.set(1.0 - switch(value));
                }
            }
            Target::Clip(clip, param) => {
                let Some(level) = self.clips.get_mut(clip).map(|clip| &mut clip.level) else {
                    return;
                };
                match (param, level.gain_db) {
                    (ClipParam::Gain, Some(built)) => {
                        to(
                            &mut level.scale,
                            db_to_gain(value) / db_to_gain(built).max(f32::MIN_POSITIVE),
                        );
                        level.scaled = true;
                    }
                    (ClipParam::Pan, _) if level.fixed_pan => {
                        let sides = balance(value.clamp(-1.0, 1.0));
                        to(&mut level.sides[0], sides[0]);
                        to(&mut level.sides[1], sides[1]);
                        level.panned = true;
                    }
                    _ => {}
                }
            }
            Target::Strip(strip, param) => {
                let Some(strip) = self.strips.get_mut(strip) else {
                    return;
                };
                match param {
                    StripParam::Trim => to(&mut strip.trim, db_to_gain(value)),
                    StripParam::Invert => strip.sign.set(1.0 - 2.0 * switch(value)),
                    StripParam::Fader => to(&mut strip.fader, db_to_gain(value)),
                    StripParam::Mute => strip.mute.set(1.0 - switch(value)),
                    StripParam::Audible => strip.audible.set(switch(value)),
                    StripParam::PanX | StripParam::PanY | StripParam::PanDivergence => {
                        for edge in strip.edges.iter_mut().filter(|edge| !edge.send) {
                            match param {
                                StripParam::PanX => to(&mut edge.pan.x, value),
                                StripParam::PanY => to(&mut edge.pan.y, value),
                                _ => to(&mut edge.pan.divergence, value / 100.0),
                            }
                        }
                    }
                    StripParam::Level(edge) => {
                        if let Some(edge) = strip.edges.get_mut(edge) {
                            to(&mut edge.level, db_to_gain(value));
                        }
                    }
                    StripParam::On(edge) => {
                        if let Some(edge) = strip.edges.get_mut(edge) {
                            edge.on.set(switch(value));
                        }
                    }
                    StripParam::EdgePanX(edge) => {
                        if let Some(edge) = strip.edges.get_mut(edge) {
                            to(&mut edge.pan.x, value);
                        }
                    }
                    StripParam::EdgePanY(edge) => {
                        if let Some(edge) = strip.edges.get_mut(edge) {
                            to(&mut edge.pan.y, value);
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
