//! The compositor: the layers of a frame put together on the card into a [`Target`], a texture
//! that a view shows (a preview) or that is made Y'CbCr and read back for an encoder (an export).
//!
//! Each layer, from the bottom up, is fitted to the canvas, put on it by its transform, and laid
//! over what is there by its opacity, premultiplied: `out = layer + (1 - alpha of layer) * under`.
//! A picture is turned from Y'CbCr into R'G'B' as it is drawn (`yuv.wgsl`, BT.709 of limited
//! range); a title is drawn into pixels on the processor (see `text.rs`) and kept, for as long as
//! the same text is drawn; a colour covers the canvas. The canvas is black at first. Colours are
//! mixed as they are coded, with their gamma (the working space of phase 1 is R'G'B' of Rec.709;
//! colour is managed in phase 2).
//!
//! Subtitles are drawn over everything, in a preview only: white with a black outline, in the
//! middle at the bottom, letters a twentieth of the height of the canvas high.

use std::collections::HashMap;
use std::sync::{mpsc, Arc, Mutex, PoisonError};

use columbina_media::Picture;
use columbina_model::project::{Rgba, TextAlign};

use crate::geometry::{
    fit, picture_stride, placement, shown_size, subtitle_origin, subtitle_size, title_origin,
    Affine, Transform,
};
use crate::gpu::Gpu;
use crate::text::{self, Raster, Text};
use crate::yuv::{self, Texel};

/// The shader of layers of R'G'B'A.
const LAYER_SHADER: &str = include_str!("layer.wgsl");

/// The shader that makes the canvas Y'CbCr for an encoder.
const CONVERT_SHADER: &str = include_str!("convert.wgsl");

/// The bytes of the uniform of `layer.wgsl`.
const LAYER_PARAMS_SIZE: u64 = 80;

/// The bytes of the uniform of `convert.wgsl`.
const CONVERT_PARAMS_SIZE: u64 = 32;

/// The invocations of a group of the shaders of `convert.wgsl`, across.
const CONVERT_GROUP: u32 = 64;

/// How many texts drawn into pixels are kept, the last drawn.
const TEXTS_KEPT: usize = 64;

/// How many frames the planes of a shape of picture are kept for while no picture of that shape
/// is drawn.
const PLANES_KEPT_FOR: u64 = 300;

/// What a frame shows: layers over a black canvas of `width` × `height`, and subtitles over
/// them (in a preview only).
#[derive(Clone, Debug, Default)]
pub struct Scene {
    pub width: u32,
    pub height: u32,
    /// From the bottom up.
    pub layers: Vec<SceneLayer>,
    /// The texts of the subtitles shown, a line (or more) each, from the top down.
    pub subtitles: Vec<String>,
}

/// What a layer shows.
#[derive(Clone, Debug)]
pub enum Source {
    /// A picture, turned clockwise so many degrees (as its media says) to be shown.
    Picture(Arc<Picture>, i16),
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
        }
    }
}

