//! The points where the levels of the mix are taken (plan T4b, "电平接点"): before and after the
//! fader of each track and bus. Each block, the thread of the mix writes what a meter needs of it
//! into a seqlock, which the meters (T9) read on another thread without holding it back.
//!
//! The point before the fader is after the strip's processing and before its mute, so that a
//! track that is muted or soloed away still shows its level there (p. 3887); the point after is
//! after the mute, the solo and the fader (`after the fader level and mute button`, p. 3887).

use crate::constants::MAX_CHANNELS;
use crate::node::Planar;
use crate::rt::{SeqReader, SeqWriter};

/// What a point of the levels holds after a block: the samples it has seen since the graph
/// started, and for each channel the peak (largest size) and the mean of the squares of that
/// block. Channels past the strip's are 0.
pub type Level = (u64, [f32; MAX_CHANNELS], [f32; MAX_CHANNELS]);

/// A point where the level is taken.
#[derive(Debug)]
pub(crate) struct Tap {
    pub(crate) writer: SeqWriter<Level>,
    pub(crate) reader: SeqReader<Level>,
    pub(crate) frames: u64,
}

impl Tap {
    /// Takes the level of the first `frames` samples of the first `channels` channels of
    /// `buffer`.
    #[allow(clippy::cast_precision_loss, reason = "a mean of a block of samples")]
    pub(crate) fn take(&mut self, buffer: &Planar, channels: usize, frames: usize) {
        if frames == 0 {
            return;
        }
        let mut peaks = [0.0_f32; MAX_CHANNELS];
        let mut squares = [0.0_f32; MAX_CHANNELS];
        for channel in 0..channels {
            let mut peak = 0.0_f32;
            let mut sum = 0.0_f64;
            for &sample in &buffer.channel(channel)[..frames] {
                peak = peak.max(sample.abs());
                sum += f64::from(sample) * f64::from(sample);
            }
            peaks[channel] = peak;
            #[allow(clippy::cast_possible_truncation, reason = "a level is an f32")]
            let mean = (sum / frames as f64) as f32;
            squares[channel] = mean;
        }
        self.frames += u64::try_from(frames).unwrap_or(0);
        self.writer.write((self.frames, peaks, squares));
    }

    /// Back to nothing seen.
    pub(crate) fn reset(&mut self) {
        self.frames = 0;
        self.writer
            .write((0, [0.0; MAX_CHANNELS], [0.0; MAX_CHANNELS]));
    }
}
