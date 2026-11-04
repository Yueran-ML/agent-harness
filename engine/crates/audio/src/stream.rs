//! The mix of the timeline made into the sound of the device (plan T5): resampled from the rate
//! of the timeline to the rate of the device as a stream, with the history that its filter
//! reaches back to, and laid on the channels of the device, clipped.
//!
//! Phases 1 and 2 (and T4b's `OutputSource`) mixed again, for each run of the device, a window
//! around it as wide as the filter's reach on either side (the plan's check of the code, 1). A
//! graph with state (an equalizer, a compressor) cannot go over the same samples twice, so the
//! mix now goes through the graph once, in order, and [`Resampler`] keeps the samples of the mix
//! that the filter still reaches: what it gives does not depend on how the mix is handed to it nor
//! on how much is taken at a time (`tests/stream.rs`), and it is the very sound that the old
//! window gave, to the bit.
//!
//! Output frame `k` of a run from sample `at` of the timeline is the timeline at
//! `at + k · rate / device` (exactly: the position `at · device + k · rate`, in units of
//! `1 / device` of a sample), each side made by the windowed sinc of the clips' speeds
//! ([`crate::graph::resampled`]) at the cutoff `min(device / rate, 1)`: where the two rates are
//! equal every position is a whole sample, which is passed on as it is.
//!
//! The device has as many channels as it has: the two sides of the mix go to the first two and
//! nothing to the rest; a device of one channel takes the two at equal power, `(L + R) / √2`.
//! Every sample for the device is clipped to −1…1 here, after the resampling, and nowhere else on
//! the way to the device (T4b: only the copy for a device is clipped).
//!
//! Nothing here allocates once made (convention 3; `tests/rt_audit.rs`, `tests/no_alloc.rs`).

use crate::graph::{radius, resampled};

/// The mix of the timeline, in stereo, made into the frames of a device. See the module.
#[derive(Debug)]
pub struct Resampler {
    /// The rate of the timeline.
    rate: u32,
    /// The rate of the device.
    device: u32,
    /// The channels of the device.
    channels: usize,
    cutoff: f64,
    /// How many samples of the mix on each side of a position the filter reads: none where the
    /// rates are the same.
    reach: i64,
    /// The sample of the timeline of the first frame out.
    origin: i64,
    /// The frames given since the start.
    given: i64,
    /// The sample of the timeline of the first sample kept.
    base: i64,
    /// The samples kept of each side.
    kept: usize,
    left: Box<[f32]>,
    right: Box<[f32]>,
}

impl Resampler {
    /// A resampler from a timeline of `rate` to a device of `device` samples a second and
    /// `channels` channels, taking at most `most` samples of the mix at a time. Made off the
    /// thread of the mix: it takes its memory here.
    ///
    /// # Panics
    ///
    /// When a rate or `channels` is 0.
    #[must_use]
    pub fn new(rate: u32, device: u32, channels: usize, most: usize) -> Self {
        assert!(
            rate > 0 && device > 0 && channels > 0,
            "a stream of nothing"
        );
        let cutoff = (f64::from(device) / f64::from(rate)).min(1.0);
        let reach = if rate == device { 0 } else { radius(cutoff) };
        let capacity = most + 2 * usize::try_from(reach).unwrap_or(0) + 2;
        Resampler {
            rate,
            device,
            channels,
            cutoff,
            reach,
            origin: 0,
            given: 0,
            base: 0,
            kept: 0,
            left: vec![0.0; capacity].into_boxed_slice(), // rt-ok: made off the thread of the mix
            right: vec![0.0; capacity].into_boxed_slice(), // rt-ok: made off the thread of the mix
        }
    }

    /// The rate of the timeline.
    #[must_use]
    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// The rate of the device.
    #[must_use]
    pub fn device(&self) -> u32 {
        self.device
    }

    /// The channels of the device.
    #[must_use]
    pub fn channels(&self) -> usize {
        self.channels
    }

    /// How many samples of the mix the filter reads on each side of a position: the mix is
    /// started this much before the first sample out ([`Resampler::first_input`]).
    #[must_use]
    pub fn reach(&self) -> i64 {
        self.reach
    }

    /// Starts again with nothing kept, the first frame out at sample `at` of the timeline. The
    /// mix is to be handed over from [`Resampler::first_input`] on.
    pub fn start(&mut self, at: i64) {
        self.origin = at;
        self.given = 0;
        self.base = self.first_input();
        self.kept = 0;
    }