/// What a frame is composed into.
///
/// A **preview** target is an `Rgba8Unorm` texture of the size of the canvas, which a view on
/// the same card can draw ([`Target::texture`]). An **export** target composes into a canvas of
/// half floats, which is then made NV12 (8 bits) or P010 (10 bits) on the card, and read back
/// into a [`Picture`] with [`Target::read_picture`].
///
/// A target is made for a size and kept from frame to frame; one that is being read must not
/// be rendered into at the same time (render into another, to read one while the next is drawn).
pub struct Target {
    gpu: Arc<Gpu>,
    width: u32,
    height: u32,
    texture: Arc<wgpu::Texture>,
    export: Option<Export>,
    /// The last submission that drew into it.
    submitted: Mutex<Option<wgpu::SubmissionIndex>>,
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
    /// A target for a preview: a texture of `width` × `height`, `Rgba8Unorm`, that can be drawn
    /// from and copied.
    ///
    /// # Errors
    /// The size is none, or more than the card takes.
    pub fn preview(gpu: &Arc<Gpu>, width: u32, height: u32) -> Result<Target, ComposeError> {
        check_size(gpu, width, height)?;
        let texture = canvas(
            gpu,
            width,
            height,
            wgpu::TextureFormat::Rgba8Unorm,
            "preview",
        );
        Ok(Target {
            gpu: Arc::clone(gpu),
            width,
            height,
            texture: Arc::new(texture),
            export: None,
            submitted: Mutex::new(None),
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
        let texture = canvas(
            gpu,
            width,
            height,
            wgpu::TextureFormat::Rgba16Float,
            "export",
        );
        Ok(Target {
            gpu: Arc::clone(gpu),
            width,
            height,
            texture: Arc::new(texture),
            export: Some(export),
            submitted: Mutex::new(None),
        })
    }

    /// The texture that layers are composed into: for a preview, the picture itself, which a
    /// view on the same card can draw.
    #[must_use]
    pub fn texture(&self) -> &Arc<wgpu::Texture> {
        &self.texture
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
        let submission = self
            .submitted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        wait_mapped(&self.gpu, &buffers, submission)?;
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

    /// Wait for what was last rendered into a preview, and read it back: R'G'B'A, 8 bits each,
    /// a row after another, `width × 4` bytes a row. For the tests, and for checks of what a view
    /// shows.
    ///
    /// # Errors
    /// It is an export, or the card could not give the picture back (it was lost).
    pub fn read_rgba(&self) -> Result<Vec<u8>, ComposeError> {
        if self.export.is_some() {
            return Err(ComposeError::Target(
                "an export is read back as a picture".into(),
            ));
        }
        let device = self.gpu.device();
        let row = (self.width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
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
        let submission = self.gpu.queue().submit([encoder.finish()]);
        wait_mapped(&self.gpu, &[&buffer], Some(submission))?;
        let mapped = buffer
            .slice(..)
            .get_mapped_range()
            .map_err(|error| ComposeError::Read(error.to_string()))?;
        let width = self.width as usize * 4;
        let mut pixels = Vec::with_capacity(width * self.height as usize);
        for line in mapped.chunks(row as usize) {
            pixels.extend_from_slice(&line[..width]);
        }
        drop(mapped);
        buffer.unmap();
        Ok(pixels)
    }
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

/// Map `buffers` for reading, and wait until they are mapped.
fn wait_mapped(
    gpu: &Gpu,
    buffers: &[&wgpu::Buffer],
    submission: Option<wgpu::SubmissionIndex>,
) -> Result<(), ComposeError> {
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
    gpu.device()
        .poll(wgpu::PollType::Wait {
            submission_index: submission,
            timeout: None,
        })
        .map_err(|error| ComposeError::Read(error.to_string()))?;
    let mut failed = None;
    for _ in buffers {
        match mapping.recv() {
            Ok(Ok(())) => {}
            Ok(Err(error)) => failed = Some(error.to_string()),
            Err(_) => failed = Some("the card did not map the buffer".into()),
        }
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

/// The kinds of pipelines that draw layers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Pipe {
    Yuv(Texel),
    Rgba,
}

/// The shape of the planes of a picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Shape {
    width: u32,
    height: u32,
    ten: bool,
    texel: Texel,
}

/// The textures of the planes of a picture of a shape, kept to be uploaded into again.
struct Planes {
    luma: wgpu::Texture,
    chroma: wgpu::Texture,
    /// What binds them, with the uniform of its generation.
    bind: Option<(u64, wgpu::BindGroup)>,
    /// The frame it was last drawn in.
    used: u64,
}

/// A text drawn into pixels, on the card.
struct TextTexture {
    view: wgpu::TextureView,
    width: u32,
    height: u32,
    block: (u32, u32),
    pad: u32,
    used: u64,
}

/// What a text drawn into pixels is known by: all that changes its pixels.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum TextKey {
    Title {
        text: String,
        font: String,
        size: u32,
        color: [u8; 4],
        align: TextAlign,
    },
    Subtitles {
        lines: Vec<String>,
        size: u32,
    },
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
    convert_layout: wgpu::BindGroupLayout,
    pipelines: HashMap<(Pipe, wgpu::TextureFormat), wgpu::RenderPipeline>,
    convert: Option<(wgpu::ComputePipeline, wgpu::ComputePipeline)>,
    sampler: wgpu::Sampler,
    /// A texel of white, for colours.
    white: wgpu::TextureView,
    /// The uniforms of the layers of a frame, one slot of `slot` bytes each.
    uniform: wgpu::Buffer,
    slot: u32,
    /// Changed when the uniform is made anew, so that what binds it is made anew too.
    generation: u64,
    planes: HashMap<Shape, Vec<Planes>>,
    texts: HashMap<TextKey, TextTexture>,
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
                    entries: &[uniform_entry(yuv::PARAMS_SIZE), plane(1), plane(2)],
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
            convert_layout,
            pipelines: HashMap::new(),
            convert: None,
            sampler,
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
        let laid = self.layers(scene, canvas, whole, &mut draws, &mut uniforms);
        if let Err(error) = laid {
            self.give_back(draws);
            return Err(error);
        }
        if target.export.is_none() && !scene.subtitles.is_empty() {
            let size = subtitle_size(scene.height);
            let key = TextKey::Subtitles {
                lines: scene.subtitles.clone(),
                size: size.to_bits(),
            };
            let subtitles = self.text(&key, || text::subtitles(&scene.subtitles, size));
            match subtitles {
                Ok(text) => {
                    let origin = subtitle_origin((scene.width, scene.height), text.block);
                    let quad = text_quad(origin, text);
                    let view = text.view.clone();
                    uniforms.push(layer_uniform(
                        &placement(canvas, canvas, &Transform::NONE),
                        canvas,
                        quad,
                        1.0,
                        1.0,
                        Some([1.0; 4]),
                    ));
                    draws.push(Draw::Texture(view));
                }
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
        for draw in &draws {
            let pipe = match draw {
                Draw::Picture { shape, .. } => Pipe::Yuv(shape.texel),
                Draw::Texture(_) => Pipe::Rgba,
            };
            self.pipeline(pipe, format);
        }
        let mut binds = Vec::with_capacity(draws.len());
        for draw in &mut draws {
            binds.push(match draw {
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
            });
        }

        let device = self.gpu.device();
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("columbina compose"),
        });
        let canvas_view = target
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("columbina layers"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &canvas_view,
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
            for ((pipe, bind), offset) in binds.iter().zip(offsets) {
                pass.set_pipeline(&self.pipelines[&(*pipe, format)]);
                pass.set_bind_group(0, bind, &[offset]);
                pass.draw(0..6, 0..1);
            }
        }
        if let Some(export) = &target.export {
            self.convert(&mut encoder, &canvas_view, export, target.height);
        }
        let submitted = self.gpu.queue().submit([encoder.finish()]);
        *target
            .submitted
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(submitted);
        self.give_back(draws);
        self.forget_unused();
        Ok(())
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
            match &layer.source {
                Source::Picture(picture, rotation) => {
                    let ten = yuv::check(picture).map_err(failed)?;
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
                        ten,
                        texel: Texel::for_picture(ten, self.gpu.has_sixteen_norm()),
                    };
                    let planes = self.take_planes(shape);
                    let uploaded =
                        yuv::upload(self.gpu.queue(), &planes.luma, &planes.chroma, picture);
                    draws.push(Draw::Picture { shape, planes });
                    uploaded.map_err(failed)?;
                    let mut values = yuv::picture_params(
                        [0.0; 4],
                        (picture.width, picture.height),
                        ten,
                        shape.texel,
                        *rotation,
                    )
                    .to_vec();
                    values.extend(layer_uniform(
                        &placement(canvas, frame, &layer.transform),
                        canvas,
                        [0.0, 0.0, frame.0, frame.1],
                        opacity,
                        scale,
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
                        Some(rgba.map(|channel| channel.clamp(0.0, 1.0))),
                    ));
                    draws.push(Draw::Texture(self.white.clone()));
                }
                Source::Title(title) => {
                    let style = &title.style;
                    let key = TextKey::Title {
                        text: title.text.clone(),
                        font: style.font.clone(),
                        size: style.size,
                        color: style.color.0,
                        align: style.align,
                    };
                    let text = self.text(&key, || text::title(title)).map_err(failed)?;
                    let origin = title_origin(
                        (scene.width, scene.height),
                        text.block,
                        (style.x, style.y),
                        style.align,
                    );
                    let quad = text_quad(origin, text);
                    let view = text.view.clone();
                    uniforms.push(layer_uniform(
                        &placement(canvas, canvas, &layer.transform),
                        canvas,
                        quad,
                        opacity,
                        scale,
                        Some([1.0; 4]),
                    ));
                    draws.push(Draw::Texture(view));
                }
            }
        }
        Ok(())
    }

    /// Planes for a picture of `shape`: some of those kept, or new ones.
    fn take_planes(&mut self, shape: Shape) -> Planes {
        let kept = self.planes.get_mut(&shape).and_then(Vec::pop);
        let mut planes = kept.unwrap_or_else(|| {
            let (luma, chroma) = yuv::plane_textures(
                self.gpu.device(),
                shape.width,
                shape.height,
                shape.ten,
                shape.texel,
            );
            Planes {
                luma,
                chroma,
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

    /// A text drawn into pixels and onto the card: the one kept for `key`, or one drawn with
    /// `draw` and kept, in place of the one drawn longest ago if there are many.
    fn text(
        &mut self,
        key: &TextKey,
        draw: impl FnOnce() -> Raster,
    ) -> Result<&TextTexture, String> {
        let frame = self.frame;
        if !self.texts.contains_key(key) {
            let raster = draw();
            let most = self.gpu.device().limits().max_texture_dimension_2d;
            if raster.width > most || raster.height > most {
                return Err(format!(
                    "a text of {}×{} pixels is larger than the {most}×{most} that the card takes",
                    raster.width, raster.height
                ));
            }
            if self.texts.len() >= TEXTS_KEPT {
                let oldest = self
                    .texts
                    .iter()
                    .min_by_key(|(_, text)| text.used)
                    .map(|(key, _)| key.clone());
                if let Some(oldest) = oldest {
                    self.texts.remove(&oldest);
                }
            }
            let texture = self.text_texture(&raster);
            self.texts.insert(key.clone(), texture);
        }
        let text = self.texts.get_mut(key).expect("kept above");
        text.used = frame;
        Ok(text)
    }

    /// Put the pixels of a text on the card.
    fn text_texture(&self, raster: &Raster) -> TextTexture {
        let texture = self.gpu.device().create_texture(&wgpu::TextureDescriptor {
            label: Some("columbina text"),
            size: wgpu::Extent3d {
                width: raster.width,
                height: raster.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.gpu.queue().write_texture(
            texture.as_image_copy(),
            &raster.pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(raster.width * 4),
                rows_per_image: Some(raster.height),
            },
            texture.size(),
        );
        TextTexture {
            view: texture.create_view(&wgpu::TextureViewDescriptor::default()),
            width: raster.width,
            height: raster.height,
            block: raster.block,
            pad: raster.pad,
            used: self.frame,
        }
    }

    /// The pipeline of `pipe` that draws into targets of `format`, made if it is not yet.
    fn pipeline(&mut self, pipe: Pipe, format: wgpu::TextureFormat) {
        if self.pipelines.contains_key(&(pipe, format)) {
            return;
        }
        let device = self.gpu.device();
        let (source, layout) = match pipe {
            Pipe::Yuv(texel) => (texel.shader(), &self.yuv_layouts[&texel]),
            Pipe::Rgba => (LAYER_SHADER.to_owned(), &self.rgba_layout),
        };
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("columbina layer"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("columbina layer"),
            bind_group_layouts: &[Some(layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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
        });
        self.pipelines.insert((pipe, format), pipeline);
    }

    /// What binds the planes of a picture, and the uniform.
    fn planes_bind(&self, texel: Texel, planes: &Planes) -> wgpu::BindGroup {
        self.gpu
            .device()
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("columbina picture"),
                layout: &self.yuv_layouts[&texel],
                entries: &[
                    self.uniform_entry(yuv::PARAMS_SIZE),
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(
                            &planes
                                .luma
                                .create_view(&wgpu::TextureViewDescriptor::default()),
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(
                            &planes
                                .chroma
                                .create_view(&wgpu::TextureViewDescriptor::default()),
                        ),
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

/// A buffer of uniforms of `size` bytes.
fn uniform_buffer(device: &wgpu::Device, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("columbina layers"),
        size,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// The quad of the frame (the canvas) that a text drawn into pixels covers, its block at
/// `origin`.
fn text_quad(origin: (f64, f64), text: &TextTexture) -> [f64; 4] {
    let pad = f64::from(text.pad);
    let (x, y) = (origin.0 - pad, origin.1 - pad);
    [x, y, x + f64::from(text.width), y + f64::from(text.height)]
}

/// The uniform of a layer that puts the quad `quad` of its frame on a canvas by `place`, at
/// `opacity`, the frame made `scale` times larger: the second half of the uniform of
/// `yuv.wgsl`, or, with a `tint`, the uniform of `layer.wgsl`.
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
    tint: Option<[f32; 4]>,
) -> Vec<f32> {
    let mut values: Vec<f32> = [
        place.a, place.b, place.tx, canvas.0, place.c, place.d, place.ty, canvas.1, quad[0],
        quad[1], quad[2], quad[3],
    ]
    .iter()
    .map(|value| *value as f32)
    .collect();
    values.extend([opacity, scale as f32, 0.0, 0.0]);
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
            None,
        );
        assert_eq!(
            values,
            [
                1.0, 0.0, 50.0, 200.0, 0.0, 1.0, 0.0, 100.0, 0.0, 0.0, 100.0, 100.0, 0.5, 1.0, 0.0,
                0.0
            ]
        );
        let tinted = layer_uniform(&place, (200.0, 100.0), [0.0; 4], 1.0, 2.0, Some([0.1; 4]));
        assert_eq!(u64::try_from(tinted.len() * 4).unwrap(), LAYER_PARAMS_SIZE);
        assert_eq!(
            u64::try_from(values.len() * 4 + 64).unwrap(),
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
