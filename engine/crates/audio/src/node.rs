//! The interface of a node of the graph: an effect, a filter, a delay, anything that takes blocks
//! of sound in and gives blocks of sound out.
//!
//! The graph runs every node once for each block, in order, the blocks one after the other (never
//! the same samples twice, never back), and the output of a node must not depend on where the
//! sound is cut into blocks: the same signal processed whole and processed in blocks of any
//! lengths gives the same output to the bit (`testing::chunk_invariance`).
//!
//! # The two times of a node
//!
//! [`Node::prepare`] is called when the graph is built, off the thread of the mix: it may
//! allocate, and it takes all the memory that the node will need. [`Node::process`],
//! [`Node::reset`] and [`Node::set`] are called on the thread of the mix: they must not lock,
//! allocate or free memory, do I/O, log, or wait (convention 3 of the plan).
//!
//! # Parameters
//!
//! A parameter is one `f32`, set by [`Node::set`] and ramped over 10 ms. Parameters are kept
//! through [`Node::reset`] and through [`Node::prepare`] again: a reset clears the sound held
//! (filters, delay lines, envelopes) and ends the ramps under way at their targets, and never puts
//! a parameter back to its default. What is not one number (the noise spectrum that Noise
//! Reduction learns, a table) and what would change the latency of the node (Noise Reduction
//! switched between its spectral and its voice mode) is not a parameter: it is a change of the
//! structure, and the graph is built again with it and swapped in (T5), never `set`.
//!
//! # Time
//!
//! A node knows no timeline. [`Block::start`] counts the samples that the node has processed
//! since it was prepared or reset; it moves on by each block and goes back to 0 on a reset only.
//! A node must not work out from it where the timeline is, or that the playhead has jumped: a seek
//! is a reset. What depends on the timeline (the values of automation, the points where the
//! stretch starts again) is worked out by the graph and handed in.
//!
//! # Denormals
//!
//! The floating-point unit is left as it is: nothing sets flush-to-zero or denormals-are-zero
//! (MXCSR), since preview and export must give the same bits on threads set up differently. A
//! node with feedback (a filter, an envelope, a reverb) flushes its state to 0 explicitly when it
//! falls below 1e-30 in size ([`crate::rt::flush`]), so that a tail into silence ends in zeros and
//! not in slow denormal arithmetic.
//!
//! # Settling along a path
//!
//! [`Node::settle`] is the node's own. A path of nodes settles in the sum of the settling of its
//! nodes, and its latency is the sum of their latencies (T4b, T6).

use crate::constants::MAX_CHANNELS;

/// A parameter of a node, by the node's own numbering.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ParamId(pub u32);

/// What a channel of a [`Layout`] carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Role {
    /// The one channel of mono.
    Mono,
    /// Left.
    Left,
    /// Right.
    Right,
    /// Centre.
    Centre,
    /// The low-frequency effects.
    Lfe,
    /// The one surround of LCRS.
    Surround,
    /// Left surround: the surround of 5.0 and 5.1, the side surround of 7.0 and 7.1.
    LeftSurround,
    /// Right surround, as [`Role::LeftSurround`].
    RightSurround,
    /// Left rear surround (7.0, 7.1).
    LeftRear,
    /// Right rear surround (7.0, 7.1).
    RightRear,
    /// A channel of an Adaptive track, by its number from 0: no place of its own (p. 3724).
    Discrete(u8),
}

/// The channels of a track or a bus, in the order they are kept (or none, for the input of a
/// source): the formats of the buses (p.
/// 3721; Atmos left out) and the Adaptive track of 1 to 24 channels (p. 3724). Surround is in
/// the SMPTE order (L R C LFE Ls Rs, then Lrs Rrs); Film order is turned into it when read (C-F8).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Layout {
    /// No channel: the input of a node that makes its own sound.
    Empty,
    /// One channel.
    Mono,
    /// L R.
    Stereo,
    /// L R C (3.0).
    Lcr,
    /// L R C S (4.0).
    Lcrs,
    /// L R C Ls Rs.
    Surround50,
    /// L R C LFE Ls Rs.
    Surround51,
    /// L R C Ls Rs Lrs Rrs.
    Surround70,
    /// L R C LFE Ls Rs Lrs Rrs.
    Surround71,
    /// 1 to 24 channels of their own.
    Adaptive(u8),
}

impl Layout {
    /// The channels.
    #[must_use]
    pub fn channels(self) -> usize {
        self.roles().len()
    }

