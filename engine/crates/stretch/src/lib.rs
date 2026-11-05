//! Time-stretching and pitch-shifting of sound with Signalsmith Stretch (plan of phase 3, T13): a
//! change of speed that keeps the pitch (a clip's Pitch Correction, p. 3827), and a pitch moved
//! apart from the speed (a clip's Pitch, p. 3826, and the Pitch insert, p. 3971).
//!
//! The library is C++ and header-only; a thin shim (`src/shim.cpp`) puts a C interface over it,
//! built by `build.rs` with the cached `cc` and linked statically, as `columbina-ocio` does with
//! `OpenColorIO`. [`Stretch`] is the one safe face of it.
//!
//! # The sources
//!
//! `vendor/` holds the headers that are compiled, copied unchanged from the releases below, each
//! beside its license (MIT). Nothing else is used, and nothing is fetched when the crate is built.
//!
//! | Library | Release | Commit | SHA-256 of the archive of the tag | Files |
//! | --- | --- | --- | --- | --- |
//! | Signalsmith Stretch, <https://github.com/Signalsmith-Audio/signalsmith-stretch> | tag `1.4.0` (its header says `version = {1, 3, 2}`) | `a670068d9aeb64913331d5cc29337b19a457a7df` | `077235709ecf2a358545e3ca7bdf32859f70f5c9afb4493e078bd753c9aff284` (`archive/refs/tags/1.4.0.tar.gz`) | `signalsmith-stretch.h`, `LICENSE.txt` |
//! | Signalsmith Linear, <https://github.com/Signalsmith-Audio/linear> | tag `0.6.4`, the one that Stretch 1.4.0 names in its `CMakeLists.txt` | `de55e6a50ffcf6f8f43f649692d94691c7025151` | `2cb10d84b96c626255ab46c4aa4c80f64ae532be487063cff6a08903c1445a56` (`archive/refs/tags/0.6.4.tar.gz`) | `stft.h`, `fft.h`, `LICENSE.txt` |
//!
//! Stretch includes `signalsmith-linear/stft.h`, which includes `fft.h` alone; with none of the
//! `SIGNALSMITH_USE_*` macros defined, the transforms are Linear's own, in portable C++.
//!
//! # What it takes, and when
//!
//! A stretcher takes all its memory when it is made ([`Stretch::new`]). [`Stretch::reset`],
//! [`Stretch::seek`], [`Stretch::process`] and [`Stretch::set_transpose`] take none (feature
//! `count-allocations` counts the allocations of C++ to show it), do not lock, and may be called
//! on the thread of the mix.
//!
//! # The same sound, the same bits
//!
//! The library randomises phases when it stretches time by more than 2; its random numbers come
//! from an engine whose state the shim keeps and [`Stretch::reset`] sets back to the seed. A
//! stretcher fed the same input in the same calls after a reset gives the same output to the bit,
//! as does a new one. The output does depend on how the input and output are cut into calls of
//! [`Stretch::process`]: a caller that wants output independent of its blocks cuts the calls on a
//! grid of its own (as `columbina-audio` does).

#[allow(unsafe_code)]
mod ffi;

pub use ffi::Stretch;

/// The most channels that a stretcher takes: those of an Adaptive track (p. 3724).
pub const MAX_CHANNELS: usize = 24;

/// A stretcher could not be made.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// Of no channel, or of more than [`MAX_CHANNELS`].
    #[error("a stretcher takes 1 to {MAX_CHANNELS} channels, not {0}")]
    Channels(usize),
    /// At a rate below 8000 samples a second.
    #[error("a stretcher needs a sample rate of at least 8 kHz")]
    Rate,
    /// The library failed to make it (it ran out of memory).
    #[error("the stretcher could not be made")]
    Made,
}

/// How a stretcher of the preset Default works at a rate (the library's `presetDefault`: blocks of
/// 120 ms every 30 ms).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    /// The samples of a block of its transform.
    pub block: usize,
    /// The samples between two blocks.
    pub interval: usize,
    /// How far ahead of the time it processes its input is fed, in input samples.
    pub input_latency: usize,
    /// How far behind the time it processes its output comes, in output samples.
    pub output_latency: usize,
    /// The samples of input that [`Stretch::seek`] reads at most (a block and an interval).
    pub seek_length: usize,
}

impl Config {
    /// The configuration of a stretcher at `rate`: the same for any count of channels. The first
    /// time a rate is asked, makes a stretcher of one channel to ask it, so it takes memory and
    /// locks: not for the thread of the mix. A rate known already is answered from what was
    /// asked before (T4c's review: a graph of many Pitches made none for each).
    ///
    /// # Errors
    ///
    /// As [`Stretch::new`].
    pub fn of(rate: u32) -> Result<Config, Error> {
        static KNOWN: std::sync::Mutex<Vec<(u32, Config)>> = std::sync::Mutex::new(Vec::new());
        let mut known = KNOWN
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(&(_, config)) = known.iter().find(|(known, _)| *known == rate) {
            return Ok(config);
        }
        let config = Stretch::new(1, rate)?.config();
        known.push((rate, config));
        Ok(config)
    }
}

/// The counting of the allocations of C++ on a thread (feature `count-allocations`).
#[cfg(feature = "count-allocations")]
pub mod allocations {
    /// Starts (`true`) or stops counting the allocations and frees of C++ on this thread; whether
    /// it was counting before.
    #[must_use]
    pub fn arm(on: bool) -> bool {
        super::ffi::arm(on)
    }

    /// The allocations and the frees of C++ counted on this thread so far.
    #[must_use]
    pub fn counted() -> (usize, usize) {
        super::ffi::counted()
    }
}
