//! The compositor: the layers of a frame put together on the card into a [`Target`], a texture
//! that a view shows (a preview) or that is made Y'CbCr and read back for an encoder (an export).
//!
//! A frame goes through the stages of colour that `color.rs` lays out, in the order of
//! `DaVinci Resolve`. Each layer, from the bottom up, is made R'G'B' (a picture from Y'CbCr as it is drawn,
//! `yuv.wgsl`, by its matrix and range; a title drawn into pixels on the processor, see
//! `text.rs`, and kept for as long as the same text is drawn; a colour), goes through its input
//! transform and its grade, is fitted to the canvas, put on it by its transform, and laid over
//! what is there by its opacity, premultiplied: `out = layer + (1 - alpha of layer) * under`. Its
//! crop cuts it in its own frame, before its transform, and a composite mode other than Normal
//! blends it with what is under it (see [`SceneLayer`], and `Compositor::draw_calls`). A layer
//! of a clip that is retimed by Frame Blend is of two pictures of its media, mixed as they are
//! stored, before they are made R'G'B' ([`Source::Between`], `between.wgsl`). A layer of a clip
//! that has a composition shows what comes out of it ([`Source::Fused`]): its source is drawn
//! alone, through its input transform, into a texture, the nodes of the composition are drawn
//! from that into textures of their own ([`crate::fusion`]), and the last of them is the picture
//! of the layer, to which its grade, its crop, its transform, its composite mode and its
//! opacity are done as they are to the picture of a title. A
//! layer whose operations read the pixels about each one is drawn first into textures of its
//! own, one for each such neighbourhood, at the size of its picture, and those are filtered
//! (`color.rs`); the others are drawn in their one pass. The canvas is black at first, and of
//! half floats ([`WORKING_FORMAT`]) for a preview and an export alike: the working space of the
//! timeline. Colours are mixed as they are coded, with their gamma (as long as there is no
//! colour management, the working space is R'G'B' of Rec.709). Then the timeline grade is done
//! over the canvas, the subtitles are drawn, and the output transform is done last.
//!
//! Subtitles are drawn over the timeline grade, in a preview only: white with a black outline, in
//! the middle at the bottom, letters a twentieth of the height of the canvas high.

use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::sync::{mpsc, Arc, OnceLock};
use std::time::{Duration, Instant};

use columbina_media::{NativePicture, Picture, PictureFormat};
use columbina_model::grade::Composite;
use columbina_model::project::{Rgba, TextAlign};

use crate::color::{self, Filter, Inputs, LayerColor, Part, Program, Stage, TextureKind};
use crate::fusion::{self, Fusion, Reads};
use crate::geometry::{
    fit, minification, picture_stride, placement, raster_scale, samples, shown_size,
    subtitle_origin, subtitle_size, text_window, title_origin, Affine, Crop, Transform, UNCUT,
};
use crate::gpu::Gpu;
use crate::neighbourhood::{self, Pass};
use crate::pipelines::{Builder, Made, Make};
use crate::text::{self, Laid, Raster, Spec, Text, MOST_PIXELS};
use crate::yuv::{self, Texel};

/// The shader of layers of R'G'B'A.
const LAYER_SHADER: &str = include_str!("layer.wgsl");

/// What a shader of layers lays a layer on the canvas by, as it is, between these lines; and
/// what is put in their place for a layer that is blended with what is under it.
const BLEND_BEGIN: &str = "// blend: begin";
const BLEND_END: &str = "// blend: end";
const BLEND_SHADER: &str = include_str!("blend.wgsl");

/// What the shader of pictures reads the texels of its planes by, as it is, between these
/// lines; and what is put in their place for a layer of two pictures of one media, mixed.
const PLANES_BEGIN: &str = "// planes: begin";
const PLANES_END: &str = "// planes: end";
const BETWEEN_SHADER: &str = include_str!("between.wgsl");

/// The function of the shader of pictures that reads one picture, and what it is named in the
/// shader of a layer of two, whose own function of that name mixes the two.
const SAMPLE_ONE: &str = "fn sample_at(";
const SAMPLE_ONE_OF_TWO: &str = "fn picture_at(";

/// The shader that makes a canvas opaque, once layers that are blended are on it.
const OPAQUE_SHADER: &str = include_str!("opaque.wgsl");

/// The shader that makes the canvas Y'CbCr for an encoder.
const CONVERT_SHADER: &str = include_str!("convert.wgsl");

/// The shader of a stage of colour over the canvas, which, as it is written, copies it.
const STAGE_SHADER: &str = include_str!("stage.wgsl");

/// The shader of a pass of the filter of a neighbourhood.
const FILTER_SHADER: &str = include_str!("neighbourhood.wgsl");

/// The bytes of the uniform of `neighbourhood.wgsl`.
const FILTER_PARAMS_SIZE: u64 = 32;

/// The format of the textures of neighbourhoods: that of the canvas.
const NEIGHBOURHOOD_FORMAT: wgpu::TextureFormat = WORKING_FORMAT;

/// How many frames the textures of neighbourhoods of a size are kept for while none of that
/// size is drawn.
const SPARES_KEPT_FOR: u64 = 60;

/// The format of the canvas, the working space of the timeline, for a preview and an export.
pub const WORKING_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// The bytes of the uniform of `layer.wgsl`.
const LAYER_PARAMS_SIZE: u64 = 128;

/// The bytes of the uniform of `convert.wgsl`.
const CONVERT_PARAMS_SIZE: u64 = 32;

/// The invocations of a group of the shaders of `convert.wgsl`, across.
const CONVERT_GROUP: u32 = 64;

/// How many texts laid out are kept, the last drawn. A text is kept by all that changes its
/// pixels, the size of its letters among it: a title whose size has keys is another text at
/// each frame, so the texts of a long change of size are let go as they come, the oldest first.
const TEXTS_KEPT: usize = 64;

/// How many parts of a text drawn into pixels are kept on the card, the last drawn.
const PARTS_KEPT: usize = 2;

/// How many bytes of texts drawn into pixels are kept on the card at most, beyond those of the
/// last drawn.
const TEXT_BYTES: u64 = 256 << 20;

/// How many times a text that is too large is laid out again, smaller.
const SHRINKS: usize = 8;

/// How many frames the planes of a shape of picture are kept for while no picture of that shape
/// is drawn.
const PLANES_KEPT_FOR: u64 = 300;

/// What a frame shows: layers over a black canvas of `width` × `height`, and subtitles over
/// them (in a preview only).
#[derive(Clone, Debug)]
pub struct Scene {
    pub width: u32,
    pub height: u32,
    /// The pixels of the canvas to a pixel of the timeline: 1 where the canvas is as large as
    /// the timeline, 0.5 for a preview at half its size. The sizes of titles are in pixels of the
    /// timeline, and are drawn so many times as large; subtitles are a share of the canvas.
    pub units: f32,
    /// From the bottom up.
    pub layers: Vec<SceneLayer>,
    /// The texts of the subtitles shown, a line (or more) each, from the top down.
    pub subtitles: Vec<String>,
    /// The timeline grade: done over the canvas once the layers are laid on it, under the
    /// subtitles.
    pub grade: Stage,
    /// The output transform: from the working space of the timeline into the space of the
    /// output, done last, over the subtitles too.
    pub output: Stage,
    /// The weights of R, G and B in the luma of the working space of the timeline, which the
    /// composite modes that go by luma read (Hue, Saturation, Color, Luminosity, Darker Color
    /// and Lighter Color): those that its grades are made with
    /// ([`crate::grade::GradeOptions::luma`]), so that a layer is blended on the timeline as
    /// a Layer Mixer blends it in a grade. Those of BT.709 by default.
    pub luma: [f32; 3],
}

impl Default for Scene {
    /// Nothing, on a canvas of no size, as large as its timeline, its colour left as it is.
    fn default() -> Self {
        Scene {
            width: 0,
            height: 0,
            units: 1.0,
            layers: Vec::new(),
            subtitles: Vec::new(),
            grade: Stage::default(),
            output: Stage::default(),
            luma: crate::grade::REC709_LUMA,
        }
    }
}

/// What a layer shows.
#[derive(Clone, Debug)]
pub enum Source {
    /// A picture, in the sampling of chroma of its file (4:2:0, 4:2:2 or 4:4:4), turned clockwise
    /// so many degrees (as its media says) to be shown. Its codes are made R'G'B' as the
    /// layer's [`LayerColor::coding`] says, which is to be the picture's own
    /// ([`crate::YuvCoding::of_native`]).
    Picture(Arc<NativePicture>, i16),
    /// A picture between two of one media, which a clip that is retimed by Frame Blend shows at
    /// a frame whose place in its source lies between them: `1 - weight` of `first` and `weight`
    /// of `second`, from 0 to 1, turned `rotation` degrees and coded as a [`Source::Picture`]
    /// is. The two are of one size, sampling, depth and alpha.
    ///
    /// They are mixed as they are stored, in the codes of their file with their gamma, before
    /// they are made R'G'B' (premultiplied, where they have alpha): so before the input
    /// transform and the grade of the layer, its crop and its transform, which are done to the
    /// mixed picture as they are to one, once. The two are pictures of one source, coded
    /// alike and graded alike, and what is mixed is what the pipeline takes for the picture of
    /// the source. (A dissolve mixes on the canvas, after the grades: its two pictures are of
    /// two clips, each with a grade of its own.)
    ///
    /// At a weight of 0 or less the first alone is drawn, as a [`Source::Picture`] of it is, to
    /// the bit, and at 1 or more the second.
    Between {
        first: Arc<NativePicture>,
        second: Arc<NativePicture>,
        weight: f32,
        rotation: i16,
    },
    /// A title: its text, placed on the canvas as its style says.
    Title(Text),
    /// A colour over the canvas: R', G', B' and how opaque it is, from 0 to 1, not
    /// premultiplied.
    Color([f32; 4]),
    /// The picture that comes out of the composition of a clip ([`Fusion`]), in place of the
    /// source of the clip, which a `MediaIn` of the composition shows: decoded and through the
    /// input transform of the layer ([`LayerColor::input`]), as it is before the grade. The
    /// picture is of the shape of the composition, fitted to the canvas as a picture is; the
    /// grade of the layer, its crop (of the picture as it is fitted), its transform, its
    /// composite mode and its opacity are done to it as they are to a source, its alpha with
    /// it. Where nothing comes into the `MediaOut` of the composition, the layer shows
    /// nothing; and where the `MediaOut` shows the `MediaIn` itself, as in the composition
    /// that a clip has at first, the layer is drawn as one of the source alone is, to the bit.
    ///
    /// The source is one of the other kinds: that of a composition within it is taken in its
    /// place ([`Source::own`]).
    Fused(Box<Fusion>),
}

impl Source {
    /// A colour as the project writes it.
    #[must_use]
    pub fn color(rgba: Rgba) -> Source {
        Source::Color(rgba.0.map(|channel| f32::from(channel) / 255.0))
    }

    /// The source of the clip itself: of a layer that shows what comes out of a composition
    /// ([`Source::Fused`]), what a `MediaIn` of it shows; of any other, the source as it is.
    /// For what goes by the media of a layer: how its codes are read, its input transform,
    /// and the shape of its picture.
    #[must_use]
    pub fn own(&self) -> &Source {
        let mut source = self;
        while let Source::Fused(fusion) = source {
            source = &fusion.source;
        }
        source
    }
}

/// A layer of a scene.
#[derive(Clone, Debug)]
pub struct SceneLayer {
    pub source: Source,
    /// Where it is put, once it is fitted to the canvas.
    pub transform: Transform,
    /// How opaque it is, from 0 to 1.
    pub opacity: f32,
    /// How its picture is decoded, and its input transform and grade.
    pub color: LayerColor,
    /// How much of its picture is cut away at each side, in its own frame, before `transform`
    /// puts it on the canvas: of a picture, the picture as it is fitted; of a colour and of a
    /// title, the canvas. What is cut away shows what is under the layer.
    pub crop: Crop,
    /// How it is laid over the layers under it: as it is ([`Composite::Normal`]), or blended
    /// with the canvas as they left it, by the maths of the Layer Mixer of a grade
    /// ([`crate::grade`]), after its own grade. Where nothing is under it, it is laid as it is.
    pub composite: Composite,
}

/// Why a frame could not be composed, or read back.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ComposeError {
    /// The device of the card was lost: the compositor and its targets are of no more use, and
    /// are made again on the next card.
    #[error("the graphics card was lost: {0}")]
    Lost(String),
    /// The target is not one for this: of another card, of another size, or not of the kind.
    #[error("{0}")]
    Target(String),
    /// A layer cannot be drawn: a picture whose planes do not hold it, or one larger than the
    /// card takes.
    #[error("layer {layer} cannot be drawn: {message}")]
    Layer { layer: usize, message: String },
    /// The picture could not be read back from the card.
    #[error("the picture cannot be read back: {0}")]
    Read(String),
    /// A stage of colour over the canvas cannot be done: its code is not WGSL that the card
    /// takes. (That of a layer is an error of the layer.)
    #[error("the colour of the frame cannot be done: {0}")]
    Color(String),
}

impl ComposeError {
    /// A word for the error, for a program to read.
    #[must_use]
    pub fn reason(&self) -> &'static str {
        match self {
            ComposeError::Lost(_) => "lost",
            ComposeError::Target(_) => "target",
            ComposeError::Layer { .. } => "layer",
            ComposeError::Read(_) => "read",
            ComposeError::Color(_) => "color",
        }
    }
}

/// What a frame is composed into.
///
/// Both kinds compose into a canvas of half floats ([`WORKING_FORMAT`]) of the size of the
/// frame. A **preview** target's canvas is the picture, which a view on the same card draws into
/// its own format ([`Target::texture`]). An **export** target's canvas is then made NV12 (8 bits)
/// or P010 (10 bits) on the card, and read back into a [`Picture`] with [`Target::read_picture`].
///
/// A target is made for a size and kept from frame to frame; one that is being read must not
/// be rendered into at the same time (render into another, to read one while the next is drawn).
/// The frame always ends in [`Target::texture`]: a stage of colour over the canvas draws from
/// one canvas into a second one of the target, made as it is first needed, and the stages are
/// begun in whichever of the two makes them end there.
pub struct Target {
    gpu: Arc<Gpu>,
    width: u32,
    height: u32,
    texture: Arc<wgpu::Texture>,
    work: OnceLock<wgpu::Texture>,
    export: Option<Export>,
}

/// The buffers that the planes of an export are written into and read back from.
struct Export {
    bits: u8,
    stride: usize,
    chroma_rows: u32,
    /// The words of a row of luma and of chroma (the same: the rows are as long).
    words: u32,
    luma: wgpu::Buffer,
    chroma: wgpu::Buffer,
    luma_read: wgpu::Buffer,
    chroma_read: wgpu::Buffer,
    params: wgpu::Buffer,
}

impl std::fmt::Debug for Target {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Target")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("bits", &self.export.as_ref().map(|export| export.bits))
            .finish_non_exhaustive()
    }
}

/// Check that a canvas of `width` × `height` can be made on the card.
fn check_size(gpu: &Gpu, width: u32, height: u32) -> Result<(), ComposeError> {
    let most = gpu.device().limits().max_texture_dimension_2d;
    if width == 0 || height == 0 || width > most || height > most {
        return Err(ComposeError::Target(format!(
            "a canvas of {width}×{height} cannot be made: the card takes from 1 to {most} on a side"
        )));
    }
    Ok(())
}

impl Target {
    /// A target for a preview: a texture of `width` × `height`, of half floats
    /// ([`WORKING_FORMAT`]), that can be drawn from, copied, and copied or written into.
    ///
    /// # Errors
    /// The size is none, or more than the card takes.
    pub fn preview(gpu: &Arc<Gpu>, width: u32, height: u32) -> Result<Target, ComposeError> {
        check_size(gpu, width, height)?;
        let texture = canvas(gpu, width, height, WORKING_FORMAT, "preview");
        Ok(Target {
            gpu: Arc::clone(gpu),
            width,
            height,
            texture: Arc::new(texture),
            work: OnceLock::new(),
            export: None,
        })
    }

