//! The graphics card of the engine, and the frames of a timeline put together on it.
//!
//! - [`Gpu`]: the one device of the engine, which the views and the compositor share, so that a
//!   frame composed on the card is shown from the card.
//! - [`Compositor`]: the layers of a [`Scene`] (pictures, titles, colours, and subtitles in a
//!   preview) put together into a [`Target`]: a texture that a view shows, or a picture of NV12
//!   or P010 read back for an encoder. Playing and rendering a timeline go through it alike, so
//!   that what is rendered is what was seen.
//! - [`geometry`]: where a layer is put, worked out without the card.
//! - [`yuv`]: the planes of a picture on the card, and the shader that makes them R'G'B', which
//!   the view draws with too.

mod compose;
pub mod geometry;
mod gpu;
mod text;
pub mod yuv;

pub use compose::{ComposeError, Compositor, Scene, SceneLayer, Source, Target};
pub use geometry::{quarter_turns, Transform};
#[doc(hidden)]
pub use gpu::gpu_for_tests;
pub use gpu::{Gpu, GpuError, Preference, Watch};
pub use text::Text;
