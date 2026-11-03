//! The transform of Fourier of a real signal and back (`realfft` over `rustfft`), planned when
//! the graph is built: [`RealFft::new`] plans both ways and takes the scratch they need, so that
//! [`RealFft::forward`] and [`RealFft::inverse`] on the thread of the mix take no memory.
//!
//! The forward transform is the plain sum, X\[k\] = Σ x\[n\]·e^(−2πikn/N), of N/2 + 1 bins; the
//! inverse divides by N, so that the inverse of the forward is the signal again.

use std::sync::Arc;

use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};

pub use rustfft::num_complex::Complex32;

/// The transforms of real signals of one length, both ways, with their scratch.
pub struct RealFft {
    forward: Arc<dyn RealToComplex<f32>>,
    inverse: Arc<dyn ComplexToReal<f32>>,
    scratch: Vec<Complex32>,
    size: usize,
    scale: f32,
}

impl std::fmt::Debug for RealFft {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RealFft")
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

impl RealFft {
    /// Plans the transforms of `size` samples. Off the thread of the mix: it plans and allocates.
    ///
    /// # Panics
    ///
    /// When `size` is 0.
    #[must_use]
    pub fn new(size: usize) -> Self {
        assert!(size > 0, "a transform of no samples");
        let mut planner = RealFftPlanner::<f32>::new();
        let forward = planner.plan_fft_forward(size);
        let inverse = planner.plan_fft_inverse(size);
        let scratch = forward.get_scratch_len().max(inverse.get_scratch_len());
        #[allow(clippy::cast_precision_loss, reason = "the lengths of transforms")]
        let scale = 1.0 / size as f32;
        RealFft {
            forward,
            inverse,
            scratch: vec![Complex32::default(); scratch],
            size,
            scale,
        }
    }

    /// The samples of the signal.
    #[must_use]
    pub fn size(&self) -> usize {
        self.size
    }

    /// The bins of the spectrum: `size / 2 + 1`.
    #[must_use]
    pub fn bins(&self) -> usize {
        self.size / 2 + 1
    }

    /// A signal of zeros of the right length. Allocates.
    #[must_use]
    pub fn signal(&self) -> Vec<f32> {
        vec![0.0; self.size]
    }

    /// A spectrum of zeros of the right length. Allocates.
    #[must_use]
    pub fn spectrum(&self) -> Vec<Complex32> {
        vec![Complex32::default(); self.bins()]
    }

    /// The spectrum of `signal` into `spectrum`. `signal` is used as scratch: what it holds after
    /// is not the signal. Takes no memory.
    ///
    /// # Panics
    ///
    /// When `signal` is not [`RealFft::size`] long or `spectrum` not [`RealFft::bins`].
    pub fn forward(&mut self, signal: &mut [f32], spectrum: &mut [Complex32]) {
        assert_eq!(signal.len(), self.size, "the length of the signal");
        assert_eq!(spectrum.len(), self.bins(), "the length of the spectrum");
        let done = self
            .forward
            .process_with_scratch(signal, spectrum, &mut self.scratch);
        debug_assert!(done.is_ok(), "the lengths were checked");
    }

    /// The signal of `spectrum` into `signal`, divided by the size. The imaginary parts of the
    /// first bin, and of the last when the size is even, are taken as 0, as they are in the
    /// spectrum of any real signal. `spectrum` is used as scratch. Takes no memory.
    ///
    /// # Panics
    ///
    /// When `spectrum` is not [`RealFft::bins`] long or `signal` not [`RealFft::size`].
    pub fn inverse(&mut self, spectrum: &mut [Complex32], signal: &mut [f32]) {
        assert_eq!(spectrum.len(), self.bins(), "the length of the spectrum");
        assert_eq!(signal.len(), self.size, "the length of the signal");
        spectrum[0].im = 0.0;
        if self.size.is_multiple_of(2) {
            spectrum[self.size / 2].im = 0.0;
        }
        let done = self
            .inverse
            .process_with_scratch(spectrum, signal, &mut self.scratch);
        debug_assert!(
            done.is_ok(),
            "the lengths were checked and the edges made real"
        );
        for sample in signal {
            *sample *= self.scale;
        }
    }
}