    /// A target for an export: a canvas of `width` × `height`, made NV12 (`bits` 8) or P010
    /// (`bits` 10) as it is rendered.
    ///
    /// # Errors
    /// The size is none or more than the card takes, or `bits` is neither 8 nor 10.
    pub fn export(
        gpu: &Arc<Gpu>,
        width: u32,
        height: u32,
        bits: u8,
    ) -> Result<Target, ComposeError> {
        check_size(gpu, width, height)?;
        if !matches!(bits, 8 | 10) {
            return Err(ComposeError::Target(format!(
                "pictures are rendered in 8 or 10 bits, not {bits}"
            )));
        }
        let stride = picture_stride(width, bits);
        let chroma_rows = height.div_ceil(2);
        let luma_size = stride as u64 * u64::from(height);
        let most = gpu.device().limits().max_storage_buffer_binding_size;
        if luma_size > most {
            return Err(ComposeError::Target(format!(
                "a picture of {width}×{height} in {bits} bits is more than the {most} bytes that \
                 the card writes at once"
            )));
        }
        let device = gpu.device();
        let buffer = |label, size, usage| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage,
                mapped_at_creation: false,
            })
        };
        let written = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC;
        let read = wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST;
        let chroma_size = stride as u64 * u64::from(chroma_rows);
        let words = u32::try_from(stride / 4)
            .map_err(|_| ComposeError::Target(format!("a canvas {width} wide is too wide")))?;
        let params = buffer(
            "columbina convert",
            CONVERT_PARAMS_SIZE,
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        );
        let values: [u32; 8] = [
            width,
            height,
            words,
            words,
            u32::from(bits > 8),
            chroma_rows,
            0,
            0,
        ];
        let bytes: Vec<u8> = values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        gpu.queue().write_buffer(&params, 0, &bytes);
        let export = Export {
            bits,
            stride,
            chroma_rows,
            words,
            luma: buffer("columbina luma", luma_size, written),
            chroma: buffer("columbina chroma", chroma_size, written),
            luma_read: buffer("columbina luma read", luma_size, read),
            chroma_read: buffer("columbina chroma read", chroma_size, read),
            params,
        };
        let texture = canvas(gpu, width, height, WORKING_FORMAT, "export");
        Ok(Target {
            gpu: Arc::clone(gpu),
            width,
            height,
            texture: Arc::new(texture),
            work: OnceLock::new(),
            export: Some(export),
        })
    }

    /// Make the frames of an export Y'CbCr by the matrix `matrix` from now on: that of its
    /// output space (BT.2020 for Rec.2100, as its tags say), BT.709 by default. Nothing for a
    /// preview, whose frames are R'G'B'.
    pub fn set_matrix(&self, matrix: crate::YuvMatrix) {
        let Some(export) = &self.export else {
            return;
        };
        let code: u32 = match matrix {
            crate::YuvMatrix::Bt2020 => 1,
            crate::YuvMatrix::Bt601 => 2,
            _ => 0,
        };
        // The seventh word of the uniform of `convert.wgsl`.
        self.gpu
            .queue()
            .write_buffer(&export.params, 24, &code.to_le_bytes());
    }

    /// The texture that a frame ends in, after its output transform: for a preview, the
    /// picture itself, of half floats, which a view on the same card can draw.
    #[must_use]
    pub fn texture(&self) -> &Arc<wgpu::Texture> {
        &self.texture
    }

    /// The second canvas, for the stages over the canvas.
    fn work(&self) -> &wgpu::Texture {
        self.work
            .get_or_init(|| canvas(&self.gpu, self.width, self.height, WORKING_FORMAT, "work"))
    }

    #[must_use]
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// The bits of the pictures of an export; none for a preview.
    #[must_use]
    pub fn bits(&self) -> Option<u8> {
        self.export.as_ref().map(|export| export.bits)
    }

    /// The card that it is on.
    #[must_use]
    pub fn gpu(&self) -> &Arc<Gpu> {
        &self.gpu
    }

    /// Wait for what was last rendered into an export, and read it back into `picture`, in the
    /// layout of [`Picture`]: its planes of NV12 or P010, rows 64 bytes apart. The number of its
    /// frame is left as it was. The memory of `picture` is kept where it is of the size.
    ///
    /// # Errors
    /// It is not an export, or the card could not give the planes back (it was lost).
    pub fn read_picture(&self, picture: &mut Picture) -> Result<(), ComposeError> {
        let Some(export) = &self.export else {
            return Err(ComposeError::Target(
                "a preview is not read back as a picture".into(),
            ));
        };
        let buffers = [&export.luma_read, &export.chroma_read];
        wait_mapped(&self.gpu, &buffers)?;
        picture.width = self.width;
        picture.height = self.height;
        picture.bits = export.bits;
        picture.luma_stride = export.stride;
        picture.chroma_stride = export.stride;
        let read = (|| {
            for (buffer, plane) in [
                (&export.luma_read, &mut picture.luma),
                (&export.chroma_read, &mut picture.chroma),
            ] {
                let mapped = buffer
                    .slice(..)
                    .get_mapped_range()
                    .map_err(|error| ComposeError::Read(error.to_string()))?;
                if plane.len() == mapped.len() {
                    plane.copy_from_slice(&mapped);
                } else {
                    *plane = mapped.to_vec();
                }
            }
            Ok(())
        })();
        for buffer in buffers {
            buffer.unmap();
        }
        read
    }

    /// Wait for what was last rendered into a preview, and read it back as a view on a window of
    /// 8 bits shows it at its own size: R'G'B'A, 8 bits each, a row after another, `width × 4`
    /// bytes a row. For the tests, and for checks of what a view shows.
    ///
    /// # Errors
    /// It is an export, or the card could not give the picture back (it was lost).
    pub fn read_rgba(&self) -> Result<Vec<u8>, ComposeError> {
        self.read_preview(eight_bits)
    }

    /// Wait for what was last rendered into a preview, and read back its half floats as they
    /// are, beyond 0..1 too: R, G, B and A of each pixel, a row after another. For the tests of
    /// colour, which hold values of HDR and of Log to their references.
    ///
    /// # Errors
    /// As [`Target::read_rgba`].
    pub fn read_floats(&self) -> Result<Vec<f32>, ComposeError> {
        self.read_preview(half_value)
    }

    /// The half floats of a preview, each made what `each` makes of it.
    fn read_preview<T>(&self, each: impl Fn(u16) -> T) -> Result<Vec<T>, ComposeError> {
        if self.export.is_some() {
            return Err(ComposeError::Target(
                "an export is read back as a picture".into(),
            ));
        }
        if let Some(why) = self.gpu.lost_reason() {
            return Err(ComposeError::Lost(why));
        }
        let device = self.gpu.device();
        // The half floats themselves, made 8 bits here: a pass on the card into a texture of 8
        // bits would cost the thread that plays a timeline more, and it reads a frame so.
        let row = (self.width * 8).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("columbina preview read"),
            size: u64::from(row) * u64::from(self.height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("columbina preview read"),
        });
        encoder.copy_texture_to_buffer(
            self.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(self.height),
                },
            },
            self.texture.size(),
        );
        self.gpu.queue().submit([encoder.finish()]);
        wait_mapped(&self.gpu, &[&buffer])?;
        let mapped = buffer
            .slice(..)
            .get_mapped_range()
            .map_err(|error| ComposeError::Read(error.to_string()))?;
        let width = self.width as usize * 8;
        let mut pixels = Vec::with_capacity(self.width as usize * 4 * self.height as usize);
        for line in mapped.chunks(row as usize) {
            for half in line[..width].as_chunks::<2>().0 {
                pixels.push(each(u16::from_le_bytes(*half)));
            }
        }
        drop(mapped);
        buffer.unmap();
        Ok(pixels)
    }

    /// Wait for what was last rendered into a preview, and read back its half floats as they
    /// are (IEEE 754 binary16, R'G'B'A, `width × 4` a row, rows one after another), nothing
    /// clamped or rounded: the picture of the viewer, for a still of the Gallery.
    ///
    /// # Errors
    /// It is an export, or the card could not give the picture back (it was lost).
    pub fn read_halves(&self) -> Result<Vec<u16>, ComposeError> {
        if self.export.is_some() {
            return Err(ComposeError::Target(
                "an export is read back as a picture".into(),
            ));
        }
        if let Some(why) = self.gpu.lost_reason() {
            return Err(ComposeError::Lost(why));
        }
        let device = self.gpu.device();
        let row = (self.width * 8).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("columbina preview read of halves"),
            size: u64::from(row) * u64::from(self.height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("columbina preview read of halves"),
        });
        encoder.copy_texture_to_buffer(
            self.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(self.height),
                },
            },
            self.texture.size(),
        );
        self.gpu.queue().submit([encoder.finish()]);
        wait_mapped(&self.gpu, &[&buffer])?;
        let mapped = buffer
            .slice(..)
            .get_mapped_range()
            .map_err(|error| ComposeError::Read(error.to_string()))?;
        let width = self.width as usize * 8;
        let mut halves = Vec::with_capacity(self.width as usize * 4 * self.height as usize);
        for line in mapped.chunks(row as usize) {
            for half in line[..width].as_chunks::<2>().0 {
                halves.push(u16::from_le_bytes(*half));
            }
        }
        drop(mapped);
        buffer.unmap();
        Ok(halves)
    }
}

/// The value of a half float; 0 for one that is not a number.
fn half_value(half: u16) -> f32 {
    let exponent = i32::from((half >> 10) & 0x1F);
    let fraction = f32::from(half & 0x3FF);
    let magnitude = match exponent {
        0 => fraction * 2.0_f32.powi(-24),
        31 if fraction == 0.0 => f32::INFINITY,
        31 => return 0.0,
        _ => (1.0 + fraction / 1024.0) * 2.0_f32.powi(exponent - 15),
    };
    if half & 0x8000 == 0 {
        magnitude
    } else {
        -magnitude
    }
}

/// A half float as a value of 8 bits, as a texture of 8 bits takes it: 0 to 1 made 0 to 255,
/// rounded to the nearest (half way, to the even one), what is beyond clamped.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a value from 0 to 255"
)]
fn eight_bits(half: u16) -> u8 {
    (half_value(half).clamp(0.0, 1.0) * 255.0).round_ties_even() as u8
}

/// A texture that layers are composed into.
fn canvas(
    gpu: &Gpu,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
    label: &str,
) -> wgpu::Texture {
    gpu.device().create_texture(&wgpu::TextureDescriptor {
        label: Some(&format!("columbina {label}")),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

/// How long the card is given to map buffers for reading before it is taken as hung.
const READ_WITHIN: Duration = Duration::from_secs(20);

/// How long to wait between asking the card whether buffers are mapped.
const READ_POLL: Duration = Duration::from_micros(250);

/// Map `buffers` for reading, and wait until they are mapped: asking the card again and again,
/// a moment apart, without blocking in it, since a poll that waits holds a lock of the device
/// that the views take to present (an export would stall the preview, and on WARP set off the
/// watchdog of the view).
fn wait_mapped(gpu: &Gpu, buffers: &[&wgpu::Buffer]) -> Result<(), ComposeError> {
    if let Some(why) = gpu.lost_reason() {
        return Err(ComposeError::Lost(why));
    }
    let (mapped, mapping) = mpsc::channel();
    for buffer in buffers {
        let mapped = mapped.clone();
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = mapped.send(result);
            });
    }
    drop(mapped);
    let deadline = Instant::now() + READ_WITHIN;
    let mut answered = 0;
    let mut failed = None;
    while answered < buffers.len() {
        gpu.device()
            .poll(wgpu::PollType::Poll)
            .map_err(|error| ComposeError::Read(error.to_string()))?;
        loop {
            match mapping.try_recv() {
                Ok(Ok(())) => answered += 1,
                Ok(Err(error)) => {
                    answered += 1;
                    failed = Some(error.to_string());
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    if answered < buffers.len() {
                        answered = buffers.len();
                        failed.get_or_insert_with(|| "the card did not map the buffer".into());
                    }
                    break;
                }
            }
        }
        if answered >= buffers.len() {
            break;
        }
        if gpu.is_lost() || Instant::now() >= deadline {
            failed.get_or_insert_with(|| {
                format!(
                    "the card did not give it back within {} s",
                    READ_WITHIN.as_secs()
                )
            });
            break;
        }
        std::thread::sleep(READ_POLL);
    }
    if let Some(message) = failed {
        for buffer in buffers {
            buffer.unmap();
        }
        return Err(match gpu.lost_reason() {
            Some(why) => ComposeError::Lost(why),
            None => ComposeError::Read(message),
        });
    }
    Ok(())
}

/// The kinds of pipelines: those that draw layers, and that of the stages over the canvas.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Pipe {
    Yuv(Texel),
    Rgba,
    Stage,
    /// A picture blended with what is under it on the canvas, by its composite mode.
    YuvBlended(Texel),
    /// A title or a colour blended with what is under it.
    RgbaBlended,
    /// Two pictures of one media mixed (`between.wgsl`): another shader, which reads the planes
    /// of the second picture too. A layer of one picture is drawn by the kind it always was,
    /// and binds nothing more.
    YuvBetween(Texel),
    /// Two pictures mixed, and blended with what is under them on the canvas.
    YuvBetweenBlended(Texel),
    /// A pass of the composition of a clip that lays a foreground over a background
    /// (`fusion.wgsl`): it draws no layer, and into a texture of the composition alone.
    Fusion,
}

impl Pipe {
    /// The kind that draws the same layer blended with what is under it: another shader, which
    /// reads the canvas as it was before the layer (group 2). A layer that is not blended is
    /// drawn by the kind it always was, and binds nothing more.
    fn blended(self) -> Pipe {
        match self {
            Pipe::Yuv(texel) => Pipe::YuvBlended(texel),
            Pipe::Rgba => Pipe::RgbaBlended,
            Pipe::YuvBetween(texel) => Pipe::YuvBetweenBlended(texel),
            other => other,
        }
    }

    /// Whether it draws a layer blended with what is under it.
    fn is_blended(self) -> bool {
        matches!(
            self,
            Pipe::YuvBlended(_) | Pipe::RgbaBlended | Pipe::YuvBetweenBlended(_)
        )
    }
}

/// What a pipeline is known by: its kind, the format it draws into, and its program of colour.
type PipelineKey = (Pipe, wgpu::TextureFormat, Option<usize>);

/// What a layer was drawn with: by its kind of pipeline, and the code of its input transform
/// (by which the layer at the same place in the next frame is taken for the same), its program of
/// colour and what that read.
struct ShownLayer {
    pipe: Pipe,
    input: Vec<Arc<str>>,
    program: usize,
    inputs: Inputs,
}

/// What of a frame cannot be drawn yet as it is, its pipelines being made: draws (by their
/// number in the frame) and the stages over the canvas (the timeline grade, the output
/// transform).
#[derive(Default)]
struct Waiting {
    draws: Vec<usize>,
    stages: [bool; 2],
}

impl Waiting {
    fn is_empty(&self) -> bool {
        self.draws.is_empty() && !self.stages.contains(&true)
    }
}

/// The kind of pipeline that draws `draw` alone, as it is: into the textures of its
/// neighbourhoods, and onto the canvas where it is not blended.
fn pipe_of(draw: &Draw) -> Pipe {
    match draw {
        Draw::Picture {
            shape,
            second: Some(_),
            ..
        } => Pipe::YuvBetween(shape.texel),
        Draw::Picture { shape, .. } => Pipe::Yuv(shape.texel),
        Draw::Texture(..) => Pipe::Rgba,
    }
}

/// Whether `draw` is blended with what is under it on the canvas.
fn is_blended(draw: &Draw) -> bool {
    match draw {
        Draw::Picture { blended, .. } | Draw::Texture(_, blended) => *blended,
    }
}

/// The kind of pipeline that lays `draw` on the canvas.
fn canvas_pipe_of(draw: &Draw) -> Pipe {
    if is_blended(draw) {
        pipe_of(draw).blended()
    } else {
        pipe_of(draw)
    }
}

/// Of what a program read for one draw, the same again.
fn copied(inputs: &Inputs) -> Inputs {
    Inputs {
        params: inputs.params.clone(),
        textures: inputs.textures.clone(),
    }
}

/// The code of the input transform of a layer.
fn input_code(layer: &SceneLayer) -> Vec<Arc<str>> {
    layer
        .color
        .input
        .ops
        .iter()
        .map(|op| Arc::clone(&op.body))
        .collect()
}

/// The colour of a layer drawn, where it does something: the number of the layer in its scene,
/// of its program, and what the program reads.
struct Graded {
    layer: usize,
    program: usize,
    inputs: Inputs,
}

/// What is done for one of the neighbourhoods that the operations of a layer, or of a stage over
/// the canvas, read: the colour that it filters drawn into `source` (by the program numbered
/// `program`, reading `inputs`: the layer drawn by its draw with the uniform numbered `uniform`
/// of the frame, or the canvas by a pass of the stage), and that filtered, across into
/// `between` and down into `out`, which the operations after it read.
struct Tap {
    program: usize,
    inputs: Inputs,
    uniform: usize,
    source: wgpu::Texture,
    between: wgpu::Texture,
    out: wgpu::Texture,
    passes: [Pass; 2],
}

/// The neighbourhoods of a layer, and the draw (of those of the frame) that draws it.
struct LayerTaps {
    layer: usize,
    draw: usize,
    taps: Vec<Tap>,
}

/// A stage over the canvas that does something: its program, what it reads, and what is done
/// for its neighbourhoods first.
struct StageWork {
    program: usize,
    inputs: Inputs,
    taps: Vec<Tap>,
}

/// What the layers of a frame draw: the draws, their uniforms and their programs of colour, one
/// each; and what is done first for the neighbourhoods of some, with the uniforms of their draws
/// alone.
///
/// And, for the layers of clips that have compositions, what is drawn into textures before any
/// of that: the passes, in the order they are drawn; the sources of those clips, each drawn
/// alone; and the uniforms of the passes that lay a foreground over a background. A frame
/// without such a layer has none of them.
#[derive(Default)]
struct Layered {
    draws: Vec<Draw>,
    uniforms: Vec<Vec<f32>>,
    colours: Vec<Option<Graded>>,
    taps: Vec<LayerTaps>,
    tap_uniforms: Vec<Vec<f32>>,
    offscreen: Vec<Offscreen>,
    sources: Vec<Alone>,
    lays: Vec<[f32; fusion::PARAMS]>,
    /// The textures of the compositions drawn so far that no pass after reads: those of the
    /// compositions of the layers after are drawn into them.
    free: Vec<wgpu::Texture>,
    /// Whether a layer is drawn of its source alone, the pipeline of the passes of its
    /// composition being made.
    waiting: bool,
}

/// The source of a clip that has a composition, drawn alone into a texture of the
/// composition: what a `MediaIn` shows.
struct Alone {
    /// The number of its layer in the scene.
    layer: usize,
    draw: Draw,
    uniform: Vec<f32>,
    /// The input transform of the layer, where it does something.
    colour: Option<Graded>,
    /// The texture that it is drawn into.
    target: Option<wgpu::Texture>,
}

/// A pass that draws into a texture of the composition of a clip.
#[allow(
    clippy::large_enum_variant,
    reason = "a frame has a few of them, made and dropped within it"
)]
enum Offscreen {
    /// The source of the clip, alone: the one of this number among those of the frame.
    Source(usize),
    /// A colour over the whole picture, premultiplied.
    Fill {
        colour: [f32; 4],
        target: wgpu::Texture,
    },
    /// A foreground laid over a background, or over nothing, by the uniform of this number
    /// among those of the frame.
    Lay {
        uniform: usize,
        background: Option<wgpu::Texture>,
        foreground: wgpu::Texture,
        target: wgpu::Texture,
    },
}

/// What a layer of a clip that has a composition shows.
enum Composed {
    /// Nothing: no picture comes out of the composition, or its crop keeps none of it.
    Nothing,
    /// The source of the clip, as the layer of a clip without a composition shows it.
    Source,
    /// The picture that comes out of the composition.
    Picture(Outcome),
}

/// The picture that comes out of the composition of a clip, to be laid on the canvas.
struct Outcome {
    view: wgpu::TextureView,
    /// The size that it is drawn at.
    size: (u32, u32),
    /// The frame that it is fitted into on the canvas.
    frame: (f64, f64),
    /// What the crop of the layer keeps of that frame.
    kept: [f64; 4],
    /// Whether the input transform of the layer is done to its source, before the
    /// composition; else it is done after, with the grade.
    early: bool,
}

/// What a texture that the passes of a frame share is drawn into with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Role {
    /// The colour that a neighbourhood filters.
    Source,
    /// The first pass of its filter.
    Between,
    /// The canvas as it is before a layer that is blended with it is drawn.
    Backdrop,
}

/// The textures of neighbourhoods, kept from frame to frame by their sizes: those lent to a
/// frame, each to one use, and those shared within it by the passes that draw into them one
/// after another, one of each size for each [`Role`]. The pictures of the compositions of
/// clips are drawn into textures lent from them too (`fusion.rs`).
#[derive(Default)]
struct Spares {
    kept: HashMap<(u32, u32), Vec<(wgpu::Texture, u64)>>,
    lent: Vec<wgpu::Texture>,
    shared: HashMap<((u32, u32), Role), wgpu::Texture>,
}

impl Spares {
    /// A texture of `size` of its own for this frame.
    fn take(&mut self, gpu: &Gpu, size: (u32, u32)) -> wgpu::Texture {
        let texture = self.kept_or_made(gpu, size);
        self.lent.push(texture.clone());
        texture
    }

    /// The texture of `size` that the passes of this frame in `role` share.
    fn shared(&mut self, gpu: &Gpu, size: (u32, u32), role: Role) -> wgpu::Texture {
        if let Some(texture) = self.shared.get(&(size, role)) {
            return texture.clone();
        }
        let texture = self.kept_or_made(gpu, size);
        self.shared.insert((size, role), texture.clone());
        texture
    }

    fn kept_or_made(&mut self, gpu: &Gpu, size: (u32, u32)) -> wgpu::Texture {
        self.kept.get_mut(&size).and_then(Vec::pop).map_or_else(
            || canvas(gpu, size.0, size.1, NEIGHBOURHOOD_FORMAT, "neighbourhood"),
            |(texture, _)| texture,
        )
    }

    /// Keep the textures of a frame, numbered `frame`, for the next; let go of those of sizes
    /// not drawn for a while.
    fn settle(&mut self, frame: u64) {
        for texture in self
            .lent
            .drain(..)
            .chain(self.shared.drain().map(|(_, t)| t))
        {
            let size = (texture.width(), texture.height());
            self.kept.entry(size).or_default().push((texture, frame));
        }
        for kept in self.kept.values_mut() {
            kept.retain(|(_, used)| used + SPARES_KEPT_FOR >= frame);
        }
        self.kept.retain(|_, kept| !kept.is_empty());
    }