    /// What each channel carries, in order.
    #[must_use]
    pub fn roles(self) -> &'static [Role] {
        use Role::{
            Centre, Discrete, Left, LeftRear, LeftSurround, Lfe, Mono, Right, RightRear,
            RightSurround, Surround,
        };
        const ADAPTIVE: [Role; MAX_CHANNELS] = {
            let mut roles = [Discrete(0); MAX_CHANNELS];
            let mut channel: u8 = 0;
            while (channel as usize) < MAX_CHANNELS {
                roles[channel as usize] = Discrete(channel);
                channel += 1;
            }
            roles
        };
        match self {
            Layout::Empty => &[],
            Layout::Mono => &[Mono],
            Layout::Stereo => &[Left, Right],
            Layout::Lcr => &[Left, Right, Centre],
            Layout::Lcrs => &[Left, Right, Centre, Surround],
            Layout::Surround50 => &[Left, Right, Centre, LeftSurround, RightSurround],
            Layout::Surround51 => &[Left, Right, Centre, Lfe, LeftSurround, RightSurround],
            Layout::Surround70 => &[
                Left,
                Right,
                Centre,
                LeftSurround,
                RightSurround,
                LeftRear,
                RightRear,
            ],
            Layout::Surround71 => &[
                Left,
                Right,
                Centre,
                Lfe,
                LeftSurround,
                RightSurround,
                LeftRear,
                RightRear,
            ],
            Layout::Adaptive(count) => &ADAPTIVE[..usize::from(count).clamp(1, MAX_CHANNELS)],
        }
    }

    /// What `channel` carries.
    ///
    /// # Panics
    ///
    /// When there is no such channel.
    #[must_use]
    pub fn role(self, channel: usize) -> Role {
        self.roles()[channel]
    }

    /// The channel of the LFE, if there is one: the channel that loudness (BS.1770) leaves out
    /// and that linked dynamics do not hear.
    #[must_use]
    pub fn lfe(self) -> Option<usize> {
        self.roles().iter().position(|&role| role == Role::Lfe)
    }

    /// `channels` of their own: an Adaptive layout (clamped to 1 to 24).
    #[must_use]
    pub fn discrete(channels: usize) -> Self {
        #[allow(clippy::cast_possible_truncation, reason = "clamped to 24")]
        Layout::Adaptive(channels.clamp(1, MAX_CHANNELS) as u8)
    }
}

/// What a node is prepared for: the rate of the timeline, the longest block, the layout of the
/// channels in and out, and the channels of the sidechain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Spec {
    /// Samples a second.
    pub rate: u32,
    /// The most samples of a block.
    pub max_block: usize,
    /// The channels of the input.
    pub layout: Layout,
    /// The channels of the output: the input's but for nodes that change the format (a panner,
    /// a downmix).
    pub output: Layout,
    /// The channels of the sidechain (Dynamics, T8; Ducker, T11), 0 when there is none.
    pub sidechain: usize,
}

impl Spec {
    /// A node of `layout` in and out, with no sidechain.
    #[must_use]
    pub fn new(rate: u32, max_block: usize, layout: Layout) -> Self {
        Spec {
            rate,
            max_block,
            layout,
            output: layout,
            sidechain: 0,
        }
    }

    /// The same, with `output` out.
    #[must_use]
    pub fn with_output(self, output: Layout) -> Self {
        Spec { output, ..self }
    }

    /// The same, with a sidechain of `channels`.
    #[must_use]
    pub fn with_sidechain(self, channels: usize) -> Self {
        Spec {
            sidechain: channels,
            ..self
        }
    }

    /// The same, with blocks of at most `max_block` samples.
    #[must_use]
    pub fn with_max_block(self, max_block: usize) -> Self {
        Spec { max_block, ..self }
    }

    /// The channels in.
    #[must_use]
    pub fn inputs(&self) -> usize {
        self.layout.channels()
    }

    /// The channels out.
    #[must_use]
    pub fn outputs(&self) -> usize {
        self.output.channels()
    }
}

/// A node that cannot work with the channels of a [`Spec`] (Stereo Width on a mono track, say).
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("this node does not take {layout:?} in, {output:?} out and {sidechain} of sidechain")]
pub struct Unsupported {
    /// The channels in that were asked for.
    pub layout: Layout,
    /// The channels out.
    pub output: Layout,
    /// The channels of the sidechain.
    pub sidechain: usize,
}

impl From<&Spec> for Unsupported {
    fn from(spec: &Spec) -> Self {
        Unsupported {
            layout: spec.layout,
            output: spec.output,
            sidechain: spec.sidechain,
        }
    }
}

