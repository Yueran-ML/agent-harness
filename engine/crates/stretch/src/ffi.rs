//! The C interface of the shim (`src/shim.cpp`), and the only unsafe code of the crate:
//! [`Stretch`] owns one handle and passes each call on with pointers to slices that it has
//! checked.

use std::ffi::c_int;
use std::ptr::NonNull;

use crate::{Config, Error, MAX_CHANNELS};

#[repr(C)]
struct Raw {
    _private: [u8; 0],
}

extern "C" {
    fn cs_stretch_new(channels: c_int, rate: f32, seed: u64) -> *mut Raw;
    fn cs_stretch_free(stretch: *mut Raw);
    fn cs_stretch_block_samples(stretch: *const Raw) -> c_int;
    fn cs_stretch_interval_samples(stretch: *const Raw) -> c_int;
    fn cs_stretch_input_latency(stretch: *const Raw) -> c_int;
    fn cs_stretch_output_latency(stretch: *const Raw) -> c_int;
    fn cs_stretch_seek_length(stretch: *const Raw) -> c_int;
    fn cs_stretch_reset(stretch: *mut Raw);
    fn cs_stretch_set_transpose(stretch: *mut Raw, factor: f32);
    fn cs_stretch_seek(stretch: *mut Raw, inputs: *const *const f32, samples: c_int, rate: f64);
    fn cs_stretch_process(
        stretch: *mut Raw,
        inputs: *const *const f32,
        input_samples: c_int,
        outputs: *const *mut f32,
        output_samples: c_int,
    );
}

#[cfg(feature = "count-allocations")]
extern "C" {
    fn cs_alloc_arm(on: c_int) -> c_int;
    fn cs_alloc_allocations() -> usize;
    fn cs_alloc_frees() -> usize;
}

#[cfg(feature = "count-allocations")]
pub(crate) fn arm(on: bool) -> bool {
    // SAFETY: sets a variable of the calling thread.
    unsafe { cs_alloc_arm(c_int::from(on)) != 0 }
}

#[cfg(feature = "count-allocations")]
pub(crate) fn counted() -> (usize, usize) {
    // SAFETY: reads variables of the calling thread.
    unsafe { (cs_alloc_allocations(), cs_alloc_frees()) }
}

/// The seed of the random numbers of every stretcher: the same everywhere, so that the same sound
/// stretches to the same bits.
const SEED: u64 = 0x5EED_0FC0_1B14_A0C5;

/// The lowest rate a stretcher takes: far below it the library's block of 120 ms would be of
/// no sample.
const LOWEST_RATE: u32 = 8_000;

/// A count of samples as the shim takes it.
fn count(samples: usize) -> c_int {
    c_int::try_from(samples).unwrap_or(c_int::MAX)
}

/// A count that the shim gave back.
fn size(value: c_int) -> usize {
    usize::try_from(value).unwrap_or(0)
}

/// One stretcher of Signalsmith Stretch, its preset Default, of a fixed count of channels: input
/// in, output out, as many of each as [`Stretch::process`] is handed (their ratio is the change of
/// speed), the pitch moved by [`Stretch::set_transpose`]. See the crate for what it takes and when.
#[derive(Debug)]
pub struct Stretch {
    raw: NonNull<Raw>,
    channels: usize,
}

// SAFETY: the handle is owned by this value alone and the library keeps no state of a thread: it
// may be moved to another thread and used there.
unsafe impl Send for Stretch {}

impl Stretch {
    /// A stretcher of `channels` at `rate`, its pitch not moved: all the memory it will take.
    ///
    /// # Errors
    ///
    /// No channel or more than [`MAX_CHANNELS`]; a rate below 8 kHz; the library could not make it.
    pub fn new(channels: usize, rate: u32) -> Result<Stretch, Error> {
        if !(1..=MAX_CHANNELS).contains(&channels) {
            return Err(Error::Channels(channels));
        }
        if rate < LOWEST_RATE {
            return Err(Error::Rate);
        }
        #[allow(clippy::cast_precision_loss, reason = "a sample rate, far below 2^24")]
        // SAFETY: the shim checks its arguments and answers null when it fails.
        let raw = unsafe { cs_stretch_new(count(channels), rate as f32, SEED) };
        NonNull::new(raw)
            .map(|raw| Stretch { raw, channels })
            .ok_or(Error::Made)
    }