    /// The bytes of the textures kept.
    fn bytes(&self) -> u64 {
        self.kept
            .iter()
            .map(|((width, height), kept)| {
                u64::from(*width) * u64::from(*height) * 8 * kept.len() as u64
            })
            .sum()
    }
}

/// A layer, or the subtitles, to be drawn.
struct Call {
    /// The kind of pipeline that draws it as it is.
    pipe: Pipe,
    program: Option<usize>,
    bind: wgpu::BindGroup,
    offset: u32,
    colour: Option<wgpu::BindGroup>,
    /// Whether it is blended with what is under it on the canvas, by the pipeline of
    /// [`Pipe::blended`].
    blended: bool,
}

/// The shape of the planes of a picture: their sizes and formats follow from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Shape {
    width: u32,
    height: u32,
    format: PictureFormat,
    /// Whether it has a plane of alpha (of Y'CbCr).
    alpha: bool,
    texel: Texel,
}

/// The textures of the planes of a picture of a shape, kept to be uploaded into again.
struct Planes {
    textures: yuv::PlaneTextures,
    /// What binds them, with the uniform of its generation.
    bind: Option<(u64, wgpu::BindGroup)>,
    /// The frame it was last drawn in.
    used: u64,
}

/// What a text laid out is known by: all that changes its pixels.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct TextKey {
    text: String,
    family: Option<String>,
    size: u32,
    color: [u8; 4],
    align: TextAlign,
    outline: u32,
}

impl TextKey {
    fn of(spec: &Spec) -> TextKey {
        TextKey {
            text: spec.text.clone(),
            family: spec.family.clone(),
            size: spec.size.to_bits(),
            color: spec.color,
            align: spec.align,
            outline: spec.outline,
        }
    }
}

/// A text laid out, and the parts of it drawn into pixels on the card, the last drawn first.
struct LaidText {
    laid: Laid,
    parts: Vec<TextPart>,
    used: u64,
}

/// A part of a text drawn into pixels: x0, y0, x1, y1 of its pixels, on the card.
struct TextPart {
    window: [u32; 4],
    view: wgpu::TextureView,
    bytes: u64,
}

/// Where a text is put.
enum Put {
    /// Over this quad of its frame, with this texture, of this size.
    Drawn([f64; 4], wgpu::TextureView, (u32, u32)),
    /// Nowhere on the canvas.
    Unseen,
    /// Too large to draw at once: to be laid out this many times smaller, at least.
    TooLarge(f64),
}

/// A text put on the canvas: the quad of its frame that it covers, its texture, and the size of
/// that.
type Fitted = ([f64; 4], wgpu::TextureView, (u32, u32));

/// What is drawn for a layer; and whether it is blended with what is under it on the canvas
/// (its composite mode is not Normal, and a layer is drawn before it).
#[allow(
    clippy::large_enum_variant,
    reason = "a frame has a few of them, made and dropped within it: a box would be an allocation for each layer of pictures"
)]
enum Draw {
    Picture {
        shape: Shape,
        planes: Planes,
        /// The planes of the second picture, of the same shape, of a layer that mixes two
        /// ([`Source::Between`]).
        second: Option<Planes>,
        blended: bool,
    },
    Texture(wgpu::TextureView, bool),
}

/// The pictures that a layer of pictures draws.
struct Pictures<'a> {
    /// The picture, or the first of two.
    first: &'a Arc<NativePicture>,
    /// The second of two, and how much of it is mixed into the first: above 0 and below 1.
    second: Option<(&'a Arc<NativePicture>, f32)>,
    /// How many degrees clockwise they are turned to be shown.
    rotation: i16,
}

/// The pictures that a layer of `source` draws, if it is one of pictures: one, or two mixed. Of
/// two of which all of one is shown, that one alone, as a [`Source::Picture`] of it is drawn:
/// the first at a weight of 0 or less, or one that is no number, and the second at 1 or more.
fn pictures_of(source: &Source) -> Option<Pictures<'_>> {
    match source {
        Source::Picture(picture, rotation) => Some(Pictures {
            first: picture,
            second: None,
            rotation: *rotation,
        }),
        Source::Between {
            first,
            second,
            weight,
            rotation,
        } => {
            let (first, second) = if weight.is_nan() || *weight <= 0.0 {
                (first, None)
            } else if *weight >= 1.0 {
                (second, None)
            } else {
                (first, Some((second, *weight)))
            };
            Some(Pictures {
                first,
                second,
                rotation: *rotation,
            })
        }
        Source::Title(_) | Source::Color(_) | Source::Fused(_) => None,
    }
}

/// Puts the layers of scenes together on a card. It keeps what it made from frame to frame: the
/// pipelines, the textures of the planes of pictures of each shape, and the titles drawn.
pub struct Compositor {
    gpu: Arc<Gpu>,
    yuv_layouts: HashMap<Texel, wgpu::BindGroupLayout>,
    /// What binds the planes of two pictures that are mixed (`between.wgsl`), by how their
    /// texels are read: made as they are first needed, by a frame with such a layer.
    between_layouts: HashMap<Texel, wgpu::BindGroupLayout>,
    rgba_layout: wgpu::BindGroupLayout,
    stage_layout: wgpu::BindGroupLayout,
    convert_layout: wgpu::BindGroupLayout,
    /// What binds what a program of colour reads (group 1), by the kinds of its textures.
    colour_layouts: HashMap<Vec<TextureKind>, wgpu::BindGroupLayout>,
    /// The programs of colour met, by their code; each is known by its place here.
    programs: Vec<Program>,
    program_numbers: HashMap<String, usize>,
    /// The pipelines, by their kind, the format they draw into, and their program of colour.
    pipelines: HashMap<PipelineKey, wgpu::RenderPipeline>,
    /// What makes the pipelines of programs of colour, on threads of its own.
    builder: Builder<PipelineKey>,
    /// The pipelines that could not be made, and why.
    failed: HashMap<PipelineKey, String>,
    /// Whether a frame is drawn without waiting for the pipelines of its programs of colour
    /// ([`Compositor::compile_in_background`]).
    background: bool,
    /// What the last frames drew each layer (by its number in the scene) and each stage over
    /// the canvas with, for the frames drawn while their pipelines are made.
    shown_layers: HashMap<usize, ShownLayer>,
    shown_stages: [Option<(usize, Inputs)>; 2],
    /// Whether the last frame was drawn with programs other than its own, as its own were being
    /// made.
    provisional: bool,
    convert: Option<(wgpu::ComputePipeline, wgpu::ComputePipeline)>,
    sampler: wgpu::Sampler,
    /// The sampler that operations of colour read their textures with.
    colour_sampler: wgpu::Sampler,
    /// The parameters of the programs of colour of a frame, each at a place of its own.
    colour_params: wgpu::Buffer,
    /// A texel of white, for colours.
    white: wgpu::TextureView,
    /// The uniforms of the layers of a frame, one slot of `slot` bytes each.
    uniform: wgpu::Buffer,
    slot: u32,
    /// Changed when the uniform is made anew, so that what binds it is made anew too.
    generation: u64,
    planes: HashMap<Shape, Vec<Planes>>,
    texts: HashMap<TextKey, LaidText>,
    /// How many parts of texts it has drawn into pixels since it was made.
    text_draws: u64,
    /// What binds the uniform of a pass of a filter and its source, and its pipeline, made as
    /// it is first needed.
    filter_layout: wgpu::BindGroupLayout,
    filter: Option<wgpu::RenderPipeline>,
    /// The uniforms of the passes of filters of a frame, one slot of `slot` bytes each.
    filter_uniform: wgpu::Buffer,
    spares: Spares,
    /// The pipelines that make a canvas opaque (`opaque.wgsl`), by its format, made as they
    /// are first needed: by a frame with a layer that is blended.
    opaque: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
    /// What the passes of compositions that lay a foreground over a background are drawn
    /// with, made as it is first needed: by a frame with a layer of a clip whose composition
    /// has such a pass.
    fusion: Option<fusion::Card>,
    /// How many passes it has drawn into textures of compositions since it was made.
    fusion_passes: u64,
    /// The number of the frame being composed.
    frame: u64,
}

impl std::fmt::Debug for Compositor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Compositor")
            .field("gpu", &self.gpu)
            .field("frame", &self.frame)
            .finish_non_exhaustive()
    }
}

