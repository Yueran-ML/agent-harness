//! Window tracking for the Color page: the cloud tracker and the point tracker of `DaVinci
//! Resolve` (Reference Manual of Resolve 20, chapter 140, pp. 3229–3255), on the processor, with
//! no other library.
//!
//! # What it does
//!
//! A window placed over a subject at one frame is made to follow it: the tracker finds how the
//! subject moves from frame to frame and writes that motion into the window's [`TrackData`], a
//! curve for each of Pan, Tilt, Zoom, Rotate and the four numbers of Perspective 3D ([`Motion`]),
//! kept apart from the window's own parameters (p. 3232).
//!
//! - [`CloudTracker`] (the default, p. 3234): good features to track (Shi–Tomasi) found within
//!   the window, each followed by pyramidal Lucas–Kanade, checked by tracking it back and by the
//!   correlation of its windows, and the motion of the window fitted to those that agree (RANSAC
//!   with a fixed seed, then least squares reweighted by Huber's weights). Features lost are
//!   replaced from within the window where it now is.
//! - [`PointTracker`] (p. 3234, pp. 3245–3249): crosshairs placed by the user, each followed on
//!   its own with a larger window. As the manual has it, rotation needs at least two points and
//!   zoom at least three (p. 3237): one gives a shift, two a shift and a rotation, three or more
//!   a similarity, and with Perspective 3D on three an affine map and four a homography
//!   ([`Model::for_points`]).
//! - The motion is fitted as a homography with Perspective 3D and as a similarity without; the
//!   boxes that are off then put their components back to no motion ([`Motion::masked`]).
//! - [`run`] makes the commands of the palette (forward, reverse, forward and back, one frame
//!   either way; pp. 3231, 3236) from a source of frames, writing [`TrackData`].
//! - [`TrackData`] holds what the palette edits: Clip and Frame mode ([`Mode`], p. 3236), Clear
//!   Selected Track Data with its line across the gap ([`TrackData::clear_range`], p. 3238),
//!   interpolation over a range, keyframes of Frame mode, Copy and Paste Track Data
//!   ([`TrackData::paste`], p. 3233), and the change of scale from a proxy to the original.
//!
//! `IntelliTrack` (Studio, an AI) is not made, nor the stabilizer, which is to share this core.
//!
//! # Determinism
//!
//! The same frames and settings give the same track to the bit: RANSAC draws from a fixed seed,
//! and the features tracked on several threads each depend on their own work alone, their
//! results gathered in order.
//!
//! # Coordinates
//!
//! Points are in pixels of the frames given, `[x, y]` with y down, the centre of the pixel at
//! column `i` and row `j` being `[i, j]`. Luma is taken as 0 for black to 1 for white.

#![allow(
    clippy::many_single_char_names,
    reason = "the numerical code names its quantities by the letters of their formulas"
)]

mod cloud;
mod data;
mod features;
mod fit;
mod image;
mod linalg;
mod lk;
mod motion;
mod point;
mod region;
mod run;

pub use cloud::{CloudOptions, CloudTracker};
pub use data::{Adjusted, Mode, TrackData};
pub use fit::Model;
pub use image::{Luma, Pyramid};
pub use linalg::{Mat3, Point};
pub use motion::{Component, Components, Motion};
pub use point::{PointOptions, PointStep, PointTracker};
pub use region::Region;
pub use run::{run, Direction, Job, Report, Tracker};

/// What a tracker found in a frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Step {
    /// The motion from the tracker's first frame to this one, as a homography of the frame
    /// (fitted with every component; the boxes that are off are left out later).
    pub motion: Mat3,
    /// How well the motion fits, from 0 (lost) to 1: the share of the features that agree with
    /// it, less when there are few of them or they agree loosely.
    pub confidence: f32,
    /// How many features or crosshairs the motion was fitted to.
    pub features: usize,
    /// Whether the subject was lost at this frame (the motion is then the last one found).
    pub lost: bool,
}

/// What can go wrong.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TrackError {
    /// A plane whose samples do not fit its size.
    #[error("a plane of {width}×{height} with {stride} samples from row to row cannot be read from {len} samples")]
    Plane {
        /// Its width.
        width: usize,
        /// Its height.
        height: usize,
        /// Its stride.
        stride: usize,
        /// The samples given.
        len: usize,
    },
    /// A frame not of the size of the first.
    #[error("a frame of {}×{} where the track started on {}×{}", got[0], got[1], expected[0], expected[1])]
    FrameSize {
        /// The size of the first frame.
        expected: [usize; 2],
        /// The size of this one.
        got: [usize; 2],
    },
    /// The window covers no pixel of the frame.
    #[error("the window covers nothing of the frame")]
    EmptyRegion,
    /// Nothing within the window can be tracked.
    #[error("the window holds nothing that can be tracked")]
    NoFeatures,
    /// No crosshair, or one outside the frame.
    #[error("the point tracker needs at least one crosshair within the frame")]
    NoPoints,
    /// The source of frames has no frame.
    #[error("frame {0} could not be had")]
    MissingFrame(i64),
    /// The playhead is outside the clip.
    #[error("frame {start} is outside the clip, from {first} to {last}")]
    OutOfRange {
        /// The playhead.
        start: i64,
        /// The first frame of the clip.
        first: i64,
        /// The last.
        last: i64,
    },
}