/// The values of one automated parameter through a block, one for each sample, in the units of
/// [`Node::set`] (track automation, T14).
#[derive(Clone, Copy, Debug)]
pub struct Automated<'a> {
    /// The parameter.
    pub param: ParamId,
    /// Its value at each sample of the block.
    pub values: &'a [f32],
}

/// The automated parameters of a node through a block, worked out by the graph from the lanes
/// at the samples of the timeline that the block plays. An automated parameter takes the values
/// here, sample by sample, in place of the value last [`Node::set`]; the value set still goes on
/// being smoothed underneath, and is taken again when the lane goes.
#[derive(Clone, Copy, Debug, Default)]
pub struct Automation<'a> {
    lanes: &'a [Automated<'a>],
}

impl<'a> Automation<'a> {
    /// No parameter automated.
    pub const NONE: Automation<'static> = Automation { lanes: &[] };

    /// The parameters of `lanes`; a parameter twice takes the first.
    #[must_use]
    pub fn new(lanes: &'a [Automated<'a>]) -> Self {
        Automation { lanes }
    }

    /// The values of `param` through the block, if it is automated.
    #[must_use]
    pub fn get(&self, param: ParamId) -> Option<&'a [f32]> {
        self.lanes
            .iter()
            .find(|lane| lane.param == param)
            .map(|lane| lane.values)
    }

    /// No parameter is automated.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.lanes.is_empty()
    }
}

/// What a node is told about the block it processes.
#[derive(Clone, Copy, Debug)]
pub struct Block<'a> {
    /// The samples the node has processed before this block, since it was prepared or reset: not
    /// a place on the timeline (see the module's "Time").
    pub start: u64,
    /// Samples a second: the rate of the [`Spec`].
    pub rate: u32,
    /// The samples of the block: the length of every channel in and out. It may be 0: a block of
    /// nothing, which changes nothing.
    pub frames: usize,
    /// The sidechain, a channel for each of the [`Spec`]'s, each `frames` long; empty when there
    /// is none.
    pub sidechain: &'a [&'a [f32]],
    /// The automated parameters.
    pub automation: Automation<'a>,
}

impl Block<'static> {
    /// A block of `frames` samples after `start` processed, with no sidechain and no automation.
    #[must_use]
    pub fn new(start: u64, rate: u32, frames: usize) -> Self {
        Block {
            start,
            rate,
            frames,
            sidechain: &[],
            automation: Automation::NONE,
        }
    }
}

impl<'a> Block<'a> {
    /// The same, with `sidechain`.
    #[must_use]
    pub fn with_sidechain<'b>(self, sidechain: &'b [&'b [f32]]) -> Block<'b>
    where
        'a: 'b,
    {
        Block { sidechain, ..self }
    }

    /// The same, with `automation`.
    #[must_use]
    pub fn with_automation<'b>(self, automation: Automation<'b>) -> Block<'b>
    where
        'a: 'b,
    {
        Block { automation, ..self }
    }
}

/// A node of the graph.
pub trait Node: Send {
    /// Makes the node ready for `spec`, taking all the memory it will need, and leaves it as
    /// [`Node::reset`] does, its parameters kept. Called off the thread of the mix, before any
    /// other call, and again whenever the spec changes.
    ///
    /// # Errors
    ///
    /// [`Unsupported`] when the node does not work with these channels.
    fn prepare(&mut self, spec: &Spec) -> Result<(), Unsupported>;

    /// Processes a block: `input` and `output` are planar (a slice for each channel), every one
    /// `ctx.frames` long, as many as the spec's inputs and outputs. A block of 0 samples changes
    /// nothing. Does not allocate.
    fn process(&mut self, ctx: &Block, input: &[&[f32]], output: &mut [&mut [f32]]);

    /// The delay of the node, in samples: the output at sample `n` answers the input at sample
    /// `n − latency`. The graph lines the paths up by it (T4b). Fixed from [`Node::prepare`] on:
    /// [`Node::set`] never changes it.
    fn latency(&self) -> u32;

    /// How many samples after a reset the output differs by less than −90 dBFS
    /// ([`crate::constants::SETTLE_FLOOR_DBFS`]) from the output of the same node fed the same
    /// sound from earlier. A preroll this long makes a start in the middle sound as a start from
    /// the beginning (T6); along a path, the sum of its nodes'.
    fn settle(&self) -> u32;

    /// Back to silence: the sound held is cleared and the ramps under way end at their targets;
    /// the parameters are kept (on a seek, or when the graph is swapped). Does not allocate.
    fn reset(&mut self);