impl Compositor {
    /// A compositor on `gpu`. Pipelines are made as they are first needed.
    #[must_use]
    #[allow(
        clippy::too_many_lines,
        reason = "the layouts of the bindings, one after another"
    )]
    pub fn new(gpu: Arc<Gpu>) -> Self {
        // The fonts of the system, read on a thread of their own before a title is drawn.
        text::warm_up();
        let device = gpu.device();
        let uniform_entry = |size: u64| wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: true,
                min_binding_size: wgpu::BufferSize::new(size),
            },
            count: None,
        };
        let rgba_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("columbina layer"),
            entries: &[
                uniform_entry(LAYER_PARAMS_SIZE),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let yuv_layouts = [Texel::Float, Texel::Uint]
            .into_iter()
            .map(|texel| {
                let plane = |binding| wgpu::BindGroupLayoutEntry {
                    binding,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: texel.sample_type(),
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                };
                let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("columbina picture"),
                    entries: &[
                        uniform_entry(yuv::PARAMS_SIZE),
                        plane(1),
                        plane(2),
                        plane(3),
                    ],
                });
                (texel, layout)
            })
            .collect();
        let storage = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: false },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let convert_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("columbina convert"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                storage(1),
                storage(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(CONVERT_PARAMS_SIZE),
                    },
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("columbina layer"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..wgpu::SamplerDescriptor::default()
        });
        let colour_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("columbina colour"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..wgpu::SamplerDescriptor::default()
        });
        let white = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("columbina white"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        gpu.queue().write_texture(
            white.as_image_copy(),
            &[255; 4],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: Some(1),
            },
            white.size(),
        );
        let alignment = device.limits().min_uniform_buffer_offset_alignment;
        let slot = u32::try_from(yuv::PARAMS_SIZE.max(LAYER_PARAMS_SIZE))
            .unwrap_or(256)
            .next_multiple_of(alignment);
        let uniform = uniform_buffer(device, u64::from(slot) * 8);
        let filter_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("columbina filter"),
            entries: &[
                uniform_entry(FILTER_PARAMS_SIZE),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let filter_uniform = uniform_buffer(device, u64::from(slot) * 4);
        Compositor {
            filter_layout,
            filter: None,
            filter_uniform,
            spares: Spares::default(),
            opaque: HashMap::new(),
            fusion: None,
            fusion_passes: 0,
            yuv_layouts,
            between_layouts: HashMap::new(),
            rgba_layout,
            stage_layout: stage_layout(device),
            convert_layout,
            colour_layouts: HashMap::new(),
            programs: Vec::new(),
            program_numbers: HashMap::new(),
            pipelines: HashMap::new(),
            builder: Builder::new(Arc::clone(&gpu)),
            failed: HashMap::new(),
            background: false,
            shown_layers: HashMap::new(),
            shown_stages: [None, None],
            provisional: false,
            convert: None,
            sampler,
            colour_sampler,
            colour_params: colour_buffer(device, 4096),
            white: white.create_view(&wgpu::TextureViewDescriptor::default()),
            uniform,
            slot,
            generation: 0,
            planes: HashMap::new(),
            texts: HashMap::new(),
            text_draws: 0,
            frame: 0,
            gpu,
        }
    }

    /// The card that it composes on.
    #[must_use]
    pub fn gpu(&self) -> &Arc<Gpu> {
        &self.gpu
    }

    /// Draw each frame without waiting for the pipelines of its programs of colour, which are
    /// made on threads of their own, and call `wake` (on such a thread) each time one is made.
    ///
    /// Until the pipeline of a layer is made, the layer is drawn as the last frame drew the
    /// layer at the same place in the scene, by that program and with what it read then, where
    /// that layer had the same input transform (the same media, as a rule): a grade being
    /// changed is shown as it was, and then as it is. Else the layer is drawn with no colour of
    /// its own, its input transform too. A layer of two pictures mixed ([`Source::Between`]) is
    /// drawn first as its first picture alone, where a layer of one picture is drawn with its
    /// colour already. A layer of a clip that has a composition ([`Source::Fused`]) is drawn
    /// of its source alone, as that of a clip without one, until the pipeline of the passes of
    /// compositions is made; and its source is drawn into the composition without its input
    /// transform until the pipeline of that is. A stage over the canvas (the timeline grade,
    /// the output transform) is done as the last frame did it, or not at such a frame. Such a
    /// frame is [`Compositor::is_provisional`]: a preview draws it, and again once `wake` is
    /// called; an export is to wait ([`Compositor::finish_compiling`]), or use a compositor of
    /// its own, which waits.
    pub fn compile_in_background(&mut self, wake: impl Fn() + Send + Sync + 'static) {
        self.background = true;
        self.builder.wake_with(Arc::new(wake));
    }

    /// Compose `scene` into `target` as [`Compositor::render`] does, but waiting for the
    /// pipelines of its programs of colour whether or not it draws frames without waiting
    /// ([`Compositor::compile_in_background`]): for a picture that is kept (a reference that a
    /// frame is compared with), not a frame of a monitor. What the frames drawn without waiting
    /// are drawn with, and whether the last was provisional, are left as they were.
    ///
    /// # Errors
    /// As [`Compositor::render`].
    pub fn render_waiting(&mut self, scene: &Scene, target: &Target) -> Result<(), ComposeError> {
        let (background, provisional) = (self.background, self.provisional);
        self.background = false;
        let rendered = self.render(scene, target);
        (self.background, self.provisional) = (background, provisional);
        rendered
    }

    /// Whether the last frame was drawn otherwise than its scene says, as the pipelines of some
    /// of its programs of colour were being made (see [`Compositor::compile_in_background`]).
    #[must_use]
    pub fn is_provisional(&self) -> bool {
        self.provisional
    }

    /// Whether pipelines are being made.
    #[must_use]
    pub fn is_compiling(&self) -> bool {
        self.builder.is_busy()
    }

    /// Wait until the pipelines being made are made: a frame drawn after is drawn as its scene
    /// says, as far as the pipelines asked for so far go.
    pub fn finish_compiling(&mut self) {
        let made = self.builder.finish();
        self.keep(made);
    }

    /// How many pipelines it has made: for the tests, that a frame makes none it need not.
    #[doc(hidden)]
    #[must_use]
    pub fn pipelines_made(&self) -> usize {
        self.pipelines.len()
    }

    /// The bytes of the textures of neighbourhoods, and of the pictures of compositions, kept
    /// for the frames to come.
    #[doc(hidden)]
    #[must_use]
    pub fn neighbourhood_bytes(&self) -> u64 {
        self.spares.bytes()
    }

    /// How many parts of texts it has drawn into pixels since it was made: for the tests, that
    /// a title that only moves is not drawn again.
    #[doc(hidden)]
    #[must_use]
    pub fn texts_drawn(&self) -> u64 {
        self.text_draws
    }

    /// How many passes it has drawn into textures of compositions since it was made (the
    /// sources of clips drawn alone, and the passes of their nodes): for the tests, that a
    /// layer without a composition, and one whose composition shows its source, draw none.
    #[doc(hidden)]
    #[must_use]
    pub fn fusion_passes(&self) -> u64 {
        self.fusion_passes
    }

    /// Merge the pixels `foreground` with the pixels `background`, both of the size of the
    /// preview `target` and row after row, by `settings` on the card, into `target`: a pass of
    /// a composition as it lays a foreground that lies where it is, for the tests that hold
    /// its shader to [`crate::merge::merge_pixel`]. The pixels are given to the card as they
    /// are, floats of 32 bits; `luma` has the weights of R, G and B in the luma of the working
    /// space.
    ///
    /// # Errors
    /// The card was lost; the target is not a preview of this card, or the pictures are not
    /// of its size; or the pipeline of the pass cannot be made.
    #[doc(hidden)]
    pub fn merge_for_tests(
        &mut self,
        background: &[[f32; 4]],
        foreground: &[[f32; 4]],
        settings: &crate::merge::Settings,
        luma: [f32; 3],
        target: &Target,
    ) -> Result<(), ComposeError> {
        if let Some(why) = self.gpu.lost_reason() {
            return Err(ComposeError::Lost(why));
        }
        let (width, height) = target.size();
        let pixels = width as usize * height as usize;
        if !Arc::ptr_eq(&self.gpu, &target.gpu)
            || target.export.is_some()
            || background.len() != pixels
            || foreground.len() != pixels
        {
            return Err(ComposeError::Target(
                "two pictures of the size of a preview of this card are merged into it".into(),
            ));
        }
        let not_made = || ComposeError::Color("the pipeline of compositions is not made".into());
        let key = (Pipe::Fusion, fusion::FORMAT, None);
        self.keep_made();
        if !self
            .pipeline(key.0, key.1, key.2)
            .map_err(ComposeError::Color)?
        {
            self.finish_compiling();
            if !self
                .pipeline(key.0, key.1, key.2)
                .map_err(ComposeError::Color)?
            {
                return Err(not_made());
            }
        }
        let under = fusion::floats(&self.gpu, width, background);
        let over = fusion::floats(&self.gpu, width, foreground);
        let uniform = fusion::merge_uniform((width, height), settings, luma);
        let offsets = self.write_lays(&[uniform]);
        let (Some(card), Some(pipeline), Some(offset)) =
            (&self.fusion, self.pipelines.get(&key), offsets.first())
        else {
            return Err(not_made());
        };
        let device = self.gpu.device();
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("columbina merge"),
        });
        card.lay(
            device,
            &mut encoder,
            pipeline,
            *offset,
            Some(&under),
            &over,
            &target.texture,
        );
        self.gpu.queue().submit([encoder.finish()]);
        Ok(())
    }

    /// How many texts laid out it keeps: for the tests, that a title whose size changes at
    /// every frame leaves no more of them than a few.
    #[doc(hidden)]
    #[must_use]
    pub fn texts_kept(&self) -> usize {
        self.texts.len()
    }

    /// Compose `scene` into `target`, which is of its size and on the same card. It returns once
    /// the work is handed to the card; an export is read back with [`Target::read_picture`].
    ///
    /// # Errors
    /// The card was lost, the target is not one for the scene, or a layer cannot be drawn
    /// (nothing is drawn then).
    #[allow(clippy::too_many_lines, reason = "one frame, step by step")]
    pub fn render(&mut self, scene: &Scene, target: &Target) -> Result<(), ComposeError> {
        if let Some(why) = self.gpu.lost_reason() {
            return Err(ComposeError::Lost(why));
        }
        if !Arc::ptr_eq(&self.gpu, &target.gpu) {
            return Err(ComposeError::Target(
                "the target is on another graphics card".into(),
            ));
        }
        if (scene.width, scene.height) != target.size() {
            return Err(ComposeError::Target(format!(
                "a scene of {}×{} is composed into a target of {}×{}",
                scene.width, scene.height, target.width, target.height
            )));
        }
        self.frame += 1;
        self.provisional = false;
        let canvas = (f64::from(scene.width), f64::from(scene.height));
        let whole = [0.0, 0.0, canvas.0, canvas.1];
        let mut laid = Layered::default();
        if let Err(error) = self.layers(scene, canvas, whole, &mut laid) {
            self.give_back(laid.draws, laid.sources);
            return Err(error);
        }
        let Layered {
            mut draws,
            mut uniforms,
            mut colours,
            mut taps,
            tap_uniforms,
            offscreen,
            mut sources,
            lays,
            waiting: fusing,
            free: _,
        } = laid;
        // The stages over the canvas, where they do something.
        let size = (scene.width, scene.height);
        let mut grade = self.stage_work(&scene.grade, size);
        let mut output = self.stage_work(&scene.output, size);
        let layered = draws.len();
        if target.export.is_none() && !scene.subtitles.is_empty() {
            let size = subtitle_size(scene.height);
            let place = placement(canvas, canvas, &Transform::NONE);
            #[allow(clippy::cast_possible_truncation, reason = "a size of letters")]
            let put = self.put_fitted(
                |per| Spec::subtitles(&scene.subtitles, (f64::from(size) * per) as f32),
                1.0,
                |block| subtitle_origin((scene.width, scene.height), block),
                &place,
                canvas,
            );
            match put {
                Ok(Some((quad, view, _))) => {
                    uniforms.push(layer_uniform(
                        &place,
                        canvas,
                        quad,
                        1.0,
                        1.0,
                        1,
                        Some([1.0; 4]),
                    ));
                    draws.push(Draw::Texture(view, false));
                    colours.push(None);
                }
                Ok(None) => {}
                Err(message) => {
                    self.give_back(draws, sources);
                    return Err(ComposeError::Layer {
                        layer: scene.layers.len(),
                        message,
                    });
                }
            }
        }

        // The uniforms of all the layers, one slot each; then of the layers as their
        // neighbourhoods draw them; and then of the sources of the clips that have
        // compositions, as they are drawn alone.
        let drawn = uniforms.len();
        uniforms.extend(tap_uniforms);
        let tapped = uniforms.len();
        uniforms.extend(
            sources
                .iter_mut()
                .map(|source| std::mem::take(&mut source.uniform)),
        );
        let mut offsets = match self.write_uniforms(&uniforms) {
            Ok(offsets) => offsets,
            Err(error) => {
                self.give_back(draws, sources);
                return Err(error);
            }
        };
        let source_offsets = offsets.split_off(tapped);
        let tap_offsets = offsets.split_off(drawn);

        let format = target.texture.format();
        let made = self
            .ask_for_sources(&sources)
            .and_then(|()| self.pipelines_of(&draws, &colours, &taps, [&grade, &output], format));
        let mut waiting = match made {
            Ok(waiting) => waiting,
            Err(error) => {
                self.give_back(draws, sources);
                return Err(error);
            }
        };
        self.provisional = !waiting.is_empty() || fusing;
        if self.background {
            self.first_pictures_alone(&mut draws, &colours, &taps, &mut waiting.draws, format);
            let stood = self.stand_in_for_layers(
                scene,
                &draws,
                &mut colours,
                &mut taps,
                &waiting.draws,
                format,
            );
            if let Err(error) = stood {
                self.give_back(draws, sources);
                return Err(error);
            }
            grade = self.stand_in_for_stage(0, grade, waiting.stages[0], format);
            output = self.stand_in_for_stage(1, output, waiting.stages[1], format);
        }
        if let Err(error) = self.sources_made(&mut sources) {
            self.give_back(draws, sources);
            return Err(error);
        }
        let mut calls = Vec::with_capacity(draws.len());
        for (draw, offset) in draws.iter_mut().zip(offsets) {
            calls.push(self.call_of(draw, offset));
        }
        let mut source_calls = Vec::with_capacity(sources.len());
        for (source, offset) in sources.iter_mut().zip(source_offsets) {
            source_calls.push(self.call_of(&mut source.draw, offset));
        }
        // What the layers that are blended read of the canvas under them: a texture of its
        // size that the canvas is copied into before each is drawn, bound as their group 2;
        // and the pipeline that makes the canvas opaque after them. A frame without such a
        // layer has neither.
        let backdrop = if calls.iter().any(|call| call.blended) {
            if let Entry::Vacant(opaque) = self.opaque.entry(format) {
                opaque.insert(opaque_pipeline(self.gpu.device(), format));
            }
            let texture = self.spares.shared(&self.gpu, size, Role::Backdrop);
            let bind = stage_bind(self.gpu.device(), &self.stage_layout, &texture);
            Some((texture, bind))
        } else {
            None
        };
        // What the programs of colour read: those of the layers, of the stages, of the
        // neighbourhoods of the layers and of the stages, and of the sources of the clips
        // that have compositions, in that order.
        let stage_taps = [&grade, &output]
            .into_iter()
            .flatten()
            .flat_map(|work| &work.taps);
        let mut blocks: Vec<(usize, &Inputs)> = Vec::new();
        for colour in colours.iter().flatten() {
            blocks.push((colour.program, &colour.inputs));
        }
        for work in [&grade, &output].into_iter().flatten() {
            blocks.push((work.program, &work.inputs));
        }
        for tap in taps.iter().flat_map(|layer| &layer.taps).chain(stage_taps) {
            blocks.push((tap.program, &tap.inputs));
        }
        let before_sources = blocks.len();
        for colour in sources.iter().filter_map(|source| source.colour.as_ref()) {
            blocks.push((colour.program, &colour.inputs));
        }
        let mut colour_binds = self.colour_binds(&blocks);
        let mut source_binds = colour_binds
            .split_off(before_sources.min(colour_binds.len()))
            .into_iter();
        let mut colour_binds = colour_binds.into_iter();
        for (call, source) in source_calls.iter_mut().zip(&sources) {
            if let Some(colour) = &source.colour {
                call.program = Some(colour.program);
                call.colour = source_binds.next();
            }
        }
        for (call, colour) in calls.iter_mut().zip(&colours) {
            if let Some(colour) = colour {
                call.program = Some(colour.program);
                call.colour = colour_binds.next();
            }
        }
        let grade_bind = grade.as_ref().and_then(|_| colour_binds.next());
        let output_bind = output.as_ref().and_then(|_| colour_binds.next());
        let mut tap_binds = colour_binds;
        // The uniforms of the passes of the filters, in the order they are done.
        let filter_passes: Vec<Pass> = taps
            .iter()
            .flat_map(|layer| &layer.taps)
            .chain(
                [&grade, &output]
                    .into_iter()
                    .flatten()
                    .flat_map(|work| &work.taps),
            )
            .flat_map(|tap| tap.passes)
            .collect();
        let mut filter_offsets = self.write_filter_uniforms(&filter_passes).into_iter();
        // The uniforms of the passes of compositions that lay a foreground over a background.
        let lay_offsets = self.write_lays(&lays);

        // The canvases drawn into, one after the other: the frame ends in the target's texture.
        let passes = usize::from(grade.is_some()) + usize::from(output.is_some());
        let main: &wgpu::Texture = &target.texture;
        let work = if passes > 0 { target.work() } else { main };
        let other = |texture: &wgpu::Texture| -> &wgpu::Texture {
            if std::ptr::eq(texture, main) {
                work
            } else {
                main
            }
        };
        let mut current = if passes % 2 == 1 { other(main) } else { main };
        let device = self.gpu.device();
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("columbina compose"),
        });
        // The pictures of the clips that have compositions: the source of each drawn alone,
        // and then the passes of its composition, before the neighbourhoods of the layers that
        // show them and before those layers.
        self.draw_offscreen(
            &mut encoder,
            &offscreen,
            &sources,
            &source_calls,
            &lay_offsets,
        );
        // The neighbourhoods of the layers, each layer drawn into a texture of its own and that
        // filtered, before the layers are laid on the canvas.
        for layer in &taps {
            let call = &calls[layer.draw];
            for (tap, colour) in layer.taps.iter().zip(&mut tap_binds) {
                self.tap_pass(&mut encoder, call, tap, tap_offsets[tap.uniform], &colour);
                self.filter_passes(&mut encoder, tap, &mut filter_offsets);
            }
        }
        // The layers, and the subtitles over them where there is no timeline grade.
        let first = if grade.is_some() {
            layered
        } else {
            calls.len()
        };
        let under = backdrop.as_ref();
        self.draw_calls(&mut encoder, current, &calls[..first], format, true, under);
        if let (Some(work), Some(colour)) = (&grade, &grade_bind) {
            for (tap, bind) in work.taps.iter().zip(&mut tap_binds) {
                self.stage_pass(&mut encoder, current, &tap.source, tap.program, &bind);
                self.filter_passes(&mut encoder, tap, &mut filter_offsets);
            }
            let next = other(current);
            self.stage_pass(&mut encoder, current, next, work.program, colour);
            current = next;
            if first < calls.len() {
                self.draw_calls(&mut encoder, current, &calls[first..], format, false, under);
            }
        }
        if let (Some(work), Some(colour)) = (&output, &output_bind) {
            for (tap, bind) in work.taps.iter().zip(&mut tap_binds) {
                self.stage_pass(&mut encoder, current, &tap.source, tap.program, &bind);
                self.filter_passes(&mut encoder, tap, &mut filter_offsets);
            }
            let next = other(current);
            self.stage_pass(&mut encoder, current, next, work.program, colour);
            current = next;
        }
        debug_assert!(std::ptr::eq(current, main), "the frame ends in the target");
        if let Some(export) = &target.export {
            let view = main.create_view(&wgpu::TextureViewDescriptor::default());
            self.convert(&mut encoder, &view, export, target.height);
        }
        self.gpu.queue().submit([encoder.finish()]);
        self.give_back(draws, sources);
        self.forget_unused();
        Ok(())
    }

    /// What draws `draw` with the uniform at `offset`: the kind of its pipeline and what binds
    /// what it reads, with no program of colour yet.
    fn call_of(&mut self, draw: &mut Draw, offset: u32) -> Call {
        let blended = is_blended(draw);
        let (pipe, bind) = match draw {
            Draw::Picture {
                shape,
                planes,
                second: Some(second),
                ..
            } => (
                Pipe::YuvBetween(shape.texel),
                self.pictures_bind(shape.texel, planes, second),
            ),
            Draw::Picture { shape, planes, .. } => {
                let bind = match &planes.bind {
                    Some((generation, bind)) if *generation == self.generation => bind.clone(),
                    _ => {
                        let bind = self.planes_bind(shape.texel, planes);
                        planes.bind = Some((self.generation, bind.clone()));
                        bind
                    }
                };
                (Pipe::Yuv(shape.texel), bind)
            }
            Draw::Texture(view, _) => (Pipe::Rgba, self.texture_bind(view)),
        };
        Call {
            pipe,
            program: None,
            bind,
            offset,
            colour: None,
            blended,
        }
    }

    /// Ask for the pipelines that draw the sources of the clips that have compositions alone,
    /// each with the input transform of its layer, where they are not made or asked for yet:
    /// before those of the layers are waited for, so that these are waited for with them.
    ///
    /// # Errors
    /// The input transform of a layer is not WGSL that the card takes.
    fn ask_for_sources(&mut self, sources: &[Alone]) -> Result<(), ComposeError> {
        for source in sources {
            let program = source.colour.as_ref().map(|colour| colour.program);
            self.pipeline(pipe_of(&source.draw), fusion::FORMAT, program)
                .map_err(|message| ComposeError::Layer {
                    layer: source.layer,
                    message,
                })?;
        }
        Ok(())
    }

    /// See that the pipeline that draws each source of a clip that has a composition alone is
    /// made. One that is being made still, where frames are drawn without waiting
    /// ([`Compositor::compile_in_background`]), is stood in for, and the frame is provisional:
    /// two pictures mixed are drawn as the first of them alone, where the pipeline of one
    /// picture with the same colour is made (as [`Compositor::first_pictures_alone`] draws a
    /// layer), and else the source is drawn without its input transform.
    ///
    /// # Errors
    /// The pipeline of a source cannot be made.
    fn sources_made(&mut self, sources: &mut [Alone]) -> Result<(), ComposeError> {
        for source in sources {
            let layer = source.layer;
            let failed = |message: String| ComposeError::Layer { layer, message };
            let pipe = pipe_of(&source.draw);
            let program = source.colour.as_ref().map(|colour| colour.program);
            if self
                .pipeline(pipe, fusion::FORMAT, program)
                .map_err(failed)?
            {
                continue;
            }
            self.provisional = true;
            if let Draw::Picture { shape, second, .. } = &mut source.draw {
                let one = (Pipe::Yuv(shape.texel), fusion::FORMAT, program);
                if second.is_some() && self.pipelines.contains_key(&one) {
                    self.planes.entry(*shape).or_default().extend(second.take());
                    continue;
                }
            }
            source.colour = None;
            self.pipeline(pipe, fusion::FORMAT, None).map_err(failed)?;
        }
        Ok(())
    }

    /// Write the uniforms of the passes of compositions of a frame that lay a foreground over
    /// a background, a slot each; give the offset of each. A frame without such a pass writes
    /// nothing, and makes nothing to write them into.
    fn write_lays(&mut self, lays: &[[f32; fusion::PARAMS]]) -> Vec<u32> {
        if lays.is_empty() {
            return Vec::new();
        }
        let slot = self.slot;
        let card = self
            .fusion
            .get_or_insert_with(|| fusion::Card::new(&self.gpu, slot));
        card.write(&self.gpu, slot, lays)
    }

    /// Draw the passes of the compositions of a frame into their textures, in their order:
    /// the sources of the clips alone, by their `calls`, and the passes of their nodes, those
    /// that lay a foreground over a background by the uniforms at `lays`.
    fn draw_offscreen(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        offscreen: &[Offscreen],
        sources: &[Alone],
        calls: &[Call],
        lays: &[u32],
    ) {
        let laying = (Pipe::Fusion, fusion::FORMAT, None);
        for pass in offscreen {
            match pass {
                Offscreen::Source(number) => {
                    let target = sources
                        .get(*number)
                        .and_then(|source| source.target.as_ref());
                    let (Some(target), Some(call)) = (target, calls.get(*number)) else {
                        continue;
                    };
                    self.source_pass(encoder, call, target);
                }
                Offscreen::Fill { colour, target } => fusion::fill(encoder, target, *colour),
                Offscreen::Lay {
                    uniform,
                    background,
                    foreground,
                    target,
                } => {
                    let (Some(card), Some(pipeline), Some(offset)) = (
                        &self.fusion,
                        self.pipelines.get(&laying),
                        lays.get(*uniform),
                    ) else {
                        continue;
                    };
                    card.lay(
                        self.gpu.device(),
                        encoder,
                        pipeline,
                        *offset,
                        background.as_ref(),
                        foreground,
                        target,
                    );
                }
            }
            self.fusion_passes += 1;
        }
    }

    /// Draw the source of a clip that has a composition alone into `target`, a texture of the
    /// composition, by `call`: over nothing, so that what it does not cover is transparent.
    fn source_pass(&self, encoder: &mut wgpu::CommandEncoder, call: &Call, target: &wgpu::Texture) {
        let Some(pipeline) = self
            .pipelines
            .get(&(call.pipe, fusion::FORMAT, call.program))
        else {
            return;
        };
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("columbina source of a composition"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &call.bind, &[call.offset]);
        if let Some(colour) = &call.colour {
            pass.set_bind_group(1, colour, &[]);
        }
        pass.draw(0..6, 0..1);
    }

    /// Do `stage` over the whole of the picture that the preview `from` holds once it is
    /// rendered, into the preview `to` of its size: a stage after the frame that only a viewer
    /// shows, such as the Color Viewer LUT of RM20 p. 152, so that the frame itself, and what
    /// measures it, are without it. Gives whether it did anything: a stage that does nothing,
    /// or whose pipeline is still being made where this compiles in the background
    /// ([`Compositor::compile_in_background`], which wakes it once made), leaves `to` as it is,
    /// `from` being the picture.
    ///
    /// # Errors
    /// The card was lost; a target is of another card, an export, or of another size than the
    /// other; the stage reads the pixels about each one, which is not done here; or its code is
    /// not WGSL that the card takes.
    pub fn apply(
        &mut self,
        stage: &Stage,
        from: &Target,
        to: &Target,
    ) -> Result<bool, ComposeError> {
        if let Some(why) = self.gpu.lost_reason() {
            return Err(ComposeError::Lost(why));
        }
        for target in [from, to] {
            if !Arc::ptr_eq(&self.gpu, &target.gpu) {
                return Err(ComposeError::Target(
                    "the target is on another graphics card".into(),
                ));
            }
            if target.export.is_some() {
                return Err(ComposeError::Target(
                    "a stage is done over a preview, not over an export".into(),
                ));
            }
        }
        if from.size() != to.size() {
            return Err(ComposeError::Target(format!(
                "a picture of {}×{} is done into a target of {}×{}",
                from.width, from.height, to.width, to.height
            )));
        }
        if stage.reads_neighbourhoods() {
            return Err(ComposeError::Color(
                "a stage over a picture reads no pixels about each one".into(),
            ));
        }
        let Some(work) = self.stage_work(stage, from.size()) else {
            return Ok(false);
        };
        let key = (Pipe::Stage, to.texture.format(), Some(work.program));
        self.keep_made();
        let mut made = self
            .pipeline(key.0, key.1, key.2)
            .map_err(ComposeError::Color)?;
        if !made && !self.background {
            self.finish_compiling();
            made = self
                .pipeline(key.0, key.1, key.2)
                .map_err(ComposeError::Color)?;
        }
        if !made {
            // Being made on a thread of its own: the picture is shown without it meanwhile, as
            // a frame drawn otherwise than its scene says, and drawn again once it is made.
            self.provisional = true;
            return Ok(false);
        }
        let Some(colour) = self.colour_binds(&[(work.program, &work.inputs)]).pop() else {
            return Err(ComposeError::Color("its parameters cannot be bound".into()));
        };
        let mut encoder =
            self.gpu
                .device()
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("columbina stage over a picture"),
                });
        self.stage_pass(
            &mut encoder,
            &from.texture,
            &to.texture,
            work.program,
            &colour,
        );
        self.gpu.queue().submit([encoder.finish()]);
        Ok(true)
    }

    /// What a stage over the canvas does, where it does something: its program, and what its
    /// neighbourhoods are taken with, over a canvas of `size`.
    fn stage_work(&mut self, stage: &Stage, size: (u32, u32)) -> Option<StageWork> {
        let stages = [stage];
        let filters = color::filters(&stages);
        if filters.is_empty() {
            return color::program(&stages).map(|(program, inputs)| StageWork {
                program: self.number(program),
                inputs,
                taps: Vec::new(),
            });
        }
        let height = f64::from(size.1);
        let units = neighbourhood::units(height, height, height);
        let (taps, whole) = self.taps_of(&stages, &filters, size, units, 0);
        whole.map(|(program, inputs)| StageWork {
            program,
            inputs,
            taps,
        })
    }

    /// What is done for the neighbourhoods of `stages`, of `filters`, over a picture of `size`
    /// that `units` of its pixels make a pixel of a frame [`color::REFERENCE_HEIGHT`] high, by
    /// draws of the uniform numbered `uniform`; and the program of the whole, which reads them.
    fn taps_of(
        &mut self,
        stages: &[&Stage],
        filters: &[Filter],
        size: (u32, u32),
        units: f32,
        uniform: usize,
    ) -> (Vec<Tap>, Option<(usize, Inputs)>) {
        let mut taps: Vec<Tap> = Vec::with_capacity(filters.len());
        let mut views = Vec::with_capacity(filters.len());
        for (number, filter) in filters.iter().enumerate() {
            let passes = neighbourhood::passes(*filter, units, size);
            let (program, inputs) = color::program_of(stages, Part::Neighbourhood(number), &views)
                .expect("a neighbourhood is of an operation");
            let source = self.spares.shared(&self.gpu, size, Role::Source);
            let between = self
                .spares
                .shared(&self.gpu, passes[0].target, Role::Between);
            let out = self.spares.take(&self.gpu, passes[1].target);
            views.push(out.create_view(&wgpu::TextureViewDescriptor::default()));
            taps.push(Tap {
                program: self.number(program),
                inputs,
                uniform,
                source,
                between,
                out,
                passes,
            });
        }
        let whole = color::program_of(stages, Part::Whole, &views)
            .map(|(program, inputs)| (self.number(program), inputs));
        (taps, whole)
    }

    /// Draw the layer of `call` into the texture of `tap`, by its program, with the uniform at
    /// `offset`.
    fn tap_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        call: &Call,
        tap: &Tap,
        offset: u32,
        colour: &wgpu::BindGroup,
    ) {
        let view = tap
            .source
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("columbina neighbourhood"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipelines[&(call.pipe, NEIGHBOURHOOD_FORMAT, Some(tap.program))]);
        pass.set_bind_group(0, &call.bind, &[offset]);
        pass.set_bind_group(1, colour, &[]);
        pass.draw(0..6, 0..1);
    }

    /// Filter the texture of `tap` across and down, by the uniforms at the next two of
    /// `offsets`.
    fn filter_passes(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        tap: &Tap,
        offsets: &mut impl Iterator<Item = u32>,
    ) {
        let device = self.gpu.device();
        let pipeline = self
            .filter
            .as_ref()
            .expect("the pipeline of filters is made");
        for (from, to) in [(&tap.source, &tap.between), (&tap.between, &tap.out)] {
            let offset = offsets.next().expect("a uniform of each pass");
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("columbina filter"),
                layout: &self.filter_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &self.filter_uniform,
                            offset: 0,
                            size: wgpu::BufferSize::new(FILTER_PARAMS_SIZE),
                        }),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(
                            &from.create_view(&wgpu::TextureViewDescriptor::default()),
                        ),
                    },
                ],
            });
            let view = to.create_view(&wgpu::TextureViewDescriptor::default());
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("columbina filter"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &bind, &[offset]);
            pass.draw(0..3, 0..1);
        }
    }

    /// Write the uniforms of the passes of the filters of a frame, a slot each, into their
    /// buffer, made larger if it is too small; give the offset of each.
    fn write_filter_uniforms(&mut self, passes: &[Pass]) -> Vec<u32> {
        if passes.is_empty() {
            return Vec::new();
        }
        let slot = self.slot as usize;
        let mut bytes = vec![0_u8; passes.len() * slot];
        for (pass, at) in passes.iter().zip(bytes.chunks_mut(slot)) {
            for (value, into) in pass.uniform().iter().zip(at.chunks_mut(4)) {
                into.copy_from_slice(&value.to_le_bytes());
            }
        }
        if self.filter_uniform.size() < bytes.len() as u64 {
            self.filter_uniform =
                uniform_buffer(self.gpu.device(), (bytes.len() as u64).next_power_of_two());
        }
        self.gpu
            .queue()
            .write_buffer(&self.filter_uniform, 0, &bytes);
        (0..passes.len())
            .map(|i| u32::try_from(i * slot).unwrap_or(u32::MAX))
            .collect()
    }

    /// Draw `calls` into `canvas`, cleared to black first or drawn over.
    ///
    /// A call that is blended with what is under it reads the canvas as it is before it, which
    /// cannot be read while it is drawn into: the pass ends before such a call, the canvas is
    /// copied into the texture of `backdrop` (bound as the call's group 2), and a pass that
    /// keeps what is on the canvas goes on from the call. Calls of which none is blended are
    /// drawn in one pass, by the commands that drew them before layers could be blended.
    ///
    /// A canvas with a call that is blended is cleared to nothing rather than to black, so that
    /// its alpha says where the layers under such a call lie (over nothing, a layer is laid as
    /// it is), and is made opaque once all its calls are drawn: what it would have been over
    /// black, its colours being premultiplied.
    fn draw_calls(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        canvas: &wgpu::Texture,
        calls: &[Call],
        format: wgpu::TextureFormat,
        clear: bool,
        backdrop: Option<&(wgpu::Texture, wgpu::BindGroup)>,
    ) {
        let view = canvas.create_view(&wgpu::TextureViewDescriptor::default());
        let through = clear && calls.iter().any(|call| call.blended);
        let mut load = match (clear, through) {
            (false, _) => wgpu::LoadOp::Load,
            (true, false) => wgpu::LoadOp::Clear(wgpu::Color::BLACK),
            (true, true) => wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
        };
        let mut from = 0;
        loop {
            // As far as the next call that is blended, which begins a pass of its own.
            let to = calls[from..]
                .iter()
                .skip(1)
                .position(|call| call.blended)
                .map_or(calls.len(), |next| from + 1 + next);
            // The first call of a canvas that is cleared has nothing under it to copy (and
            // is not blended: `Compositor::layers`).
            let reads = calls.get(from).is_some_and(|call| call.blended) && (from > 0 || !clear);
            if let (true, Some((texture, _))) = (reads, backdrop) {
                encoder.copy_texture_to_texture(
                    canvas.as_image_copy(),
                    texture.as_image_copy(),
                    canvas.size(),
                );
            }
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("columbina layers"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                for call in &calls[from..to] {
                    let pipe = if call.blended {
                        call.pipe.blended()
                    } else {
                        call.pipe
                    };
                    pass.set_pipeline(&self.pipelines[&(pipe, format, call.program)]);
                    pass.set_bind_group(0, &call.bind, &[call.offset]);
                    if let Some(colour) = &call.colour {
                        pass.set_bind_group(1, colour, &[]);
                    }
                    if let (true, Some((_, under))) = (call.blended, backdrop) {
                        pass.set_bind_group(2, under, &[]);
                    }
                    pass.draw(0..6, 0..1);
                }
                if through && to == calls.len() {
                    pass.set_pipeline(&self.opaque[&format]);
                    pass.draw(0..3, 0..1);
                }
            }
            load = wgpu::LoadOp::Load;
            from = to;
            if from >= calls.len() {
                break;
            }
        }
    }

    /// Do a stage of colour over the whole of `from`, into `to`.
    fn stage_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        from: &wgpu::Texture,
        to: &wgpu::Texture,
        program: usize,
        colour: &wgpu::BindGroup,
    ) {
        let device = self.gpu.device();
        let bind = stage_bind(device, &self.stage_layout, from);
        let view = to.create_view(&wgpu::TextureViewDescriptor::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("columbina stage"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipelines[&(Pipe::Stage, to.format(), Some(program))]);
        pass.set_bind_group(0, &bind, &[]);
        pass.set_bind_group(1, colour, &[]);
        pass.draw(0..3, 0..1);
    }

    /// The number of a program of colour, given it the first time it is met.
    fn number(&mut self, program: Program) -> usize {
        if let Some(number) = self.program_numbers.get(&program.code) {
            return *number;
        }
        let number = self.programs.len();
        self.program_numbers.insert(program.code.clone(), number);
        self.programs.push(program);
        number
    }

    /// Ask for the pipelines that the draws and the stages of a frame need, where they are not
    /// made yet, those of programs of colour made side by side; and wait for them, or, with
    /// [`Compositor::compile_in_background`], say what waits for them.
    ///
    /// # Errors
    /// A program of colour is not WGSL that the card takes: an error of its layer, or of the
    /// colour of the frame for a stage.
    fn pipelines_of(
        &mut self,
        draws: &[Draw],
        colours: &[Option<Graded>],
        taps: &[LayerTaps],
        stages: [&Option<StageWork>; 2],
        format: wgpu::TextureFormat,
    ) -> Result<Waiting, ComposeError> {
        /// What a pipeline is for: a draw (its number, and its layer's), or a stage.
        #[derive(Clone, Copy)]
        enum For {
            Draw(usize, usize),
            Stage(usize),
        }
        let mut wanted: Vec<(For, PipelineKey)> = Vec::new();
        for (number, (draw, colour)) in draws.iter().zip(colours).enumerate() {
            let layer = colour.as_ref().map_or(0, |colour| colour.layer);
            wanted.push((
                For::Draw(number, layer),
                (
                    canvas_pipe_of(draw),
                    format,
                    colour.as_ref().map(|colour| colour.program),
                ),
            ));
        }
        for layer in taps {
            for tap in &layer.taps {
                wanted.push((
                    For::Draw(layer.draw, layer.layer),
                    (
                        pipe_of(&draws[layer.draw]),
                        NEIGHBOURHOOD_FORMAT,
                        Some(tap.program),
                    ),
                ));
            }
        }
        for (number, work) in stages.into_iter().enumerate() {
            if let Some(work) = work {
                wanted.push((
                    For::Stage(number),
                    (Pipe::Stage, format, Some(work.program)),
                ));
                for tap in &work.taps {
                    wanted.push((
                        For::Stage(number),
                        (Pipe::Stage, NEIGHBOURHOOD_FORMAT, Some(tap.program)),
                    ));
                }
            }
        }
        let failed = |what: For, message: String| match what {
            For::Draw(_, layer) => ComposeError::Layer { layer, message },
            For::Stage(_) => ComposeError::Color(message),
        };
        self.keep_made();
        for &(what, (pipe, format, program)) in &wanted {
            self.pipeline(pipe, format, program)
                .map_err(|message| failed(what, message))?;
        }
        if !self.background {
            self.finish_compiling();
        }
        let mut waiting = Waiting::default();
        for &(what, (pipe, format, program)) in &wanted {
            let made = self
                .pipeline(pipe, format, program)
                .map_err(|message| failed(what, message))?;
            match what {
                _ if made => {}
                For::Draw(number, _) => {
                    if !waiting.draws.contains(&number) {
                        waiting.draws.push(number);
                    }
                }
                For::Stage(number) => waiting.stages[number] = true,
            }
        }
        let filtered = !taps.is_empty()
            || stages
                .iter()
                .any(|work| work.as_ref().is_some_and(|work| !work.taps.is_empty()));
        if filtered && self.filter.is_none() {
            self.filter = Some(filter_pipeline(self.gpu.device(), &self.filter_layout));
        }
        Ok(waiting)
    }

    /// Keep the pipelines that the builder made since it was last asked.
    fn keep_made(&mut self) {
        let made = self.builder.take();
        self.keep(made);
    }

    /// Draw the layers of two pictures whose pipelines are not made yet as their first pictures
    /// alone, where the pipelines that draw one picture with the same colour are made: so a
    /// clip that mixes two pictures at every other frame is shown, until the pipeline that
    /// mixes them is made, as it is shown without mixing, and not without its colour. Such a
    /// draw is taken out of `waiting`, the draws whose pipelines are not made: it waits for
    /// nothing more, and its second planes are kept for the next frame.
    fn first_pictures_alone(
        &mut self,
        draws: &mut [Draw],
        colours: &[Option<Graded>],
        taps: &[LayerTaps],
        waiting: &mut Vec<usize>,
        format: wgpu::TextureFormat,
    ) {
        waiting.retain(|&number| {
            let Draw::Picture {
                shape,
                second,
                blended,
                ..
            } = &mut draws[number]
            else {
                return true;
            };
            if second.is_none() {
                return true;
            }
            let one = Pipe::Yuv(shape.texel);
            let on_canvas = if *blended { one.blended() } else { one };
            let program = colours[number].as_ref().map(|colour| colour.program);
            let made = self.pipelines.contains_key(&(on_canvas, format, program))
                && taps
                    .iter()
                    .filter(|layer| layer.draw == number)
                    .flat_map(|layer| &layer.taps)
                    .all(|tap| {
                        let key = (one, NEIGHBOURHOOD_FORMAT, Some(tap.program));
                        self.pipelines.contains_key(&key)
                    });
            if !made {
                return true;
            }
            self.planes.entry(*shape).or_default().extend(second.take());
            false
        });
    }

    /// Draw the layers whose pipelines are not made yet otherwise, and remember what the layers
    /// of this frame are drawn with, for the frames to come (see
    /// [`Compositor::compile_in_background`]).
    ///
    /// # Errors
    /// The pipeline of a layer without a program of colour cannot be made.
    fn stand_in_for_layers(
        &mut self,
        scene: &Scene,
        draws: &[Draw],
        colours: &mut [Option<Graded>],
        taps: &mut Vec<LayerTaps>,
        waiting: &[usize],
        format: wgpu::TextureFormat,
    ) -> Result<(), ComposeError> {
        let mut shown = HashMap::new();
        for &number in waiting {
            let pipe = canvas_pipe_of(&draws[number]);
            let layer = colours[number].as_ref().map(|colour| colour.layer);
            taps.retain(|taps| taps.draw != number);
            let stand_in = layer.and_then(|layer| {
                let before = self.shown_layers.remove(&layer)?;
                let same = before.pipe == pipe
                    && scene.layers.get(layer).map(input_code).as_ref() == Some(&before.input)
                    && self
                        .pipelines
                        .contains_key(&(pipe, format, Some(before.program)));
                same.then_some((layer, before))
            });
            colours[number] = if let Some((layer, before)) = stand_in {
                let graded = Graded {
                    layer,
                    program: before.program,
                    inputs: copied(&before.inputs),
                };
                shown.insert(layer, before);
                Some(graded)
            } else {
                self.pipeline(pipe, format, None)
                    .map_err(|message| ComposeError::Layer {
                        layer: layer.unwrap_or(0),
                        message,
                    })?;
                None
            };
        }
        for (number, (draw, colour)) in draws.iter().zip(colours.iter()).enumerate() {
            let Some(colour) = colour else { continue };
            if waiting.contains(&number) || taps.iter().any(|taps| taps.draw == number) {
                continue;
            }
            let Some(layer) = scene.layers.get(colour.layer) else {
                continue;
            };
            shown.insert(
                colour.layer,
                ShownLayer {
                    pipe: canvas_pipe_of(draw),
                    input: input_code(layer),
                    program: colour.program,
                    inputs: copied(&colour.inputs),
                },
            );
        }
        self.shown_layers = shown;
        Ok(())
    }

    /// The stage over the canvas numbered `number` (the timeline grade, the output transform),
    /// done as the last frame did it where its pipelines are not made yet, or not at all; and
    /// what it is done with remembered for the frames to come.
    fn stand_in_for_stage(
        &mut self,
        number: usize,
        work: Option<StageWork>,
        waiting: bool,
        format: wgpu::TextureFormat,
    ) -> Option<StageWork> {
        if !waiting {
            self.shown_stages[number] = work
                .as_ref()
                .filter(|work| work.taps.is_empty())
                .map(|work| (work.program, copied(&work.inputs)));
            return work;
        }
        match &self.shown_stages[number] {
            Some((program, inputs))
                if self
                    .pipelines
                    .contains_key(&(Pipe::Stage, format, Some(*program))) =>
            {
                Some(StageWork {
                    program: *program,
                    inputs: copied(inputs),
                    taps: Vec::new(),
                })
            }
            _ => None,
        }
    }

    /// Write what the programs of colour of a frame read, each block at a place of its own in
    /// the buffer of parameters, made larger if it is too small; give what binds each.
    fn colour_binds(&mut self, blocks: &[(usize, &Inputs)]) -> Vec<wgpu::BindGroup> {
        if blocks.is_empty() {
            return Vec::new();
        }
        let device = self.gpu.device();
        let align = device.limits().min_storage_buffer_offset_alignment as usize;
        let mut bytes: Vec<u8> = Vec::new();
        let mut spans = Vec::with_capacity(blocks.len());
        for (_, inputs) in blocks {
            let start = bytes.len();
            for value in inputs.params.iter().flatten() {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
            spans.push((start as u64, (bytes.len() - start) as u64));
            bytes.resize(bytes.len().next_multiple_of(align), 0);
        }
        if self.colour_params.size() < bytes.len() as u64 {
            self.colour_params = colour_buffer(device, (bytes.len() as u64).next_power_of_two());
        }
        self.gpu
            .queue()
            .write_buffer(&self.colour_params, 0, &bytes);
        for (program, _) in blocks {
            let kinds = self.programs[*program].textures.clone();
            if !self.colour_layouts.contains_key(&kinds) {
                let layout = colour_layout(self.gpu.device(), &kinds);
                self.colour_layouts.insert(kinds, layout);
            }
        }
        let device = self.gpu.device();
        blocks
            .iter()
            .zip(spans)
            .map(|((program, inputs), (offset, size))| {
                let layout = &self.colour_layouts[&self.programs[*program].textures];
                let mut entries = vec![
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &self.colour_params,
                            offset,
                            size: wgpu::BufferSize::new(size),
                        }),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.colour_sampler),
                    },
                ];
                for (number, view) in (color::FIRST_TEXTURE_BINDING..).zip(&inputs.textures) {
                    entries.push(wgpu::BindGroupEntry {
                        binding: number,
                        resource: wgpu::BindingResource::TextureView(view),
                    });
                }
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("columbina colour"),
                    layout,
                    entries: &entries,
                })
            })
            .collect()
    }

    /// Write the uniforms of the layers of a frame, a slot each, into the buffer of uniforms,
    /// made larger if it is too small; give the offset of each.
    fn write_uniforms(&mut self, uniforms: &[Vec<f32>]) -> Result<Vec<u32>, ComposeError> {
        let too_many = || ComposeError::Target(format!("{} layers are too many", uniforms.len()));
        let slots = u32::try_from(uniforms.len().max(1)).map_err(|_| too_many())?;
        let needed = slots.checked_mul(self.slot).ok_or_else(too_many)?;
        if self.uniform.size() < u64::from(needed) {
            self.uniform = uniform_buffer(self.gpu.device(), u64::from(needed).next_power_of_two());
            self.generation += 1;
        }
        let mut bytes = vec![0_u8; needed as usize];
        let mut offsets = Vec::with_capacity(uniforms.len());
        for (values, slot) in uniforms.iter().zip(bytes.chunks_mut(self.slot as usize)) {
            offsets.push(self.slot * u32::try_from(offsets.len()).map_err(|_| too_many())?);
            for (value, at) in values.iter().zip(slot.chunks_mut(4)) {
                at.copy_from_slice(&value.to_le_bytes());
            }
        }
        self.gpu.queue().write_buffer(&self.uniform, 0, &bytes);
        Ok(offsets)
    }

    /// The draws and uniforms of the layers of a scene, and what is done first for the
    /// neighbourhoods of each. The planes taken for pictures are in its draws, to be given back,
    /// whatever comes of it.
    #[allow(
        clippy::too_many_lines,
        reason = "each kind of layer, one after another"
    )]
    fn layers(
        &mut self,
        scene: &Scene,
        canvas: (f64, f64),
        whole: [f64; 4],
        laid: &mut Layered,
    ) -> Result<(), ComposeError> {
        for (index, layer) in scene.layers.iter().enumerate() {
            let opacity = if layer.opacity.is_nan() {
                0.0
            } else {
                layer.opacity.clamp(0.0, 1.0)
            };
            let scale = layer.transform.scale;
            if opacity <= 0.0 || !scale.is_finite() || scale <= 0.0 {
                continue;
            }
            let failed = |message: String| ComposeError::Layer {
                layer: index,
                message,
            };
            // What comes out of the composition of its clip, where it has one, drawn into a
            // texture of its own before the layers; a layer without one shows its source, as
            // it always did.
            let composed = if let Source::Fused(fusion) = &layer.source {
                match self.composition(scene, index, layer, fusion, canvas, laid)? {
                    Composed::Nothing => continue,
                    Composed::Source => None,
                    Composed::Picture(picture) => Some(picture),
                }
            } else {
                None
            };
            // Blended with what is under it, where its mode is not Normal and a layer is
            // drawn before it: the first has nothing under it, and is laid as it is.
            let blended = layer.composite != Composite::Normal && !laid.draws.is_empty();
            let blend = blended.then(|| blend_of(layer.composite, scene.luma));
            // Its input transform and its grade, after it is decoded; with what is done first
            // for their neighbourhoods, where they read any, once the size of its picture is
            // known. The input transform of a layer that shows what comes out of a
            // composition was done to its source, before the composition: its grade alone is
            // done to the picture.
            let done = Stage::default();
            let stages = match &composed {
                Some(picture) if picture.early => [&done, &layer.color.grade],
                _ => [&layer.color.input, &layer.color.grade],
            };
            let filters = color::filters(&stages);
            let spatial = stages
                .iter()
                .flat_map(|stage| &stage.ops)
                .any(|op| op.spatial);
            let mut graded = if filters.is_empty() {
                color::program(&stages).map(|(program, inputs)| Graded {
                    layer: index,
                    program: self.number(program),
                    inputs,
                })
            } else {
                None
            };
            // What is done for the neighbourhoods of a layer drawn by the draw numbered `draw`,
            // of a picture of `size`, `units` of its pixels to a pixel of the reference frame,
            // drawn alone by `uniform`.
            let mut neighbourhoods = |this: &mut Self,
                                      laid: &mut Layered,
                                      draw: usize,
                                      size: (u32, u32),
                                      units: f32,
                                      uniform: Vec<f32>| {
                if filters.is_empty() {
                    return;
                }
                let number = laid.tap_uniforms.len();
                laid.tap_uniforms.push(uniform);
                let (taps, program) = this.taps_of(&stages, &filters, size, units, number);
                graded = program.map(|(program, inputs)| Graded {
                    layer: index,
                    program,
                    inputs,
                });
                laid.taps.push(LayerTaps {
                    layer: index,
                    draw,
                    taps,
                });
            };
            if let Some(picture) = composed {
                // The picture of the composition, laid as the picture of a title is: its
                // texture over the frame that it is fitted into, which its crop cuts.
                let frame = picture.frame;
                let own = (f64::from(picture.size.0), f64::from(picture.size.1));
                let alone = layer_uniform(
                    &placement(own, own, &Transform::NONE),
                    own,
                    [0.0, 0.0, own.0, own.1],
                    1.0,
                    1.0,
                    1,
                    Some([1.0; 4]),
                );
                let units = neighbourhood::units(own.1, frame.1, canvas.1);
                let draw = laid.draws.len();
                neighbourhoods(self, laid, draw, picture.size, units, alone);
                let mut values = layer_uniform(
                    &placement(canvas, frame, &layer.transform),
                    canvas,
                    [0.0, 0.0, frame.0, frame.1],
                    opacity,
                    scale,
                    1,
                    Some([1.0; 4]),
                );
                cut_and_blend(&mut values, picture.kept, layer.crop.fade(frame.1), blend);
                laid.uniforms.push(values);
                laid.draws.push(Draw::Texture(picture.view, blended));
                laid.colours.push(graded);
                continue;
            }
            let source = layer.source.own();
            match source {
                Source::Picture(..) | Source::Between { .. } => {
                    let Some(Pictures {
                        first: picture,
                        second,
                        rotation,
                    }) = pictures_of(source)
                    else {
                        continue;
                    };
                    let layout = yuv::check(picture).map_err(failed)?;
                    let most = self.gpu.device().limits().max_texture_dimension_2d;
                    if picture.width > most || picture.height > most {
                        return Err(failed(format!(
                            "a picture of {}×{} is larger than the {most}×{most} that the card \
                             takes",
                            picture.width, picture.height
                        )));
                    }
                    let shown = shown_size((picture.width, picture.height), rotation);
                    let frame = fit(canvas, (f64::from(shown.0), f64::from(shown.1)));
                    if frame.0 <= 0.0 || frame.1 <= 0.0 {
                        continue;
                    }
                    // Its crop, of the picture as it is fitted; nothing is drawn of one that
                    // is all cut away.
                    let Some(kept) = layer.crop.kept(frame) else {
                        continue;
                    };
                    let shape = Shape {
                        width: picture.width,
                        height: picture.height,
                        format: picture.format,
                        alpha: layout.alpha_plane(),
                        texel: layout.texel(self.gpu.has_sixteen_norm()),
                    };
                    let planes = self.take_planes(shape);
                    let uploaded = yuv::upload(self.gpu.queue(), &planes.textures, picture);
                    // The second picture of two that are mixed, into planes of its own, of the
                    // shape of the first: the one uniform says how either is read.
                    let (other, mixed) = match second {
                        Some((second, weight)) => {
                            let other = self.take_planes(shape);
                            let uploaded = yuv::check(second).and_then(|second_layout| {
                                if second_layout == layout {
                                    yuv::upload(self.gpu.queue(), &other.textures, second)
                                } else {
                                    Err(format!(
                                        "a picture of {}×{} of {:?} in {} bits is mixed with one \
                                         of {}×{} of {:?} in {} bits",
                                        picture.width,
                                        picture.height,
                                        picture.format,
                                        picture.bits,
                                        second.width,
                                        second.height,
                                        second.format,
                                        second.bits
                                    ))
                                }
                            });
                            (Some(other), uploaded.map(|()| Some(weight)))
                        }
                        None => (None, Ok(None)),
                    };
                    let draw = laid.draws.len();
                    laid.draws.push(Draw::Picture {
                        shape,
                        planes,
                        second: other,
                        blended,
                    });
                    uploaded.map_err(failed)?;
                    let weight = mixed.map_err(failed)?;
                    let mut decoded = yuv::picture_params(
                        [0.0; 4],
                        layout,
                        shape.texel,
                        layer.color.coding,
                        rotation,
                    );
                    yuv::set_unclamped(&mut decoded, layer.color.unclamped);
                    if let Some(weight) = weight {
                        yuv::set_between(&mut decoded, weight);
                    }
                    // Alone, at its own size.
                    let own = (f64::from(shown.0), f64::from(shown.1));
                    let mut alone = decoded.to_vec();
                    alone.extend(layer_uniform(
                        &placement(own, own, &Transform::NONE),
                        own,
                        [0.0, 0.0, own.0, own.1],
                        1.0,
                        1.0,
                        1,
                        None,
                    ));
                    let units = neighbourhood::units(own.1, frame.1, canvas.1);
                    neighbourhoods(self, laid, draw, shown, units, alone);
                    laid.colours.push(graded);
                    let mut placed = layer_uniform(
                        &placement(canvas, frame, &layer.transform),
                        canvas,
                        [0.0, 0.0, frame.0, frame.1],
                        opacity,
                        scale,
                        samples(minification(shown, frame, scale)),
                        None,
                    );
                    cut_and_blend(&mut placed, kept, layer.crop.fade(frame.1), blend);
                    let mut values = decoded.to_vec();
                    values.extend(placed);
                    laid.uniforms.push(values);
                }
                Source::Color(rgba) => {
                    // Its crop, of the canvas, which is its picture.
                    let Some(kept) = layer.crop.kept(canvas) else {
                        continue;
                    };
                    let tint = rgba.map(|channel| channel.clamp(0.0, 1.0));
                    // Uniform colour stays one texel unless a spatial key changes it.
                    let pixels = if spatial {
                        (scene.width, scene.height)
                    } else {
                        (1, 1)
                    };
                    let own = (f64::from(pixels.0), f64::from(pixels.1));
                    let alone = layer_uniform(
                        &placement(own, own, &Transform::NONE),
                        own,
                        [0.0, 0.0, own.0, own.1],
                        1.0,
                        1.0,
                        1,
                        Some(tint),
                    );
                    let units = neighbourhood::units(own.1, canvas.1, canvas.1);
                    let draw = laid.draws.len();
                    neighbourhoods(self, laid, draw, pixels, units, alone);
                    let mut values = layer_uniform(
                        &placement(canvas, canvas, &layer.transform),
                        canvas,
                        whole,
                        opacity,
                        scale,
                        1,
                        Some(tint),
                    );
                    cut_and_blend(&mut values, kept, layer.crop.fade(canvas.1), blend);
                    laid.uniforms.push(values);
                    laid.draws.push(Draw::Texture(self.white.clone(), blended));
                    laid.colours.push(graded);
                }
                Source::Title(title) => {
                    // Its crop, of the canvas: the picture of a title is the frame of the
                    // timeline, of which its quad is only the block of its text.
                    let Some(kept) = layer.crop.kept(canvas) else {
                        continue;
                    };
                    let style = &title.style;
                    let place = placement(canvas, canvas, &layer.transform);
                    #[allow(clippy::cast_possible_truncation, reason = "a size of letters")]
                    let size =
                        |per: f64| (style.size.min(4096.0) * f64::from(scene.units) * per) as f32;
                    let put = self.put_fitted(
                        |per| Spec::title(title, size(per).max(1.0)),
                        raster_scale(scale),
                        |block| {
                            title_origin(
                                (scene.width, scene.height),
                                block,
                                (style.x, style.y),
                                style.align,
                            )
                        },
                        &place,
                        canvas,
                    );
                    if let Some((quad, view, pixels)) = put.map_err(failed)? {
                        let own = (f64::from(pixels.0), f64::from(pixels.1));
                        let mut alone = layer_uniform(
                            &placement(own, own, &Transform::NONE),
                            own,
                            [0.0, 0.0, own.0, own.1],
                            1.0,
                            1.0,
                            1,
                            Some([1.0; 4]),
                        );
                        window_canvas(&mut alone, quad, canvas);
                        let units = neighbourhood::units(own.1, quad[3] - quad[1], canvas.1);
                        let draw = laid.draws.len();
                        neighbourhoods(self, laid, draw, pixels, units, alone);
                        let mut values =
                            layer_uniform(&place, canvas, quad, opacity, scale, 1, Some([1.0; 4]));
                        window_canvas(&mut values, quad, canvas);
                        cut_and_blend(&mut values, kept, layer.crop.fade(canvas.1), blend);
                        laid.uniforms.push(values);
                        laid.draws.push(Draw::Texture(view, blended));
                        laid.colours.push(graded);
                    }
                }
                // The source of a clip is never what comes out of a composition
                // ([`Source::own`]).
                Source::Fused(_) => {}
            }
        }
        Ok(())
    }

    /// What the layer numbered `index`, of a clip that has a composition, shows: nothing, its
    /// source as it is, or the picture that comes out of the composition, whose passes are
    /// put among those of the frame (`laid`), each with the texture that it draws into.
    ///
    /// The pictures are drawn at the size that the layer shows them at
    /// ([`fusion::drawn_size`]). The source is drawn alone only where a pass reads it. Where
    /// the pipeline of the passes is being made still and frames are drawn without waiting
    /// ([`Compositor::compile_in_background`]), the layer shows its source meanwhile.
    ///
    /// # Errors
    /// The source of the clip cannot be drawn, or the pipeline of the passes cannot be made.
    #[allow(
        clippy::too_many_lines,
        reason = "one composition, step by step: its plan, its source, its textures, its passes"
    )]
    fn composition(
        &mut self,
        scene: &Scene,
        index: usize,
        layer: &SceneLayer,
        fusion: &Fusion,
        canvas: (f64, f64),
        laid: &mut Layered,
    ) -> Result<Composed, ComposeError> {
        let failed = |message: String| ComposeError::Layer {
            layer: index,
            message,
        };
        // Fitted to the canvas by its shape, as a picture of its size is; its crop, of the
        // picture as it is fitted.
        let nominal = (f64::from(fusion.size.0), f64::from(fusion.size.1));
        let frame = fit(canvas, nominal);
        let most = self.gpu.device().limits().max_texture_dimension_2d;
        let drawn = fusion::drawn_size(fusion.size, canvas, layer.transform.scale, most);
        let (Some(size), Some(kept)) = (drawn, layer.crop.kept(frame)) else {
            return Ok(Composed::Nothing);
        };
        let mut plan = fusion::plan(&fusion.composition, size, scene.luma, true);
        match plan.output {
            Reads::Nothing => return Ok(Composed::Nothing),
            Reads::Source => return Ok(Composed::Source),
            Reads::Pass(_) => {}
        }
        if plan.lays() {
            let key = (Pipe::Fusion, fusion::FORMAT, None);
            self.keep_made();
            let mut made = self.pipeline(key.0, key.1, key.2).map_err(failed)?;
            if !made && !self.background {
                self.finish_compiling();
                made = self.pipeline(key.0, key.1, key.2).map_err(failed)?;
            }
            if !made {
                laid.waiting = true;
                return Ok(Composed::Source);
            }
        }
        // The input transform of the layer is done to the source, before the composition,
        // which works in the space of the timeline; but one that reads the pixels about each
        // one, which is not drawn alone here, is done after it, with the grade.
        let early = !layer.color.input.reads_neighbourhoods();
        let mut source = None;
        if plan.reads_source() {
            source = self.alone(index, layer, fusion, size, early, laid)?;
            if source.is_none() {
                // A title of which nothing is seen: a `MediaIn` is no picture.
                plan = fusion::plan(&fusion.composition, size, scene.luma, false);
            }
        }
        let Reads::Pass(output) = plan.output else {
            return Ok(Composed::Nothing);
        };
        let slots = plan.slots();
        let textures: Vec<wgpu::Texture> = (0..slots.count)
            .map(|_| self.fusion_texture(size, &mut laid.free))
            .collect();
        let texture = |slot: Option<usize>| slot.and_then(|slot| textures.get(slot)).cloned();
        let read = |reads: Reads| match reads {
            Reads::Nothing => None,
            Reads::Source => texture(slots.source),
            Reads::Pass(at) => texture(slots.passes.get(at).copied()),
        };
        if let (Some(source), Some(target)) = (source, read(Reads::Source)) {
            if let Some(alone) = laid.sources.get_mut(source) {
                alone.target = Some(target);
                laid.offscreen.push(Offscreen::Source(source));
            }
        }
        for (at, pass) in plan.passes.iter().enumerate() {
            let Some(target) = read(Reads::Pass(at)) else {
                continue;
            };
            match pass {
                fusion::Pass::Fill(colour) => laid.offscreen.push(Offscreen::Fill {
                    colour: *colour,
                    target,
                }),
                fusion::Pass::Lay {
                    background,
                    foreground,
                    uniform,
                } => {
                    let Some(foreground) = read(*foreground) else {
                        continue;
                    };
                    laid.offscreen.push(Offscreen::Lay {
                        uniform: laid.lays.len(),
                        background: read(*background),
                        foreground,
                        target,
                    });
                    laid.lays.push(*uniform);
                }
            }
        }
        let Some(out) = read(Reads::Pass(output)) else {
            return Ok(Composed::Nothing);
        };
        let view = out.create_view(&wgpu::TextureViewDescriptor::default());
        // The textures but that of the picture that comes out, which the layer reads, are
        // free for the compositions of the layers after this one.
        let kept_slot = slots.passes.get(output).copied();
        for (slot, texture) in textures.into_iter().enumerate() {
            if Some(slot) != kept_slot {
                laid.free.push(texture);
            }
        }
        Ok(Composed::Picture(Outcome {
            view,
            size,
            frame,
            kept,
            early,
        }))
    }

    /// A texture of `size` for a pass of a composition: one that the compositions drawn
    /// before in the frame read no more (`free`), or one of the spares of neighbourhoods,
    /// which are kept from frame to frame by their size.
    fn fusion_texture(&mut self, size: (u32, u32), free: &mut Vec<wgpu::Texture>) -> wgpu::Texture {
        let done_with = free
            .iter()
            .position(|texture| (texture.width(), texture.height()) == size);
        match done_with {
            Some(at) => free.swap_remove(at),
            None => self.spares.take(&self.gpu, size),
        }
    }

    /// The source of the clip of the layer numbered `index`, to be drawn alone over a texture
    /// of `size` pixels of its composition, which it fills: what a `MediaIn` shows. It is kept
    /// among the sources of the frame (`laid`), and its number there is given; none where it
    /// shows nothing, as a title of which nothing is seen. With `early`, the input transform
    /// of the layer is done to it.
    ///
    /// A picture is read as a layer reads it, at as many points of each pixel as it is drawn
    /// smaller; a title is drawn where its style puts it in the picture of the composition,
    /// its letters as many pixels high as the picture is drawn larger or smaller than the
    /// timeline; a colour covers the picture.
    ///
    /// # Errors
    /// A picture whose planes do not hold it, or one larger than the card takes; two pictures
    /// to be mixed that are not laid out alike; or a text too large to be drawn.
    #[allow(
        clippy::too_many_lines,
        reason = "each kind of source, one after another"
    )]
    fn alone(
        &mut self,
        index: usize,
        layer: &SceneLayer,
        fusion: &Fusion,
        size: (u32, u32),
        early: bool,
        laid: &mut Layered,
    ) -> Result<Option<usize>, ComposeError> {
        let failed = |message: String| ComposeError::Layer {
            layer: index,
            message,
        };
        let own = (f64::from(size.0), f64::from(size.1));
        let whole = [0.0, 0.0, own.0, own.1];
        let place = placement(own, own, &Transform::NONE);
        let colour = if early {
            color::program(&[&layer.color.input]).map(|(program, inputs)| Graded {
                layer: index,
                program: self.number(program),
                inputs,
            })
        } else {
            None
        };
        let (draw, uniform) = match fusion.source.own() {
            source @ (Source::Picture(..) | Source::Between { .. }) => {
                let Some(Pictures {
                    first: picture,
                    second,
                    rotation,
                }) = pictures_of(source)
                else {
                    return Ok(None);
                };
                let layout = yuv::check(picture).map_err(failed)?;
                let most = self.gpu.device().limits().max_texture_dimension_2d;
                if picture.width > most || picture.height > most {
                    return Err(failed(format!(
                        "a picture of {}×{} is larger than the {most}×{most} that the card takes",
                        picture.width, picture.height
                    )));
                }
                let shown = shown_size((picture.width, picture.height), rotation);
                let shape = Shape {
                    width: picture.width,
                    height: picture.height,
                    format: picture.format,
                    alpha: layout.alpha_plane(),
                    texel: layout.texel(self.gpu.has_sixteen_norm()),
                };
                let planes = self.take_planes(shape);
                let uploaded = yuv::upload(self.gpu.queue(), &planes.textures, picture);
                // The second picture of two that are mixed, into planes of its own.
                let (other, mixed) = match second {
                    Some((second, weight)) => {
                        let other = self.take_planes(shape);
                        let uploaded = yuv::check(second).and_then(|second_layout| {
                            if second_layout == layout {
                                yuv::upload(self.gpu.queue(), &other.textures, second)
                            } else {
                                Err("the two pictures that it mixes are not laid out alike".into())
                            }
                        });
                        (Some(other), uploaded.map(|()| Some(weight)))
                    }
                    None => (None, Ok(None)),
                };
                // Kept before anything of it is refused, so that its planes are given back.
                let number = laid.sources.len();
                laid.sources.push(Alone {
                    layer: index,
                    draw: Draw::Picture {
                        shape,
                        planes,
                        second: other,
                        blended: false,
                    },
                    uniform: Vec::new(),
                    colour,
                    target: None,
                });
                uploaded.map_err(failed)?;
                let weight = mixed.map_err(failed)?;
                let mut decoded = yuv::picture_params(
                    [0.0; 4],
                    layout,
                    shape.texel,
                    layer.color.coding,
                    rotation,
                );
                yuv::set_unclamped(&mut decoded, layer.color.unclamped);
                if let Some(weight) = weight {
                    yuv::set_between(&mut decoded, weight);
                }
                let mut values = decoded.to_vec();
                values.extend(layer_uniform(
                    &place,
                    own,
                    whole,
                    1.0,
                    1.0,
                    samples(minification(shown, own, 1.0)),
                    None,
                ));
                if let Some(alone) = laid.sources.get_mut(number) {
                    alone.uniform = values;
                }
                return Ok(Some(number));
            }
            Source::Color(rgba) => {
                let tint = rgba.map(|channel| channel.clamp(0.0, 1.0));
                let values = layer_uniform(&place, own, whole, 1.0, 1.0, 1, Some(tint));
                (Draw::Texture(self.white.clone(), false), values)
            }
            Source::Title(title) => {
                let style = &title.style;
                // The pixels that the picture is drawn at to a pixel of the composition, whose
                // pixels are those of the timeline, in which the size of the letters is said.
                let per = own.0 / f64::from(fusion.size.0.max(1));
                #[allow(clippy::cast_possible_truncation, reason = "a size of letters")]
                let letters = |by: f64| (style.size.min(4096.0) * per * by) as f32;
                let put = self.put_fitted(
                    |by| Spec::title(title, letters(by).max(1.0)),
                    1.0,
                    |block| title_origin(size, block, (style.x, style.y), style.align),
                    &place,
                    own,
                );
                let Some((quad, view, _)) = put.map_err(failed)? else {
                    return Ok(None);
                };
                let mut values = layer_uniform(&place, own, quad, 1.0, 1.0, 1, Some([1.0; 4]));
                window_canvas(&mut values, quad, own);
                (Draw::Texture(view, false), values)
            }
            // The source of a clip is never what comes out of a composition.
            Source::Fused(_) => return Ok(None),
        };
        laid.sources.push(Alone {
            layer: index,
            draw,
            uniform,
            colour,
            target: None,
        });
        Ok(Some(laid.sources.len() - 1))
    }

    /// Planes for a picture of `shape`: some of those kept, or new ones.
    fn take_planes(&mut self, shape: Shape) -> Planes {
        let kept = self.planes.get_mut(&shape).and_then(Vec::pop);
        let mut planes = kept.unwrap_or_else(|| {
            let textures = yuv::plane_textures(
                self.gpu.device(),
                shape.width,
                shape.height,
                shape.format,
                shape.alpha,
                shape.texel,
            );
            Planes {
                textures,
                bind: None,
                used: 0,
            }
        });
        planes.used = self.frame;
        planes
    }

    /// Keep the planes of the draws of a frame, and of the sources of its compositions that
    /// were drawn alone, for the next.
    fn give_back(&mut self, draws: Vec<Draw>, sources: Vec<Alone>) {
        let alone = sources.into_iter().map(|source| source.draw);
        for draw in draws.into_iter().chain(alone) {
            if let Draw::Picture {
                shape,
                planes,
                second,
                ..
            } = draw
            {
                let kept = self.planes.entry(shape).or_default();
                kept.push(planes);
                kept.extend(second);
            }
        }
        self.spares.settle(self.frame);
    }

    /// Let go of the planes of shapes not drawn for a while.
    fn forget_unused(&mut self) {
        let frame = self.frame;
        for kept in self.planes.values_mut() {
            kept.retain(|planes| planes.used + PLANES_KEPT_FOR >= frame);
        }
        self.planes.retain(|_, kept| !kept.is_empty());
    }

    /// Put a text, `per` of its pixels to a pixel of its frame, laid out as `spec` says for that
    /// many, and made smaller until it can be drawn at once: the quad of its frame that it
    /// covers and its texture, or none where it is not seen.
    fn put_fitted(
        &mut self,
        spec: impl Fn(f64) -> Spec,
        per: f64,
        origin: impl Fn((f64, f64)) -> (f64, f64),
        place: &Affine,
        canvas: (f64, f64),
    ) -> Result<Option<Fitted>, String> {
        let mut per = per;
        for _ in 0..SHRINKS {
            match self.put_text(&spec(per), per, &origin, place, canvas) {
                Put::Drawn(quad, view, size) => return Ok(Some((quad, view, size))),
                Put::Unseen => return Ok(None),
                // Smaller by a quarter of an octave at least, on quarters of an octave.
                Put::TooLarge(by) => {
                    per = 2.0_f64.powf(((4.0 * (per * by).log2()).floor() - 1.0) / 4.0);
                }
            }
        }
        Err("a text too large to be drawn".into())
    }

    /// Put a text laid out as `spec` says, `per` of its pixels to a pixel of its frame (the
    /// canvas, which `place` puts on the canvas), its block where `origin` says for its size in
    /// the frame: the part of it that is seen, drawn into pixels and onto the card, or kept from
    /// a frame before.
    #[allow(clippy::cast_precision_loss, reason = "counts of pixels")]
    fn put_text(
        &mut self,
        spec: &Spec,
        per: f64,
        origin: impl Fn((f64, f64)) -> (f64, f64),
        place: &Affine,
        canvas: (f64, f64),
    ) -> Put {
        let key = TextKey::of(spec);
        let frame = self.frame;
        if !self.texts.contains_key(&key) {
            if self.texts.len() >= TEXTS_KEPT {
                self.forget_oldest_text(&key);
            }
            let laid = text::lay_out(spec);
            self.texts.insert(
                key.clone(),
                LaidText {
                    laid,
                    parts: Vec::new(),
                    used: frame,
                },
            );
        }
        let most_side = self.gpu.device().limits().max_texture_dimension_2d;
        let gpu = &self.gpu;
        let Some(text) = self.texts.get_mut(&key) else {
            return Put::Unseen;
        };
        text.used = frame;
        let (block, pad, full) = (text.laid.block, text.laid.pad, text.laid.full());
        let at = origin((f64::from(block.0) / per, f64::from(block.1) / per));
        let start = (at.0 - f64::from(pad) / per, at.1 - f64::from(pad) / per);
        let Some(window) = text_window(place, canvas, start, per, full) else {
            return Put::Unseen;
        };
        let (width, height) = (window[2] - window[0], window[3] - window[1]);
        let area = u64::from(width) * u64::from(height);
        if area > MOST_PIXELS || width > most_side || height > most_side {
            let by_area = (MOST_PIXELS as f64 / area as f64).sqrt();
            let by_side = f64::from(most_side) / f64::from(width.max(height));
            return Put::TooLarge(by_area.min(by_side));
        }
        let view = if let Some(index) = text.parts.iter().position(|part| part.window == window) {
            let part = text.parts.remove(index);
            let view = part.view.clone();
            text.parts.insert(0, part);
            view
        } else {
            let raster = text::draw(&mut text.laid, window);
            self.text_draws += 1;
            let view = text_texture(gpu, &raster);
            text.parts.insert(
                0,
                TextPart {
                    window,
                    view: view.clone(),
                    bytes: area * 4,
                },
            );
            text.parts.truncate(PARTS_KEPT);
            self.keep_text_bytes(&key);
            view
        };
        let quad = [
            start.0 + f64::from(window[0]) / per,
            start.1 + f64::from(window[1]) / per,
            start.0 + f64::from(window[2]) / per,
            start.1 + f64::from(window[3]) / per,
        ];
        Put::Drawn(quad, view, (width, height))
    }

    /// Let go of the text drawn longest ago, other than `keep`.
    fn forget_oldest_text(&mut self, keep: &TextKey) {
        let oldest = self
            .texts
            .iter()
            .filter(|(key, _)| *key != keep)
            .min_by_key(|(_, text)| text.used)
            .map(|(key, _)| key.clone());
        if let Some(oldest) = oldest {
            self.texts.remove(&oldest);
        }
    }

    /// Let go of the texts drawn longest ago, other than `keep`, while those on the card take
    /// more than [`TEXT_BYTES`].
    fn keep_text_bytes(&mut self, keep: &TextKey) {
        loop {
            let bytes: u64 = self
                .texts
                .iter()
                .filter(|(key, _)| *key != keep)
                .flat_map(|(_, text)| text.parts.iter().map(|part| part.bytes))
                .sum();
            if bytes <= TEXT_BYTES || self.texts.len() <= 1 {
                return;
            }
            self.forget_oldest_text(keep);
        }
    }

    /// Whether the pipeline of `pipe` that draws into targets of `format`, with a program of
    /// colour or none, is made: one with a program is asked of the builder if it is not (and is
    /// not made yet as this returns); one without, the compositor's own shader, is made at once.
    /// But that of the passes of compositions ([`Pipe::Fusion`]), whose shader has the maths of
    /// every composite mode in it, is asked of the builder too: a frame that is drawn without
    /// waiting is not held up by it.
    ///
    /// # Errors
    /// The code of the program is not WGSL that the card takes, or reads what is not bound.
    fn pipeline(
        &mut self,
        pipe: Pipe,
        format: wgpu::TextureFormat,
        program: Option<usize>,
    ) -> Result<bool, String> {
        let key = (pipe, format, program);
        if self.pipelines.contains_key(&key) {
            return Ok(true);
        }
        if let Some(why) = self.failed.get(&key) {
            return Err(why.clone());
        }
        if self.builder.is_pending(&key) {
            return Ok(false);
        }
        if let Some(number) = program {
            let kinds = &self.programs[number].textures;
            if !self.colour_layouts.contains_key(kinds) {
                let layout = colour_layout(self.gpu.device(), kinds);
                self.colour_layouts.insert(kinds.clone(), layout);
            }
        }
        if let Pipe::YuvBetween(texel) | Pipe::YuvBetweenBlended(texel) = pipe {
            self.between_layout(texel);
        }
        if pipe == Pipe::Fusion && self.fusion.is_none() {
            self.fusion = Some(fusion::Card::new(&self.gpu, self.slot));
        }
        let (shader, layout) = match pipe {
            Pipe::Yuv(texel) => (texel.shader(), &self.yuv_layouts[&texel]),
            Pipe::Rgba => (LAYER_SHADER.to_owned(), &self.rgba_layout),
            Pipe::YuvBlended(texel) => (blended_shader(&texel.shader()), &self.yuv_layouts[&texel]),
            Pipe::RgbaBlended => (blended_shader(LAYER_SHADER), &self.rgba_layout),
            Pipe::YuvBetween(texel) => (
                between_shader(&texel.shader()),
                &self.between_layouts[&texel],
            ),
            Pipe::YuvBetweenBlended(texel) => (
                blended_shader(&between_shader(&texel.shader())),
                &self.between_layouts[&texel],
            ),
            Pipe::Stage => (STAGE_SHADER.to_owned(), &self.stage_layout),
            Pipe::Fusion => match &self.fusion {
                Some(card) => (fusion::shader(), &card.layout),
                None => return Err("a pass of a composition has nothing to bind it".into()),
            },
        };
        // What binds the canvas under a layer that is blended: a texture, as a stage reads one.
        let backdrop = pipe.is_blended().then(|| self.stage_layout.clone());
        let (source, colour) = match program {
            Some(number) => {
                let program = &self.programs[number];
                (
                    color::with_program(&shader, program),
                    Some(self.colour_layouts[&program.textures].clone()),
                )
            }
            None => (shader, None),
        };
        let making = maker(pipe, source, layout.clone(), colour, backdrop, format);
        if program.is_none() && pipe != Pipe::Fusion {
            let made = making(self.gpu.device());
            self.keep(vec![(key, made)]);
            return self.pipeline(pipe, format, program);
        }
        self.builder.ask(key, making);
        Ok(false)
    }

    /// Keep the pipelines made, and why those that were not could not be.
    fn keep(&mut self, made: Vec<(PipelineKey, Made)>) {
        for (key, made) in made {
            match made {
                Ok(pipeline) => {
                    self.pipelines.insert(key, pipeline);
                }
                Err(why) => {
                    self.failed.insert(key, why);
                }
            }
        }
    }

    /// What binds the planes of a picture (luma, chroma and alpha), and the uniform.
    fn planes_bind(&self, texel: Texel, planes: &Planes) -> wgpu::BindGroup {
        let [luma, chroma, alpha] = planes.textures.views();
        self.gpu
            .device()
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("columbina picture"),
                layout: &self.yuv_layouts[&texel],
                entries: &[
                    self.uniform_entry(yuv::PARAMS_SIZE),
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&luma),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&chroma),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(&alpha),
                    },
                ],
            })
    }

    /// What binds the planes of two pictures that are mixed, read as `texel`s
    /// (`between.wgsl`): made the first time it is asked for, so that a compositor that draws
    /// no such layer has none.
    fn between_layout(&mut self, texel: Texel) -> wgpu::BindGroupLayout {
        let device = self.gpu.device();
        self.between_layouts
            .entry(texel)
            .or_insert_with(|| pictures_layout(device, texel))
            .clone()
    }

    /// What binds the planes of two pictures that are mixed, those of `first` and then those
    /// of `second`, and the uniform. It is made for each frame: the planes that two pictures
    /// are given are not the same two from one frame to the next.
    fn pictures_bind(&mut self, texel: Texel, first: &Planes, second: &Planes) -> wgpu::BindGroup {
        let layout = self.between_layout(texel);
        let views: Vec<wgpu::TextureView> = first
            .textures
            .views()
            .into_iter()
            .chain(second.textures.views())
            .collect();
        let mut entries = vec![self.uniform_entry(yuv::PARAMS_SIZE)];
        entries.extend(
            views
                .iter()
                .zip(1..)
                .map(|(view, binding)| wgpu::BindGroupEntry {
                    binding,
                    resource: wgpu::BindingResource::TextureView(view),
                }),
        );
        self.gpu
            .device()
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("columbina pictures"),
                layout: &layout,
                entries: &entries,
            })
    }

    /// What binds a texture of R'G'B'A, and the uniform.
    fn texture_bind(&self, view: &wgpu::TextureView) -> wgpu::BindGroup {
        self.gpu
            .device()
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("columbina layer"),
                layout: &self.rgba_layout,
                entries: &[
                    self.uniform_entry(LAYER_PARAMS_SIZE),
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            })
    }

    /// The uniform, bound a slot of `size` bytes at a time.
    fn uniform_entry(&self, size: u64) -> wgpu::BindGroupEntry<'_> {
        wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer: &self.uniform,
                offset: 0,
                size: wgpu::BufferSize::new(size),
            }),
        }
    }

    /// Make the canvas of an export Y'CbCr into its buffers, and copy them to be read back.
    fn convert(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        canvas: &wgpu::TextureView,
        export: &Export,
        height: u32,
    ) {
        let device = self.gpu.device();
        let (luma_pipeline, chroma_pipeline) = self.convert.get_or_insert_with(|| {
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("columbina convert"),
                source: wgpu::ShaderSource::Wgsl(CONVERT_SHADER.into()),
            });
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("columbina convert"),
                bind_group_layouts: &[Some(&self.convert_layout)],
                immediate_size: 0,
            });
            let pipeline = |entry| {
                device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some("columbina convert"),
                    layout: Some(&layout),
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    cache: None,
                })
            };
            (pipeline("luma_main"), pipeline("chroma_main"))
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("columbina convert"),
            layout: &self.convert_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(canvas),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: export.luma.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: export.chroma.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: export.params.as_entire_binding(),
                },
            ],
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("columbina convert"),
                timestamp_writes: None,
            });
            pass.set_bind_group(0, &bind, &[]);
            let groups = export.words.div_ceil(CONVERT_GROUP);
            pass.set_pipeline(luma_pipeline);
            pass.dispatch_workgroups(groups, height, 1);
            pass.set_pipeline(chroma_pipeline);
            pass.dispatch_workgroups(groups, export.chroma_rows, 1);
        }
        encoder.copy_buffer_to_buffer(&export.luma, 0, &export.luma_read, 0, None);
        encoder.copy_buffer_to_buffer(&export.chroma, 0, &export.chroma_read, 0, None);
    }
}

