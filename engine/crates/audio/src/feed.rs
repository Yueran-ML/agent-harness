//! Where the graph takes the sound of its clips from (plan T4b): a [`ClipFeed`]. Playback (T5)
//! gives it from rings that threads of their own fill ahead; export and analysis (T6, T9) read
//! the media as they go; the tests make it up. The graph does not care which.

use std::ops::Range;

/// A clip whose sound the graph reads, as [`crate::graph::Graph::clips`] lists them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipSource {
    /// The clip.
    pub clip: String,
    /// Its track.
    pub track: String,
    /// The media it is cut from.
    pub media: String,
    /// The channels it is read in: its track's.
    pub channels: usize,
    /// Whether the mix needs it: its track is heard (`resolve::heard`), or drives a sidechain or
    /// a Ducker, itself or through the buses it feeds. A feed may give silence for a clip that
    /// is not needed and cannot be read (a muted track of offline media), and go on.
    pub heard: bool,
}

/// The sound of a clip could not be given; what went wrong is the feed's to keep (the thread of
/// the mix holds no text).
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("the sound of clip {clip} of the graph could not be read")]
pub struct FeedError {
    /// The clip, by its place in [`crate::graph::Graph::clips`].
    pub clip: usize,
}

/// What gives the graph the sound of its clips.
pub trait ClipFeed {
    /// Writes the sound of clip `clip` (its place in [`crate::graph::Graph::clips`]) into `out`:
    /// samples `range` of its media, counted at the rate of the timeline from the start of the
    /// media (as phases 1 and 2 read it: the media resampled to the rate of the timeline, its
    /// sample 0 at the start of its picture), each channel of its track in a slice of
    /// `range.end - range.start` samples. A clip whose channels are named (the model's
    /// `channels`) gives them in the order of its track; with none, as libswresample makes the
    /// track's count of channels. Samples before or after the media are silence.
    ///
    /// The graph calls it from the thread of the mix when it plays: a feed for playback must not
    /// lock, allocate or wait, and gives silence for what it has not got.
    ///
    /// # Errors
    ///
    /// The sound could not be read: the graph leaves the clip silent in the block and says so.
    fn read(
        &mut self,
        clip: usize,
        range: Range<i64>,
        out: &mut [&mut [f32]],
    ) -> Result<(), FeedError>;
}

impl<F> ClipFeed for F
where
    F: FnMut(usize, Range<i64>, &mut [&mut [f32]]) -> Result<(), FeedError>,
{
    fn read(
        &mut self,
        clip: usize,
        range: Range<i64>,
        out: &mut [&mut [f32]],
    ) -> Result<(), FeedError> {
        self(clip, range, out)
    }
}
