//! The graphics card of the engine, and what is put together on it.
//!
//! - [`Gpu`]: the one device of the engine, which the views and the compositor share.

mod gpu;

#[doc(hidden)]
pub use gpu::gpu_for_tests;
pub use gpu::{Gpu, GpuError, Preference, Watch};