/// What makes the pipeline of `pipe` of the shader `source` that draws into targets of
/// `format` (that of a pass of a composition, into the format of its textures), `layout`
/// binding what it reads, `colour` what its program of colour reads, where it has one, and
/// `backdrop` the canvas under a layer that is blended: on whatever thread, the errors of its
/// validation caught in a scope of that thread, as the code of a program is not the
/// compositor's.
fn maker(
    pipe: Pipe,
    source: String,
    layout: wgpu::BindGroupLayout,
    colour: Option<wgpu::BindGroupLayout>,
    backdrop: Option<wgpu::BindGroupLayout>,
    format: wgpu::TextureFormat,
) -> Make {
    Box::new(move |device: &wgpu::Device| {
        let scope = colour
            .as_ref()
            .map(|_| device.push_error_scope(wgpu::ErrorFilter::Validation));
        let pipeline = if pipe == Pipe::Stage {
            stage_pipeline(device, &source, &layout, colour.as_ref(), format)
        } else if pipe == Pipe::Fusion {
            fusion::pipeline(device, &source, &layout)
        } else {
            let groups = (colour.as_ref(), backdrop.as_ref());
            layer_pipeline(device, &source, &layout, groups, format)
        };
        if let Some(scope) = scope {
            if let Some(error) = pollster::block_on(scope.pop()) {
                return Err(format!("its colour cannot be done: {error}"));
            }
        }
        Ok(pipeline)
    })
}

