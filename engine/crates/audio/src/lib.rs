//! The graph of the sound of a timeline (plan of phase 3, Fairlight): clips, track strips, buses
//! and outputs, processed in blocks on the processor, the same code for playback, export and
//! analysis.
//!
//! This first part (T4a) is what every node and the graph stand on:
//!
//! - [`node`]: the interface of a node ([`Node`]), what it is told of a block ([`Block`]: where
//!   it starts on the timeline, the sidechain, the automation), and the planar buffers between
//!   nodes ([`Planar`]).
//! - [`rt`]: what the threads of the sound share without locks (rings, a seqlock) and the 10 ms
//!   ramp of a parameter ([`rt::Smoothed`]).
//! - [`fft`]: the transform of Fourier of real signals, planned before the sound runs.
//! - [`constants`]: the numbers that the manual does not give, in one place.
//! - [`fx`]: the nodes of the effects (the EQ, T7; Dynamics, T8).
//! - [`graph`]: the graph of a timeline built from its project, its routing, mixing, delays and
//!   levels (T4b), and [`feed`], where it takes the sound of its clips from.
//! - [`play`]: playback (T5): the thread of the mix that runs a graph ahead of the device, takes
//!   the changes of its settings and swaps in a new graph at the edge of a block; the clips' sound
//!   read ahead into rings; and what the device takes from its ring. [`stream`] makes the mix into
//!   the device's rate and channels.
//! - [`meter`], [`loudness`], [`standards`], [`analysis`]: the level meters, the loudness of
//!   BS.1770, the table of the loudness standards, and the analysis of a whole sound with the
//!   gains of Normalize Audio Levels and Level Matcher (T9).
//! - [`deliver`]: the sound of an export (T6): the bus that is rendered, taken from the graph,
//!   with the gain and the limiter of Audio Normalization after it, at the rate of the file.
//! - `testing` (feature `testing`): the check that the output of a node does not depend on its
//!   blocks, the allocator that counts what the thread of the mix takes, test nodes, signals.
//!
//! # The rules of the thread of the mix
//!
//! The thread of the mix and the callback of the card do not lock, allocate or free memory, do
//! I/O, log, or wait for another thread (convention 3 of the plan). Everything they use is taken
//! when the graph is built ([`Node::prepare`]).

pub mod analysis;
pub mod constants;
pub mod deliver;
pub mod feed;
pub mod fft;
pub mod fx;
pub mod graph;
pub mod loudness;
pub mod meter;
pub mod node;
pub mod play;
pub mod rt;
pub mod standards;
pub mod stream;
#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use node::{
    Automated, Automation, Block, Layout, Node, ParamId, Planar, Role, Spec, Unsupported,
};

/// The gain of `db` decibels; silence at or below [`constants::SILENCE_DB`].
#[must_use]
pub fn db_to_gain(db: f32) -> f32 {
    if db <= constants::SILENCE_DB {
        0.0
    } else {
        10.0_f32.powf(db / 20.0)
    }
}

/// The decibels of `gain`; −∞ for silence.
#[must_use]
pub fn gain_to_db(gain: f32) -> f32 {
    20.0 * gain.abs().log10()
}