    /// The sample of the timeline that the mix is to be handed over from.
    #[must_use]
    pub fn first_input(&self) -> i64 {
        self.origin - self.reach
    }

    /// The sample of the timeline that is to be handed over next.
    #[must_use]
    pub fn next_input(&self) -> i64 {
        self.base + i64::try_from(self.kept).unwrap_or(i64::MAX)
    }

    /// The sample of the timeline of the next frame out, and the share of the way to the sample
    /// after it.
    fn position(&self, frame: i64) -> (i64, u32) {
        let position = i128::from(self.origin) * i128::from(self.device)
            + i128::from(frame) * i128::from(self.rate);
        let device = i128::from(self.device);
        (
            i64::try_from(position.div_euclid(device)).unwrap_or(i64::MAX),
            u32::try_from(position.rem_euclid(device)).unwrap_or(0),
        )
    }

    /// The sample of the timeline that the next frame out is at (to the sample below).
    #[must_use]
    pub fn next_output(&self) -> i64 {
        self.position(self.given).0
    }

    /// The frames out that `samples` more samples of the mix give at the most.
    #[must_use]
    pub fn most_out(&self, samples: usize) -> usize {
        let samples = u64::try_from(samples).unwrap_or(u64::MAX);
        usize::try_from(samples * u64::from(self.device) / u64::from(self.rate)).unwrap_or(0) + 2
    }

    /// The lowest sample of the mix that the frame at `sample` reads.
    fn lowest(&self, sample: i64) -> i64 {
        if self.reach == 0 {
            sample
        } else {
            sample - self.reach + 1
        }
    }

    /// How many samples kept the next frame out no longer reaches.
    fn gone(&self) -> usize {
        let needed = self.lowest(self.next_output());
        usize::try_from(needed - self.base)
            .unwrap_or(0)
            .min(self.kept)
    }

    /// How many more samples of the mix it takes now.
    #[must_use]
    pub fn room(&self) -> usize {
        self.left.len() - (self.kept - self.gone())
    }

    /// Hands over the next samples of the mix, left and right (the same length), as many as
    /// there is room for ([`Resampler::room`]); the samples taken.
    pub fn push(&mut self, left: &[f32], right: &[f32]) -> usize {
        // What the next frame out no longer reaches is let go.
        let gone = self.gone();
        if gone > 0 {
            self.left.copy_within(gone..self.kept, 0);
            self.right.copy_within(gone..self.kept, 0);
            self.kept -= gone;
            self.base += i64::try_from(gone).unwrap_or(0);
        }
        let count = left.len().min(right.len()).min(self.left.len() - self.kept);
        self.left[self.kept..self.kept + count].copy_from_slice(&left[..count]);
        self.right[self.kept..self.kept + count].copy_from_slice(&right[..count]);
        self.kept += count;
        count
    }

    /// Writes into `out` (interleaved, the channels of the device) as many frames as the mix
    /// handed over makes and `out` holds; the frames written.
    #[allow(
        clippy::cast_precision_loss,
        reason = "the share of the way to the next sample is a fraction in [0, 1)"
    )]
    pub fn pull(&mut self, out: &mut [f32]) -> usize {
        let channels = self.channels;
        let window = [&self.left[..self.kept], &self.right[..self.kept]];
        let mut written = 0;
        for frame in out.chunks_exact_mut(channels) {
            let (sample, remainder) = self.position(self.given);
            if sample + self.reach >= self.base + i64::try_from(self.kept).unwrap_or(0)
                || self.lowest(sample) < self.base
            {
                break;
            }
            let index = usize::try_from(sample - self.base).unwrap_or(0);
            let mut pair = [0.0_f32; 2];
            resampled(
                &window,
                index,
                f64::from(remainder) / f64::from(self.device),
                self.cutoff,
                &mut pair,
            );
            if channels == 1 {
                frame[0] = ((pair[0] + pair[1]) * std::f32::consts::FRAC_1_SQRT_2).clamp(-1.0, 1.0);
            } else {
                frame[0] = pair[0].clamp(-1.0, 1.0);
                frame[1] = pair[1].clamp(-1.0, 1.0);
                frame[2..].fill(0.0);
            }
            self.given += 1;
            written += 1;
        }
        written
    }
}