/// A pipeline that draws layers of the shader `source` into targets of `format`, laid over what
/// is there, premultiplied; with what a program of colour reads as group 1, where it has one,
/// and the canvas under the layer as group 2, where it is blended with it (`groups`, in that
/// order). A layer that is not blended has the two groups that it always had.
fn layer_pipeline(
    device: &wgpu::Device,
    source: &str,
    layout: &wgpu::BindGroupLayout,
    groups: (
        Option<&wgpu::BindGroupLayout>,
        Option<&wgpu::BindGroupLayout>,
    ),
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("columbina layer"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let (colour, backdrop) = groups;
    let all = [Some(layout), colour, backdrop];
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("columbina layer"),
        bind_group_layouts: if backdrop.is_some() {
            &all[..]
        } else {
            &all[..2]
        },
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("columbina layer"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs_layer"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs_layer"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// The pipeline that makes a canvas of `format` opaque (`opaque.wgsl`): it writes alpha alone,
/// 1 everywhere, and leaves the colours as they are.
fn opaque_pipeline(device: &wgpu::Device, format: wgpu::TextureFormat) -> wgpu::RenderPipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("columbina opaque"),
        source: wgpu::ShaderSource::Wgsl(OPAQUE_SHADER.into()),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("columbina opaque"),
        bind_group_layouts: &[],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("columbina opaque"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALPHA,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// A shader of layers (`layer.wgsl`, `yuv.wgsl`) for a layer that is blended with what is under
/// it: the function that lays a layer as it is, between "blend: begin" and "blend: end", put in
/// place by the maths of the composite modes ([`crate::grade::composite_library`], that of the
/// Layer Mixer of a grade, not written a second time) and the function of `blend.wgsl`, which
/// blends by it. The shader of a layer that is not blended has none of this in it.
fn blended_shader(shader: &str) -> String {
    let begin = shader
        .find(BLEND_BEGIN)
        .expect("a shader of layers has a function that lays them");
    let end = shader[begin..]
        .find(BLEND_END)
        .map(|at| begin + at + BLEND_END.len())
        .expect("a shader of layers has a function that lays them");
    format!(
        "{}{}{BLEND_SHADER}{}",
        &shader[..begin],
        crate::grade::composite_library(),
        &shader[end..]
    )
}

/// The shader of pictures (`yuv.wgsl`) for a layer of two pictures of one media, mixed: the
/// functions that read the texels of its planes, between "planes: begin" and "planes: end",
/// put in place by those of `between.wgsl`, which read the planes of either picture and mix the
/// two; and its own function that reads one picture named as `between.wgsl` calls it. The
/// shader of a layer of one picture has none of this in it. What blends a layer with what is
/// under it is put into the shader that this gives as it is into the other
/// ([`blended_shader`]).
fn between_shader(shader: &str) -> String {
    let begin = shader
        .find(PLANES_BEGIN)
        .expect("a shader of pictures has functions that read its planes");
    let end = shader[begin..]
        .find(PLANES_END)
        .map(|at| begin + at + PLANES_END.len())
        .expect("a shader of pictures has functions that read its planes");
    format!(
        "{}{BETWEEN_SHADER}{}",
        &shader[..begin],
        shader[end..].replacen(SAMPLE_ONE, SAMPLE_ONE_OF_TWO, 1)
    )
}

/// What binds the uniform of `yuv.wgsl` and the planes of two pictures of one media read as
/// `texel`s: those of the first as a layer of one picture binds them (1 to 3), and then those
/// of the second (4 to 6), as `between.wgsl` reads them.
fn pictures_layout(device: &wgpu::Device, texel: Texel) -> wgpu::BindGroupLayout {
    let mut entries = vec![wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: true,
            min_binding_size: wgpu::BufferSize::new(yuv::PARAMS_SIZE),
        },
        count: None,
    }];
    entries.extend((1..=6).map(|binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: texel.sample_type(),
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }));
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("columbina pictures"),
        entries: &entries,
    })
}

