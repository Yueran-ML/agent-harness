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
//! - [`neighbourhood`]: the filters of the pixels about each one that operations of colour read
//!   (Midtone Detail; blur and sharpen), on the processor as the card takes them.
//! - [`sample`]: a few pixels of a frame read back as they are, for a picker of colour.
//! - [`ocio`]: the transforms of colour management (`OpenColorIO`) as operations of a stage.
//! - [`tone`]: the tone mappings of colour management (its DRTs), and the matrices and limits
//!   of its gamuts, as operations of a stage.
//! - [`scopes`]: the video scopes of a frame counted on the card, and its figures for the agent.
//! - [`yuv`]: the planes of a picture on the card, and the shader that makes them R'G'B', which
//!   the view draws with too.

pub mod color;
mod compose;
pub mod geometry;
mod gpu;
pub mod grade;
pub mod neighbourhood;
pub mod ocio;
pub mod overlay;
mod pipelines;
pub mod sample;
pub mod scopes;
mod text;
pub mod tone;
pub mod wipe;
pub mod yuv;

pub use color::{
    ColorOp, ColorRange, Filter, LayerColor, Neighbourhood, OpTexture, Stage, TextureKind,
    YuvCoding, YuvMatrix,
};
/// How the cubes of LUTs are read between their points ([`grade::CardLuts::set_interpolation`]).
pub use columbina_lut::Interpolation;
pub use compose::{ComposeError, Compositor, Scene, SceneLayer, Source, Target, WORKING_FORMAT};
pub use geometry::{quarter_turns, Transform};
#[doc(hidden)]
pub use gpu::gpu_for_tests;
pub use gpu::{
    ChosenBy, CompilerKind, Gpu, GpuError, Preference, ShaderCompiler, Watch, COMPILER_VARIABLE,
};
pub use text::{warm_up as warm_up_fonts, Text};