    /// Sets a parameter, one number, while the sound plays: the change is smoothed (10 ms,
    /// [`crate::constants::SMOOTHING`]) from the next sample processed. Does not change the
    /// latency, does not allocate; an unknown parameter is ignored.
    fn set(&mut self, param: ParamId, value: f32);
}

/// Sound in planar layout, a channel after another, each of a fixed length, taken once: the
/// buffers between the nodes. [`Planar::read`] and [`Planar::write`] give the slices that
/// [`Node::process`] takes, without allocating; [`Planar::read_range`] and
/// [`Planar::write_range`] give a part of a block (a block cut where a clip starts or ends).
#[derive(Clone, Debug, PartialEq)]
pub struct Planar {
    data: Vec<f32>,
    channels: usize,
    frames: usize,
}

impl Planar {
    /// Silence of `channels` of `frames` samples each.
    ///
    /// # Panics
    ///
    /// When `channels` is more than [`MAX_CHANNELS`].
    #[must_use]
    pub fn new(channels: usize, frames: usize) -> Self {
        assert!(
            channels <= MAX_CHANNELS,
            "{channels} channels, more than {MAX_CHANNELS}"
        );
        Planar {
            data: vec![0.0; channels * frames],
            channels,
            frames,
        }
    }

    /// The channels.
    #[must_use]
    pub fn channels(&self) -> usize {
        self.channels
    }

    /// The samples of each channel.
    #[must_use]
    pub fn frames(&self) -> usize {
        self.frames
    }

    /// The whole of `channel`.
    #[must_use]
    pub fn channel(&self, channel: usize) -> &[f32] {
        &self.data[channel * self.frames..(channel + 1) * self.frames]
    }

    /// The whole of `channel`, to write.
    #[must_use]
    pub fn channel_mut(&mut self, channel: usize) -> &mut [f32] {
        &mut self.data[channel * self.frames..(channel + 1) * self.frames]
    }

    /// Silence everywhere.
    pub fn silence(&mut self) {
        self.data.fill(0.0);
    }

    /// Calls `f` with the first `frames` samples of each channel.
    ///
    /// # Panics
    ///
    /// When `frames` is more than [`Planar::frames`].
    pub fn read<R>(&self, frames: usize, f: impl FnOnce(&[&[f32]]) -> R) -> R {
        self.read_range(0, frames, f)
    }

    /// Calls `f` with the first `frames` samples of each channel, to write.
    ///
    /// # Panics
    ///
    /// When `frames` is more than [`Planar::frames`].
    pub fn write<R>(&mut self, frames: usize, f: impl FnOnce(&mut [&mut [f32]]) -> R) -> R {
        self.write_range(0, frames, f)
    }

    /// Calls `f` with `frames` samples of each channel from `offset`.
    ///
    /// # Panics
    ///
    /// When the range goes past [`Planar::frames`].
    pub fn read_range<R>(&self, offset: usize, frames: usize, f: impl FnOnce(&[&[f32]]) -> R) -> R {
        let end = self.end(offset, frames);
        let mut slices: [&[f32]; MAX_CHANNELS] = [&[]; MAX_CHANNELS];
        if self.frames > 0 {
            for (slice, channel) in slices.iter_mut().zip(self.data.chunks_exact(self.frames)) {
                *slice = &channel[offset..end];
            }
        }
        f(&slices[..self.channels])
    }

    /// Calls `f` with `frames` samples of each channel from `offset`, to write.
    ///
    /// # Panics
    ///
    /// When the range goes past [`Planar::frames`].
    pub fn write_range<R>(
        &mut self,
        offset: usize,
        frames: usize,
        f: impl FnOnce(&mut [&mut [f32]]) -> R,
    ) -> R {
        let end = self.end(offset, frames);
        let mut slices: [&mut [f32]; MAX_CHANNELS] =
            std::array::from_fn(|_| <&mut [f32]>::default());
        if self.frames > 0 {
            for (slice, channel) in slices
                .iter_mut()
                .zip(self.data.chunks_exact_mut(self.frames))
            {
                *slice = &mut channel[offset..end];
            }
        }
        f(&mut slices[..self.channels])
    }

    /// The end of `frames` from `offset`, checked.
    fn end(&self, offset: usize, frames: usize) -> usize {
        let end = offset.saturating_add(frames);
        assert!(
            end <= self.frames,
            "{frames} samples from {offset} of a buffer of {}",
            self.frames
        );
        end
    }
}