    /// Its channels.
    #[must_use]
    pub fn channels(&self) -> usize {
        self.channels
    }

    /// How it works: its block, interval, latencies and the length of a seek.
    #[must_use]
    pub fn config(&self) -> Config {
        let raw = self.raw.as_ptr();
        // SAFETY: `raw` is a live handle; these only read it.
        unsafe {
            Config {
                block: size(cs_stretch_block_samples(raw)),
                interval: size(cs_stretch_interval_samples(raw)),
                input_latency: size(cs_stretch_input_latency(raw)),
                output_latency: size(cs_stretch_output_latency(raw)),
                seek_length: size(cs_stretch_seek_length(raw)),
            }
        }
    }

    /// Back to silence, its random numbers from their seed again; the pitch set is kept. Takes no
    /// memory.
    pub fn reset(&mut self) {
        // SAFETY: `raw` is a live handle, used by this value alone.
        unsafe { cs_stretch_reset(self.raw.as_ptr()) }
    }

    /// Moves the pitch by `factor` (2 an octave up), from the next block it processes.
    pub fn set_transpose(&mut self, factor: f32) {
        if factor.is_finite() && factor > 0.0 {
            // SAFETY: `raw` is a live handle, used by this value alone.
            unsafe { cs_stretch_set_transpose(self.raw.as_ptr(), factor) }
        }
    }

    /// Hands it `input` (a slice for each channel, all as long) as the sound just before where it
    /// is to go on, without output: after a [`Stretch::reset`], the time it processes is then the
    /// end of `input` less its input latency. `rate` is the change of speed to come (input samples
    /// for each output sample). Takes no memory. A count of channels other than its own does
    /// nothing.
    pub fn seek(&mut self, input: &[&[f32]], rate: f64) {
        let Some((inputs, samples)) = self.inputs(input) else {
            return;
        };
        // SAFETY: `raw` is a live handle; `inputs` holds a pointer to each of `channels` slices,
        // each at least `samples` long, which the library reads and does not keep.
        unsafe { cs_stretch_seek(self.raw.as_ptr(), inputs.as_ptr(), count(samples), rate) }
    }

    /// Takes in `input` and gives out `output` (a slice for each channel, the inputs all as long,
    /// the outputs all as long): the speed changes by their ratio. Takes no memory. Counts of
    /// channels other than its own do nothing.
    pub fn process(&mut self, input: &[&[f32]], output: &mut [&mut [f32]]) {
        let Some((inputs, in_samples)) = self.inputs(input) else {
            return;
        };
        if output.len() != self.channels {
            return;
        }
        let out_samples = output
            .iter()
            .map(|channel| channel.len())
            .min()
            .unwrap_or(0);
        let mut outputs = [std::ptr::null_mut::<f32>(); MAX_CHANNELS];
        for (pointer, channel) in outputs.iter_mut().zip(output.iter_mut()) {
            *pointer = channel.as_mut_ptr();
        }
        // SAFETY: `raw` is a live handle; `inputs` and `outputs` hold a pointer to each of
        // `channels` slices, at least `in_samples` and `out_samples` long, which the library
        // reads and writes within those lengths and does not keep; the outputs are other memory
        // than the inputs (`&mut`).
        unsafe {
            cs_stretch_process(
                self.raw.as_ptr(),
                inputs.as_ptr(),
                count(in_samples),
                outputs.as_ptr(),
                count(out_samples),
            );
        }
    }

    /// Pointers to the channels of `input`, and their samples (the shortest), if they are as many
    /// as its own.
    fn inputs(&self, input: &[&[f32]]) -> Option<([*const f32; MAX_CHANNELS], usize)> {
        if input.len() != self.channels {
            return None;
        }
        let samples = input.iter().map(|channel| channel.len()).min().unwrap_or(0);
        let mut inputs = [std::ptr::null::<f32>(); MAX_CHANNELS];
        for (pointer, channel) in inputs.iter_mut().zip(input) {
            *pointer = channel.as_ptr();
        }
        Some((inputs, samples))
    }
}

impl Drop for Stretch {
    fn drop(&mut self) {
        // SAFETY: `raw` was made by `cs_stretch_new` and is freed once, here.
        unsafe { cs_stretch_free(self.raw.as_ptr()) }
    }
}
