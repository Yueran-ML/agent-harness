//! The graphics card of the engine, and the frames of a timeline put together on it.
//!
//! - [`Gpu`]: the one device of the engine, which the views and the compositor share, so that a
//!   frame composed on the card is shown from the card.
//! - [`Compositor`]: the layers of a [`Scene`] (pictures, titles, colours, and subtitles in a
//!   preview) put together into a [`Target`]: a texture that a view shows, or a picture of NV12
//!   or P010 read back for an encoder. Playing and rendering a timeline go through it alike, so
//!   that what is rendered is what was seen.
//! - [`color`]: the stages of colour of a frame, in the order of `DaVinci Resolve`: how a picture
//!   is decoded ([`YuvCoding`]), and the input transform, grade, timeline grade and output
//!   transform ([`Stage`]), with how later work plugs into them.
//! - [`geometry`]: where a layer is put, worked out without the card.
//! - [`grade`]: a grade of the Color page at a frame, made into an operation of a [`Stage`].
//! - [`yuv`]: the planes of a picture on the card, and the shader that makes them R'G'B', which
//!   the view draws with too.

pub mod color;
mod compose;
pub mod geometry;
mod gpu;
pub mod grade;
mod text;
pub mod yuv;

pub use color::{
    ColorOp, ColorRange, LayerColor, OpTexture, Stage, TextureKind, YuvCoding, YuvMatrix,
};
pub use compose::{ComposeError, Compositor, Scene, SceneLayer, Source, Target, WORKING_FORMAT};
pub use geometry::{quarter_turns, Transform};
#[doc(hidden)]
pub use gpu::gpu_for_tests;
pub use gpu::{Gpu, GpuError, Preference, Watch};
pub use text::{warm_up as warm_up_fonts, Text};
