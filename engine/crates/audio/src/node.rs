//! The interface of a node of the graph: an effect, a filter, a delay, anything that takes blocks
//! of sound in and gives blocks of sound out.
//!
//! The graph runs every node once for each block, in order, the blocks one after the other along
//! the timeline (never the same samples twice, never back), and the output of a node must not
//! depend on where the sound is cut into blocks: the same signal processed whole and processed in
//! blocks of any lengths gives the same output to the bit (`testing::chunk_invariance`).
//!
//! # The two times of a node
//!
//! [`Node::prepare`] is called when the graph is built, off the thread of the mix: it may
//! allocate, and it takes all the memory that the node will need. [`Node::process`],
//! [`Node::reset`] and [`Node::set`] are called on the thread of the mix: they must not lock,
//! allocate or free memory, do I/O, log, or wait (convention 3 of the plan).

use crate::constants::MAX_CHANNELS;

/// A parameter of a node, by the node's own numbering.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ParamId(pub u32);

/// What a node is prepared for: the rate of the timeline, the longest block, and the channels in,
/// out and of the sidechain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Spec {
    /// Samples a second.
    pub rate: u32,
    /// The most samples of a block.
    pub max_block: usize,
    /// The channels of the input.
    pub inputs: usize,
    /// The channels of the output.
    pub outputs: usize,
    /// The channels of the sidechain (Dynamics, T8; Ducker, T11), 0 when there is none.
    pub sidechain: usize,
}

impl Spec {
    /// A node of `channels` in and as many out, with no sidechain.
    #[must_use]
    pub fn new(rate: u32, max_block: usize, channels: usize) -> Self {
        Spec {
            rate,
            max_block,
            inputs: channels,
            outputs: channels,
            sidechain: 0,
        }
    }

    /// The same, with `outputs` channels out.
    #[must_use]
    pub fn with_outputs(self, outputs: usize) -> Self {
        Spec { outputs, ..self }
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
}

/// A node that cannot work with the channels of a [`Spec`] (Stereo Width on a mono track, say).
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("this node does not take {inputs} channels in, {outputs} out and {sidechain} of sidechain")]
pub struct Unsupported {
    /// The channels in that were asked for.
    pub inputs: usize,
    /// The channels out.
    pub outputs: usize,
    /// The channels of the sidechain.
    pub sidechain: usize,
}

impl From<&Spec> for Unsupported {
    fn from(spec: &Spec) -> Self {
        Unsupported {
            inputs: spec.inputs,
            outputs: spec.outputs,
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

/// The automated parameters of a node through a block. An automated parameter takes the values
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
    /// The sample of the timeline of the first sample of the block (negative in the preroll
    /// before the start).
    pub start: i64,
    /// Samples a second: the rate of the [`Spec`].
    pub rate: u32,
    /// The samples of the block: the length of every channel in and out.
    pub frames: usize,
    /// The sidechain, a channel for each of the [`Spec`]'s, each `frames` long; empty when there
    /// is none.
    pub sidechain: &'a [&'a [f32]],
    /// The automated parameters.
    pub automation: Automation<'a>,
}

impl Block<'static> {
    /// A block of `frames` samples from `start`, with no sidechain and no automation.
    #[must_use]
    pub fn new(start: i64, rate: u32, frames: usize) -> Self {
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
    /// [`Node::reset`] does. Called off the thread of the mix, before any other call, and again
    /// whenever the spec changes.
    ///
    /// # Errors
    ///
    /// [`Unsupported`] when the node does not work with these channels.
    fn prepare(&mut self, spec: &Spec) -> Result<(), Unsupported>;

    /// Processes a block: `input` and `output` are planar (a slice for each channel), every one
    /// `ctx.frames` long, as many as the spec's inputs and outputs. Does not allocate.
    fn process(&mut self, ctx: &Block, input: &[&[f32]], output: &mut [&mut [f32]]);

    /// The delay of the node, in samples: the output at sample `n` answers the input at sample
    /// `n − latency`. The graph lines the paths up by it (T4b).
    fn latency(&self) -> u32;

    /// How many samples after a reset the output differs by less than −90 dBFS
    /// ([`crate::constants::SETTLE_FLOOR_DBFS`]) from the output of the same node fed the same
    /// sound from earlier. A preroll this long makes a start in the middle sound as a start from
    /// the beginning (T6).
    fn settle(&self) -> u32;

    /// Back to silence, as after `prepare` (on a seek, or when the graph is swapped). Does not
    /// allocate.
    fn reset(&mut self);

    /// Sets a parameter while the sound plays: the change is smoothed (10 ms,
    /// [`crate::constants::SMOOTHING`]) from the next sample processed. Does not allocate; an
    /// unknown parameter is ignored.
    fn set(&mut self, param: ParamId, value: f32);
}

/// Sound in planar layout, a channel after another, each of a fixed length, taken once: the
/// buffers between the nodes. [`Planar::read`] and [`Planar::write`] give the slices that
/// [`Node::process`] takes, without allocating.
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
        assert!(
            frames <= self.frames,
            "{frames} samples of a buffer of {}",
            self.frames
        );
        let mut slices: [&[f32]; MAX_CHANNELS] = [&[]; MAX_CHANNELS];
        if self.frames > 0 {
            for (slice, channel) in slices.iter_mut().zip(self.data.chunks_exact(self.frames)) {
                *slice = &channel[..frames];
            }
        }
        f(&slices[..self.channels])
    }

    /// Calls `f` with the first `frames` samples of each channel, to write.
    ///
    /// # Panics
    ///
    /// When `frames` is more than [`Planar::frames`].
    pub fn write<R>(&mut self, frames: usize, f: impl FnOnce(&mut [&mut [f32]]) -> R) -> R {
        assert!(
            frames <= self.frames,
            "{frames} samples of a buffer of {}",
            self.frames
        );
        let mut slices: [&mut [f32]; MAX_CHANNELS] =
            std::array::from_fn(|_| <&mut [f32]>::default());
        if self.frames > 0 {
            for (slice, channel) in slices
                .iter_mut()
                .zip(self.data.chunks_exact_mut(self.frames))
            {
                *slice = &mut channel[..frames];
            }
        }
        f(&mut slices[..self.channels])
    }
}
