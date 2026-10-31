//! The compositor: the layers of a frame put together on the card into a [`Target`], a texture
//! that a view shows (a preview) or that is made Y'CbCr and read back for an encoder (an export).
//!
//! A frame goes through the stages of colour that `color.rs` lays out, in the order of
//! `DaVinci Resolve`. Each layer, from the bottom up, is made R'G'B' (a picture from Y'CbCr as it is drawn,
//! `yuv.wgsl`, by its matrix and range; a title drawn into pixels on the processor, see
//! `text.rs`, and kept for as long as the same text is drawn; a colour), goes through its input
//! transform and its grade, is fitted to the canvas, put on it by its transform, and laid over
//! what is there by its opacity, premultiplied: `out = layer + (1 - alpha of layer) * under`. The
//! canvas is black at first, and of half floats ([`WORKING_FORMAT`]) for a preview and an export
//! alike: the working space of the timeline. Colours are mixed as they are coded, with their gamma
//! (as long as there is no colour management, the working space is R'G'B' of Rec.709). Then the
//! timeline grade is done over the canvas, the subtitles are drawn, and the output transform is
//! done last.
//!
//! Subtitles are drawn over the timeline grade, in a preview only: white with a black outline, in
//! the middle at the bottom, letters a twentieth of the height of the canvas high.

use std::collections::HashMap;
use std::sync::{mpsc, Arc, OnceLock};
use std::time::{Duration, Instant};

use columbina_media::{NativePicture, Picture, PictureFormat};
use columbina_model::project::{Rgba, TextAlign};

use crate::color::{self, Inputs, LayerColor, Program, Stage, TextureKind};
use crate::geometry::{
    fit, minification, picture_stride, placement, raster_scale, samples, shown_size,
    subtitle_origin, subtitle_size, text_window, title_origin, Affine, Transform,
};
use crate::gpu::Gpu;
use crate::text::{self, Laid, Raster, Spec, Text, MOST_PIXELS};
use crate::yuv::{self, Texel};

/// The shader of layers of R'G'B'A.
const LAYER_SHADER: &str = include_str!("layer.wgsl");

/// The shader that makes the canvas Y'CbCr for an encoder.
const CONVERT_SHADER: &str = include_str!("convert.wgsl");

/// The shader of a stage of colour over the canvas, which, as it is written, copies it.
const STAGE_SHADER: &str = include_str!("stage.wgsl");

/// The format of the canvas, the working space of the timeline, for a preview and an export.
pub const WORKING_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// The bytes of the uniform of `layer.wgsl`.
const LAYER_PARAMS_SIZE: u64 = 80;

/// The bytes of the uniform of `convert.wgsl`.
const CONVERT_PARAMS_SIZE: u64 = 32;

/// The invocations of a group of the shaders of `convert.wgsl`, across.
const CONVERT_GROUP: u32 = 64;

/// How many texts laid out are kept, the last drawn.
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
    /// A title: its text, placed on the canvas as its style says.
    Title(Text),
    /// A colour over the canvas: R', G', B' and how opaque it is, from 0 to 1, not
    /// premultiplied.
    Color([f32; 4]),
}

impl Source {
    /// A colour as the project writes it.
    #[must_use]
    pub fn color(rgba: Rgba) -> Source {
        Source::Color(rgba.0.map(|channel| f32::from(channel) / 255.0))
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
    /// ([`WORKING_FORMAT`]), that can be drawn from and copied.
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
                pixels.push(eight_bits(u16::from_le_bytes(*half)));
            }
        }
        drop(mapped);
        buffer.unmap();
        Ok(pixels)
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
            | wgpu::TextureUsages::COPY_SRC,
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
}

/// The colour of a layer drawn, where it does something: the number of the layer in its scene,
/// of its program, and what the program reads.
struct Graded {
    layer: usize,
    program: usize,
    inputs: Inputs,
}

/// A layer, or the subtitles, to be drawn.
struct Call {
    pipe: Pipe,
    program: Option<usize>,
    bind: wgpu::BindGroup,
    offset: u32,
    colour: Option<wgpu::BindGroup>,
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
    /// Over this quad of its frame, with this texture.
    Drawn([f64; 4], wgpu::TextureView),
    /// Nowhere on the canvas.
    Unseen,
    /// Too large to draw at once: to be laid out this many times smaller, at least.
    TooLarge(f64),
}

/// What is drawn for a layer.
enum Draw {
    Picture { shape: Shape, planes: Planes },
    Texture(wgpu::TextureView),
}