/// A pipeline of a stage over the canvas (`stage.wgsl`, with its program of colour put in, or as
/// it is written, a copy) that writes into a texture of `format`.
fn stage_pipeline(
    device: &wgpu::Device,
    source: &str,
    layout: &wgpu::BindGroupLayout,
    colour: Option<&wgpu::BindGroupLayout>,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("columbina stage"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("columbina stage"),
        bind_group_layouts: &[Some(layout), colour],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("columbina stage"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// The pipeline of the passes of filters (`neighbourhood.wgsl`), which write into textures of
/// neighbourhoods.
fn filter_pipeline(device: &wgpu::Device, layout: &wgpu::BindGroupLayout) -> wgpu::RenderPipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("columbina filter"),
        source: wgpu::ShaderSource::Wgsl(FILTER_SHADER.into()),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("columbina filter"),
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("columbina filter"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: NEIGHBOURHOOD_FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// What binds the canvas that a stage reads.
fn stage_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("columbina stage"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        }],
    })
}

/// The canvas that a stage reads, bound.
fn stage_bind(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    canvas: &wgpu::Texture,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("columbina stage"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::TextureView(
                &canvas.create_view(&wgpu::TextureViewDescriptor::default()),
            ),
        }],
    })
}

/// What binds what a program of colour reads, with textures of `kinds` (group 1): its
/// parameters, a sampler, and its textures, in the order of `color::program`.
fn colour_layout(device: &wgpu::Device, kinds: &[TextureKind]) -> wgpu::BindGroupLayout {
    let mut entries = vec![
        wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(16),
            },
            count: None,
        },
        wgpu::BindGroupLayoutEntry {
            binding: 1,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        },
    ];
    for (binding, kind) in (color::FIRST_TEXTURE_BINDING..).zip(kinds) {
        entries.push(wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: kind.dimension(),
                multisampled: false,
            },
            count: None,
        });
    }
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("columbina colour"),
        entries: &entries,
    })
}

/// A buffer of the parameters of programs of colour, of `size` bytes.
fn colour_buffer(device: &wgpu::Device, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("columbina colour"),
        size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// A buffer of uniforms of `size` bytes.
fn uniform_buffer(device: &wgpu::Device, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("columbina layers"),
        size,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// Put the pixels of a text on the card.
fn text_texture(gpu: &Gpu, raster: &Raster) -> wgpu::TextureView {
    let texture = gpu.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("columbina text"),
        size: wgpu::Extent3d {
            width: raster.width.max(1),
            height: raster.height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    if raster.width > 0 && raster.height > 0 {
        let bytes: &[u8] = raster.pixels.as_flattened();
        gpu.queue().write_texture(
            texture.as_image_copy(),
            &bytes[raster.start * 4..],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(raster.row * 4),
                rows_per_image: Some(raster.height),
            },
            texture.size(),
        );
    }
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

/// The uniform of a layer that puts the quad `quad` of its frame on a canvas by `place`, at
/// `opacity`, the frame made `scale` times larger, each pixel made of `samples` × `samples`
/// points of it: the second half of the uniform of `yuv.wgsl`, or, with a `tint`, the uniform of
/// `layer.wgsl`. Nothing of it is cut away, and it is not blended: [`cut_and_blend`] says
/// otherwise. Its last eight values are its crop and its composite mode, in both shaders.
#[allow(
    clippy::cast_possible_truncation,
    reason = "pixels and shares, as the card takes them"
)]
fn layer_uniform(
    place: &Affine,
    canvas: (f64, f64),
    quad: [f64; 4],
    opacity: f32,
    scale: f64,
    samples: u32,
    tint: Option<[f32; 4]>,
) -> Vec<f32> {
    let mut values: Vec<f32> = [
        place.a, place.b, place.tx, canvas.0, place.c, place.d, place.ty, canvas.1, quad[0],
        quad[1], quad[2], quad[3],
    ]
    .iter()
    .map(|value| *value as f32)
    .collect();
    #[allow(clippy::cast_precision_loss, reason = "a count from 1 to 8")]
    values.extend([opacity, scale as f32, samples as f32, 0.0]);
    if let Some(tint) = tint {
        values.extend(tint);
        values.extend([0.0, 0.0, 1.0, 1.0]);
    }
    // No side cut ([`Crop::kept`] of no crop), and no composite mode.
    let far = UNCUT as f32;
    values.extend([-far, -far, far, far]);
    values.extend([0.0; 4]);
    values
}

/// Where the softness of the crop of a layer is in its uniform, as [`layer_uniform`] makes it:
/// the fourth value of `extra`.
const FADE_AT: usize = 15;

/// The composite mode of a layer as its uniform has it: the number of the mode, as the maths
/// of a grade numbers them, and the weights `luma` of R, G and B in luma.
fn blend_of(mode: Composite, luma: [f32; 3]) -> [f32; 4] {
    let number = u8::try_from(crate::grade::layout::composite(mode)).unwrap_or(0);
    [f32::from(number), luma[0], luma[1], luma[2]]
}

/// Cut the layer of `uniform` (as [`layer_uniform`] makes it) to `kept` of its frame
/// ([`Crop::kept`]), each cut edge fading over `fade` pixels of the frame; and blend it with
/// what is under it by `blend` ([`blend_of`]), where it is blended.
#[allow(
    clippy::cast_possible_truncation,
    reason = "pixels of a frame, as the card takes them"
)]
fn cut_and_blend(uniform: &mut [f32], kept: [f64; 4], fade: f64, blend: Option<[f32; 4]>) {
    let end = uniform.len();
    uniform[FADE_AT] = fade as f32;
    uniform[end - 8..end - 4].copy_from_slice(&kept.map(|side| side as f32));
    if let Some(blend) = blend {
        uniform[end - 4..].copy_from_slice(&blend);
    }
}

/// Map a title's visible texture into its complete transparent source canvas. The same map
/// is used while rendering its cropped neighbourhood texture and while placing it on canvas.
#[allow(
    clippy::cast_possible_truncation,
    reason = "normalized source coordinates for the shader"
)]
fn window_canvas(uniform: &mut [f32], quad: [f64; 4], canvas: (f64, f64)) {
    uniform[20..24].copy_from_slice(&[
        (quad[0] / canvas.0) as f32,
        (quad[1] / canvas.1) as f32,
        ((quad[2] - quad[0]) / canvas.0) as f32,
        ((quad[3] - quad[1]) / canvas.1) as f32,
    ]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_layer_is_given_to_the_card_as_rows_of_its_map_its_quad_and_its_opacity() {
        let place = placement((200.0, 100.0), (100.0, 100.0), &Transform::NONE);
        let values = layer_uniform(
            &place,
            (200.0, 100.0),
            [0.0, 0.0, 100.0, 100.0],
            0.5,
            1.0,
            3,
            None,
        );
        #[allow(clippy::cast_possible_truncation, reason = "as the card takes it")]
        let far = UNCUT as f32;
        assert_eq!(
            values[..16],
            [
                1.0, 0.0, 50.0, 200.0, 0.0, 1.0, 0.0, 100.0, 0.0, 0.0, 100.0, 100.0, 0.5, 1.0, 3.0,
                0.0
            ]
        );
        // No side is cut, and it is not blended.
        assert_eq!(values[16..], [-far, -far, far, far, 0.0, 0.0, 0.0, 0.0]);
        let tinted = layer_uniform(
            &place,
            (200.0, 100.0),
            [0.0; 4],
            1.0,
            2.0,
            1,
            Some([0.1; 4]),
        );
        assert_eq!(u64::try_from(tinted.len() * 4).unwrap(), LAYER_PARAMS_SIZE);
        assert_eq!(
            u64::try_from((values.len() + yuv::PICTURE_PARAMS) * 4).unwrap(),
            yuv::PARAMS_SIZE
        );
        assert_eq!(tinted[13], 2.0);
        assert_eq!(tinted[16..20], [0.1; 4]);
        assert_eq!(tinted[20..24], [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(tinted[24..], [-far, -far, far, far, 0.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn a_crop_and_a_composite_mode_are_the_last_values_of_the_uniform_of_a_layer() {
        let canvas = (200.0, 100.0);
        let place = placement(canvas, canvas, &Transform::NONE);
        let crop = Crop {
            left: 0.25,
            bottom: 0.5,
            softness: 0.1,
            ..Crop::NONE
        };
        let kept = crop.kept(canvas).unwrap();
        let blend = blend_of(Composite::Multiply, [0.25, 0.5, 0.25]);
        assert_eq!(blend, [19.0, 0.25, 0.5, 0.25]);
        #[allow(clippy::cast_possible_truncation, reason = "as the card takes it")]
        let far = UNCUT as f32;
        // Of a picture, after what decodes it; of a title or a colour, after its tint.
        for tint in [None, Some([1.0; 4])] {
            let whole = [0.0, 0.0, canvas.0, canvas.1];
            let plain = layer_uniform(&place, canvas, whole, 1.0, 1.0, 1, tint);
            let mut values = plain.clone();
            cut_and_blend(&mut values, kept, crop.fade(canvas.1), Some(blend));
            let end = values.len();
            assert_eq!(values[FADE_AT], 10.0);
            assert_eq!(values[end - 8..end - 4], [50.0, -far, far, 50.0]);
            assert_eq!(values[end - 4..], blend);
            // Nothing else of it is changed.
            assert_eq!(values[..FADE_AT], plain[..FADE_AT]);
            assert_eq!(values[FADE_AT + 1..end - 8], plain[FADE_AT + 1..end - 8]);
            // A layer that is not blended keeps no mode.
            let mut values = plain.clone();
            cut_and_blend(&mut values, kept, 0.0, None);
            assert_eq!(values[end - 4..], [0.0; 4]);
        }
        assert_eq!(blend_of(Composite::Normal, [0.0; 3])[0], 0.0);
        assert_eq!(blend_of(Composite::VividLight, [0.0; 3])[0], 26.0);
    }

    #[test]
    fn the_shader_of_a_layer_that_is_blended_has_the_maths_of_the_modes_and_the_others_do_not() {
        for shader in [LAYER_SHADER.to_owned(), Texel::Float.shader()] {
            // As it is written: the layer is laid as it is, and nothing reads the canvas.
            assert_eq!(shader.matches(BLEND_BEGIN).count(), 1);
            assert_eq!(shader.matches(BLEND_END).count(), 1);
            assert!(!shader.contains("layer_composite"));
            assert!(!shader.contains("backdrop"));
            let blended = blended_shader(&shader);
            assert!(!blended.contains(BLEND_BEGIN));
            assert!(!blended.contains(BLEND_END));
            assert_eq!(blended.matches("fn laid(").count(), 1);
            assert_eq!(blended.matches("fn layer_composite(").count(), 1);
            assert_eq!(blended.matches("fn layer_luma(").count(), 1);
            assert!(blended.contains("@group(2) @binding(0) var backdrop"));
            // By names of its own, which those of the grade of a layer do not meet.
            assert!(!blended.contains("grade_"));
            assert!(!blended.contains("GRADE_"));
            assert!(blended.contains("var<private> LAYER_LUMA: vec3<f32>;"));
            // The function of colour of the layer is still where a program is put.
            assert_eq!(blended.matches(color::IDENTITY_BEGIN).count(), 1);
            assert_eq!(blended.matches(color::IDENTITY_END).count(), 1);
            // Nothing else of the shader is changed.
            let begin = shader.find(BLEND_BEGIN).unwrap();
            let end = shader.find(BLEND_END).unwrap() + BLEND_END.len();
            assert!(blended.starts_with(&shader[..begin]));
            assert!(blended.ends_with(&shader[end..]));
        }
    }

    #[test]
    fn the_shader_of_two_pictures_reads_the_planes_of_both_and_that_of_one_does_not() {
        for texel in [Texel::Float, Texel::Uint] {
            let shader = texel.shader();
            // As it is written: one picture is read, and no second one is bound.
            assert_eq!(shader.matches(PLANES_BEGIN).count(), 1);
            assert_eq!(shader.matches(PLANES_END).count(), 1);
            assert_eq!(shader.matches(SAMPLE_ONE).count(), 1);
            assert!(!shader.contains("picture_at"));
            assert!(!shader.contains("second_"));
            assert!(!shader.contains("@binding(4)"));
            let between = between_shader(&shader);
            assert!(!between.contains(PLANES_BEGIN));
            assert!(!between.contains(PLANES_END));
            // The function that read the one picture reads either, and a new one mixes them.
            assert_eq!(between.matches(SAMPLE_ONE).count(), 1);
            assert_eq!(between.matches(SAMPLE_ONE_OF_TWO).count(), 1);
            assert!(between.contains("mix(first, second, params.siting.z)"));
            for function in ["luma_at", "chroma_at", "rgba_at", "alpha_at"] {
                assert_eq!(
                    between.matches(&format!("fn {function}(")).count(),
                    1,
                    "{function}"
                );
            }
            for (binding, plane) in [(4, "luma"), (5, "chroma"), (6, "alpha")] {
                let bound = format!("@group(0) @binding({binding}) var second_{plane}_plane");
                assert!(between.contains(&bound), "{bound}");
            }
            // Its texels are read as those of the first are.
            let alias = if texel == Texel::Uint { "u32" } else { "f32" };
            assert!(between.contains(&format!("\nalias Texel = {alias};")));
            // Nothing else of the shader is changed, but the name of the one function.
            let begin = shader.find(PLANES_BEGIN).unwrap();
            let end = shader.find(PLANES_END).unwrap() + PLANES_END.len();
            assert!(between.starts_with(&shader[..begin]));
            let rest = shader[end..].replacen(SAMPLE_ONE, SAMPLE_ONE_OF_TWO, 1);
            assert!(between.ends_with(&rest));
            // Its colour and its composite mode are put in as they are into the other.
            assert_eq!(between.matches(color::IDENTITY_BEGIN).count(), 1);
            assert_eq!(between.matches(color::IDENTITY_END).count(), 1);
            let blended = blended_shader(&between);
            assert!(blended.contains("@group(2) @binding(0) var backdrop"));
            assert!(blended.contains("var second_luma_plane"));
        }
    }

    #[test]
    fn a_layer_of_two_pictures_of_which_all_of_one_is_shown_is_a_layer_of_that_one() {
        let picture = |frame: i64| {
            Arc::new(NativePicture {
                frame,
                ..NativePicture::default()
            })
        };
        let between = |weight: f32| Source::Between {
            first: picture(1),
            second: picture(2),
            weight,
            rotation: 90,
        };
        // The number of the first, of the second where two are mixed, and its weight.
        let drawn = |source: &Source| {
            let pictures = pictures_of(source).expect("a layer of pictures");
            assert_eq!(pictures.rotation, 90);
            let second = pictures
                .second
                .map(|(second, weight)| (second.frame, weight));
            (pictures.first.frame, second)
        };
        assert_eq!(drawn(&between(0.25)), (1, Some((2, 0.25))));
        assert_eq!(drawn(&between(0.0)), (1, None));
        assert_eq!(drawn(&between(-1.0)), (1, None));
        assert_eq!(drawn(&between(f32::NAN)), (1, None));
        assert_eq!(drawn(&between(1.0)), (2, None));
        assert_eq!(drawn(&between(2.0)), (2, None));
        assert_eq!(drawn(&Source::Picture(picture(7), 90)), (7, None));
        assert!(pictures_of(&Source::Color([0.0; 4])).is_none());
        // The kinds of pipelines of two pictures are blended as those of one are.
        let two = Pipe::YuvBetween(Texel::Float);
        assert_eq!(two.blended(), Pipe::YuvBetweenBlended(Texel::Float));
        assert!(two.blended().is_blended() && !two.is_blended());
        assert_eq!(two.blended().blended(), two.blended());
    }

    #[test]
    fn a_colour_of_the_project_is_taken_from_0_to_1() {
        let Source::Color(colour) = Source::color(Rgba([255, 0, 51, 128])) else {
            panic!("a colour");
        };
        assert_eq!(colour, [1.0, 0.0, 0.2, 128.0 / 255.0]);
    }

    #[test]
    fn a_compositor_and_its_targets_can_be_sent_to_the_threads_that_play_and_render() {
        fn shared<T: Send + Sync>() {}
        shared::<Compositor>();
        shared::<Target>();
        shared::<Scene>();
    }

    #[test]
    fn a_title_of_a_few_hundred_thousand_pixels_is_drawn_at_its_own_scale() {
        let Some(gpu) = crate::gpu_for_tests() else {
            return;
        };
        let mut compositor = Compositor::new(gpu);
        // Some 512 × 256 pixels, well within 4096 × 2048.
        let spec = Spec {
            text: "Columbina".into(),
            family: None,
            size: 96.0,
            align: TextAlign::Center,
            color: [255; 4],
            outline: 0,
        };
        let canvas = (1920.0, 1080.0);
        let place = placement(canvas, canvas, &Transform::NONE);
        let put = compositor.put_text(&spec, 1.0, |_| (100.0, 100.0), &place, canvas);
        assert!(matches!(put, Put::Drawn(..)), "drawn smaller");
    }

    #[test]
    fn half_floats_are_read_as_8_bits_rounded_and_clamped() {
        // 0, 1, 0.5 (127.5, to the even 128), 0.25, 2, -1, a subnormal, infinity, not a number.
        let halves = [
            0x0000, 0x3C00, 0x3800, 0x3400, 0x4000, 0xBC00, 0x0001, 0x7C00, 0x7E00,
        ];
        assert_eq!(halves.map(eight_bits), [0, 255, 128, 64, 255, 0, 0, 255, 0]);
        // Every code of 8 bits, as the canvas holds it (the nearest half float), comes back as
        // it was.
        for code in 0..=255_u8 {
            let value = f32::from(code) / 255.0;
            let nearest = (0..=0x3C00_u16)
                .min_by(|a, b| {
                    (half_value(*a) - value)
                        .abs()
                        .total_cmp(&(half_value(*b) - value).abs())
                })
                .unwrap();
            assert_eq!(eight_bits(nearest), code, "{code}");
        }
    }

    #[test]
    fn errors_have_a_word_each() {
        assert_eq!(ComposeError::Lost(String::new()).reason(), "lost");
        assert_eq!(ComposeError::Target(String::new()).reason(), "target");
        assert_eq!(
            ComposeError::Layer {
                layer: 0,
                message: String::new()
            }
            .reason(),
            "layer"
        );
        assert_eq!(ComposeError::Read(String::new()).reason(), "read");
    }
}