/// Puts the layers of scenes together on a card. It keeps what it made from frame to frame: the
/// pipelines, the textures of the planes of pictures of each shape, and the titles drawn.
pub struct Compositor {
    gpu: Arc<Gpu>,
    yuv_layouts: HashMap<Texel, wgpu::BindGroupLayout>,
    rgba_layout: wgpu::BindGroupLayout,
    stage_layout: wgpu::BindGroupLayout,
    convert_layout: wgpu::BindGroupLayout,
    /// What binds what a program of colour reads (group 1), by the kinds of its textures.
    colour_layouts: HashMap<Vec<TextureKind>, wgpu::BindGroupLayout>,
    /// The programs of colour met, by their code; each is known by its place here.
    programs: Vec<Program>,
    program_numbers: HashMap<String, usize>,
    /// The pipelines, by their kind, the format they draw into, and their program of colour.
    pipelines: HashMap<(Pipe, wgpu::TextureFormat, Option<usize>), wgpu::RenderPipeline>,
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
        Compositor {
            yuv_layouts,
            rgba_layout,
            stage_layout: stage_layout(device),
            convert_layout,
            colour_layouts: HashMap::new(),
            programs: Vec::new(),
            program_numbers: HashMap::new(),
            pipelines: HashMap::new(),
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
            frame: 0,
            gpu,
        }
    }

    /// The card that it composes on.
    #[must_use]
    pub fn gpu(&self) -> &Arc<Gpu> {
        &self.gpu
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
        let canvas = (f64::from(scene.width), f64::from(scene.height));
        let whole = [0.0, 0.0, canvas.0, canvas.1];
        let mut draws: Vec<Draw> = Vec::new();
        let mut uniforms: Vec<Vec<f32>> = Vec::new();
        let mut colours: Vec<Option<Graded>> = Vec::new();
        let laid = self.layers(
            scene,
            canvas,
            whole,
            &mut draws,
            &mut uniforms,
            &mut colours,
        );
        if let Err(error) = laid {
            self.give_back(draws);
            return Err(error);
        }
        // The stages over the canvas, where they do something.
        let grade =
            color::program(&[&scene.grade]).map(|(program, inputs)| (self.number(program), inputs));
        let output = color::program(&[&scene.output])
            .map(|(program, inputs)| (self.number(program), inputs));
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
                Ok(Some((quad, view))) => {
                    uniforms.push(layer_uniform(
                        &place,
                        canvas,
                        quad,
                        1.0,
                        1.0,
                        1,
                        Some([1.0; 4]),
                    ));
                    draws.push(Draw::Texture(view));
                    colours.push(None);
                }
                Ok(None) => {}
                Err(message) => {
                    self.give_back(draws);
                    return Err(ComposeError::Layer {
                        layer: scene.layers.len(),
                        message,
                    });
                }
            }
        }

        // The uniforms of all the layers, one slot each.
        let offsets = match self.write_uniforms(&uniforms) {
            Ok(offsets) => offsets,
            Err(error) => {
                self.give_back(draws);
                return Err(error);
            }
        };

        let format = target.texture.format();
        let made = self.pipelines_of(&draws, &colours, [&grade, &output], format);
        if let Err(error) = made {
            self.give_back(draws);
            return Err(error);
        }
        let mut calls = Vec::with_capacity(draws.len());
        for (draw, offset) in draws.iter_mut().zip(offsets) {
            let (pipe, bind) = match draw {
                Draw::Picture { shape, planes } => {
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
                Draw::Texture(view) => (Pipe::Rgba, self.texture_bind(view)),
            };
            calls.push(Call {
                pipe,
                program: None,
                bind,
                offset,
                colour: None,
            });
        }
        // What the programs of colour read: those of the layers, then of the stages.
        let mut blocks: Vec<(usize, &Inputs)> = Vec::new();
        for colour in colours.iter().flatten() {
            blocks.push((colour.program, &colour.inputs));
        }
        for (program, inputs) in [&grade, &output].into_iter().flatten() {
            blocks.push((*program, inputs));
        }
        let mut colour_binds = self.colour_binds(&blocks).into_iter();
        for (call, colour) in calls.iter_mut().zip(&colours) {
            if let Some(colour) = colour {
                call.program = Some(colour.program);
                call.colour = colour_binds.next();
            }
        }
        let stage_binds: Vec<(usize, wgpu::BindGroup)> = [&grade, &output]
            .into_iter()
            .flatten()
            .map(|(program, _)| *program)
            .zip(colour_binds)
            .collect();
        let (grade, output) = match (&grade, &output) {
            (Some(_), _) => (stage_binds.first(), stage_binds.get(1)),
            (None, _) => (None, stage_binds.first()),
        };

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
        // The layers, and the subtitles over them where there is no timeline grade.
        let first = if grade.is_some() {
            layered
        } else {
            calls.len()
        };
        self.draw_calls(&mut encoder, current, &calls[..first], format, true);
        if let Some((program, colour)) = grade {
            let next = other(current);
            self.stage_pass(&mut encoder, current, next, *program, colour);
            current = next;
            if first < calls.len() {
                self.draw_calls(&mut encoder, current, &calls[first..], format, false);
            }
        }
        if let Some((program, colour)) = output {
            let next = other(current);
            self.stage_pass(&mut encoder, current, next, *program, colour);
            current = next;
        }
        debug_assert!(std::ptr::eq(current, main), "the frame ends in the target");
        if let Some(export) = &target.export {
            let view = main.create_view(&wgpu::TextureViewDescriptor::default());
            self.convert(&mut encoder, &view, export, target.height);
        }
        self.gpu.queue().submit([encoder.finish()]);
        self.give_back(draws);
        self.forget_unused();
        Ok(())
    }

    /// Draw `calls` into `canvas`, cleared to black first or drawn over.
    fn draw_calls(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        canvas: &wgpu::Texture,
        calls: &[Call],
        format: wgpu::TextureFormat,
        clear: bool,
    ) {
        let view = canvas.create_view(&wgpu::TextureViewDescriptor::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("columbina layers"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: if clear {
                        wgpu::LoadOp::Clear(wgpu::Color::BLACK)
                    } else {
                        wgpu::LoadOp::Load
                    },
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        for call in calls {
            pass.set_pipeline(&self.pipelines[&(call.pipe, format, call.program)]);
            pass.set_bind_group(0, &call.bind, &[call.offset]);
            if let Some(colour) = &call.colour {
                pass.set_bind_group(1, colour, &[]);
            }
            pass.draw(0..6, 0..1);
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

    /// Make the pipelines that the draws and the stages of a frame need, where they are not
    /// made yet.
    fn pipelines_of(
        &mut self,
        draws: &[Draw],
        colours: &[Option<Graded>],
        stages: [&Option<(usize, Inputs)>; 2],
        format: wgpu::TextureFormat,
    ) -> Result<(), ComposeError> {
        for (draw, colour) in draws.iter().zip(colours) {
            let pipe = match draw {
                Draw::Picture { shape, .. } => Pipe::Yuv(shape.texel),
                Draw::Texture(_) => Pipe::Rgba,
            };
            self.pipeline(pipe, format, colour.as_ref().map(|colour| colour.program))
                .map_err(|message| ComposeError::Layer {
                    layer: colour.as_ref().map_or(0, |colour| colour.layer),
                    message,
                })?;
        }
        for (program, _) in stages.into_iter().flatten() {
            self.pipeline(Pipe::Stage, format, Some(*program))
                .map_err(ComposeError::Color)?;
        }
        Ok(())
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

    /// The draws and uniforms of the layers of a scene. The planes taken for pictures are in
    /// `draws`, to be given back, whatever comes of it.
    #[allow(
        clippy::too_many_lines,
        reason = "each kind of layer, one after another"
    )]
    fn layers(
        &mut self,
        scene: &Scene,
        canvas: (f64, f64),
        whole: [f64; 4],
        draws: &mut Vec<Draw>,
        uniforms: &mut Vec<Vec<f32>>,
        colours: &mut Vec<Option<Graded>>,
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
            // Its input transform and its grade, after it is decoded.
            let graded = color::program(&[&layer.color.input, &layer.color.grade]).map(
                |(program, inputs)| Graded {
                    layer: index,
                    program: self.number(program),
                    inputs,
                },
            );
            match &layer.source {
                Source::Picture(picture, rotation) => {
                    let layout = yuv::check(picture).map_err(failed)?;
                    let most = self.gpu.device().limits().max_texture_dimension_2d;
                    if picture.width > most || picture.height > most {
                        return Err(failed(format!(
                            "a picture of {}×{} is larger than the {most}×{most} that the card \
                             takes",
                            picture.width, picture.height
                        )));
                    }
                    let shown = shown_size((picture.width, picture.height), *rotation);
                    let frame = fit(canvas, (f64::from(shown.0), f64::from(shown.1)));
                    if frame.0 <= 0.0 || frame.1 <= 0.0 {
                        continue;
                    }
                    let shape = Shape {
                        width: picture.width,
                        height: picture.height,
                        format: picture.format,
                        alpha: layout.alpha_plane(),
                        texel: layout.texel(self.gpu.has_sixteen_norm()),
                    };
                    let planes = self.take_planes(shape);
                    let uploaded = yuv::upload(self.gpu.queue(), &planes.textures, picture);
                    draws.push(Draw::Picture { shape, planes });
                    colours.push(graded);
                    uploaded.map_err(failed)?;
                    let mut values = yuv::picture_params(
                        [0.0; 4],
                        layout,
                        shape.texel,
                        layer.color.coding,
                        *rotation,
                    )
                    .to_vec();
                    values.extend(layer_uniform(
                        &placement(canvas, frame, &layer.transform),
                        canvas,
                        [0.0, 0.0, frame.0, frame.1],
                        opacity,
                        scale,
                        samples(minification(shown, frame, scale)),
                        None,
                    ));
                    uniforms.push(values);
                }
                Source::Color(rgba) => {
                    uniforms.push(layer_uniform(
                        &placement(canvas, canvas, &layer.transform),
                        canvas,
                        whole,
                        opacity,
                        scale,
                        1,
                        Some(rgba.map(|channel| channel.clamp(0.0, 1.0))),
                    ));
                    draws.push(Draw::Texture(self.white.clone()));
                    colours.push(graded);
                }
                Source::Title(title) => {
                    let style = &title.style;
                    let place = placement(canvas, canvas, &layer.transform);
                    #[allow(clippy::cast_possible_truncation, reason = "a size of letters")]
                    let size = |per: f64| {
                        (f64::from(style.size.min(4096)) * f64::from(scene.units) * per) as f32
                    };
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
                    if let Some((quad, view)) = put.map_err(failed)? {
                        uniforms.push(layer_uniform(
                            &place,
                            canvas,
                            quad,
                            opacity,
                            scale,
                            1,
                            Some([1.0; 4]),
                        ));
                        draws.push(Draw::Texture(view));
                        colours.push(graded);
                    }
                }
            }
        }
        Ok(())
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

    /// Keep the planes of the draws of a frame, for the next.
    fn give_back(&mut self, draws: Vec<Draw>) {
        for draw in draws {
            if let Draw::Picture { shape, planes } = draw {
                self.planes.entry(shape).or_default().push(planes);
            }
        }
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
    ) -> Result<Option<([f64; 4], wgpu::TextureView)>, String> {
        let mut per = per;
        for _ in 0..SHRINKS {
            match self.put_text(&spec(per), per, &origin, place, canvas) {
                Put::Drawn(quad, view) => return Ok(Some((quad, view))),
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
        Put::Drawn(quad, view)
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

    /// The pipeline of `pipe` that draws into targets of `format`, with a program of colour or
    /// none, made if it is not yet.
    ///
    /// # Errors
    /// The code of the program is not WGSL that the card takes, or reads what is not bound.
    fn pipeline(
        &mut self,
        pipe: Pipe,
        format: wgpu::TextureFormat,
        program: Option<usize>,
    ) -> Result<(), String> {
        if self.pipelines.contains_key(&(pipe, format, program)) {
            return Ok(());
        }
        if let Some(number) = program {
            let kinds = &self.programs[number].textures;
            if !self.colour_layouts.contains_key(kinds) {
                let layout = colour_layout(self.gpu.device(), kinds);
                self.colour_layouts.insert(kinds.clone(), layout);
            }
        }
        let device = self.gpu.device();
        let (shader, layout) = match pipe {
            Pipe::Yuv(texel) => (texel.shader(), &self.yuv_layouts[&texel]),
            Pipe::Rgba => (LAYER_SHADER.to_owned(), &self.rgba_layout),
            Pipe::Stage => (STAGE_SHADER.to_owned(), &self.stage_layout),
        };
        let (source, colour) = match program {
            Some(number) => {
                let program = &self.programs[number];
                (
                    color::with_program(&shader, program),
                    Some(&self.colour_layouts[&program.textures]),
                )
            }
            None => (shader, None),
        };
        // The code of a program is not the compositor's: what is wrong with it is caught here,
        // and not left to the card's handler of errors.
        let scope = program.map(|_| device.push_error_scope(wgpu::ErrorFilter::Validation));
        let pipeline = if pipe == Pipe::Stage {
            stage_pipeline(device, &source, layout, colour, format)
        } else {
            layer_pipeline(device, &source, layout, colour, format)
        };
        if let Some(scope) = scope {
            if let Some(error) = pollster::block_on(scope.pop()) {
                return Err(format!("its colour cannot be done: {error}"));
            }
        }
        self.pipelines.insert((pipe, format, program), pipeline);
        Ok(())
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

/// A pipeline that draws layers of the shader `source` into targets of `format`, laid over what
/// is there, premultiplied; with what a program of colour reads as group 1, where it has one.
fn layer_pipeline(
    device: &wgpu::Device,
    source: &str,
    layout: &wgpu::BindGroupLayout,
    colour: Option<&wgpu::BindGroupLayout>,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("columbina layer"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("columbina layer"),
        bind_group_layouts: &[Some(layout), colour],
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
/// `layer.wgsl`.
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
    }
    values
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
        assert_eq!(
            values,
            [
                1.0, 0.0, 50.0, 200.0, 0.0, 1.0, 0.0, 100.0, 0.0, 0.0, 100.0, 100.0, 0.5, 1.0, 3.0,
                0.0
            ]
        );
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
        assert_eq!(tinted[16..], [0.1; 4]);
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
