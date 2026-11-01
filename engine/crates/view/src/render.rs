//! The picture drawn on the graphics card: its planes of Y'CbCr uploaded as they are, turned
//! into R'G'B' as they are drawn (`yuv.wgsl` of `columbina-compose`, which composes the layers
//! of a timeline with the same shader), and fitted into the target with black around it.
//!
//! Nothing here knows of windows: [`Painter`] draws into any texture of the format it was made
//! for, a window's or one that the tests read back.
//!
//! Where the picture is mapped for the display ([`Mapping`]: SDR into an HDR surface, or HDR
//! content), it is drawn as above into a texture of half floats of the size of the target, and
//! that is mapped onto the target by `display.wgsl`. Where it is not (SDR into SDR), it is drawn
//! straight into the target, as it always was.

use std::cell::RefCell;

use columbina_compose::yuv::{self, Layout, Texel};
use columbina_compose::YuvCoding;
use columbina_media::NativePicture;

use crate::display::{self, Mapping};
use crate::{Viewport, ViewportMap};

pub use columbina_compose::quarter_turns;

/// Where a picture is drawn in a target: in pixels, from the top left corner of the target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Where a picture of `picture` pixels as it is stored, turned `rotation` degrees clockwise to
/// be shown, is drawn in a target of `target` pixels: as large as it fits, its shape kept,
/// centred, with black bars on the two sides that it does not reach. Pixels are taken as square.
/// None where the target or the picture has no pixels: then nothing is drawn.
#[must_use]
pub fn letterbox(target: (u32, u32), picture: (u32, u32), rotation: i16) -> Option<Placement> {
    let (target_width, target_height) = target;
    let (mut width, mut height) = picture;
    if quarter_turns(rotation) % 2 == 1 {
        std::mem::swap(&mut width, &mut height);
    }
    if target_width == 0 || target_height == 0 || width == 0 || height == 0 {
        return None;
    }
    let (tw, th) = (u128::from(target_width), u128::from(target_height));
    let (pw, ph) = (u128::from(width), u128::from(height));
    // a / b to the nearest whole number, halves up; never below 1, never beyond the target.
    let rounded = |a: u128, b: u128, most: u32| {
        u32::try_from((2 * a + b) / (2 * b)).map_or(most, |value| value.clamp(1, most))
    };
    let (width, height) = if tw * ph <= th * pw {
        // As wide as the target, or the picture is of its shape.
        (target_width, rounded(tw * ph, pw, target_height))
    } else {
        (rounded(th * pw, ph, target_width), target_height)
    };
    Some(Placement {
        x: (target_width - width) / 2,
        y: (target_height - height) / 2,
        width,
        height,
    })
}

/// A pipeline for planes of one kind of texel.
struct Pipeline {
    layout: wgpu::BindGroupLayout,
    draw: wgpu::RenderPipeline,
    viewport: Option<wgpu::RenderPipeline>,
}

/// The textures of the planes of the last picture, kept for the next one of the same shape, and
/// how that picture is read: its layout and its coding.
struct Planes {
    layout: Layout,
    coding: YuvCoding,
    texel: Texel,
    textures: yuv::PlaneTextures,
    bind: wgpu::BindGroup,
}

/// An already composed frame, still on the shared graphics device.
struct RgbFrame {
    width: u32,
    height: u32,
    bind: wgpu::BindGroup,
}

/// The pass that maps the picture onto the target, and the texture that the picture is drawn
/// in first, of the size of the target.
struct DisplayPass {
    pipeline: Pipeline,
    uniform: wgpu::Buffer,
    drawn: Option<Drawn>,
}

/// A texture that the picture is drawn in before it is mapped, and what binds it.
struct Drawn {
    size: (u32, u32),
    view: wgpu::TextureView,
    bind: wgpu::BindGroup,
}

impl DisplayPass {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        Self {
            pipeline: display_pipeline(device, format),
            uniform: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("columbina-view mapping"),
                size: display::PARAMS_SIZE,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            drawn: None,
        }
    }

    /// Make the texture to draw the picture in of `size`, again where its size changes.
    fn prepare(&mut self, device: &wgpu::Device, size: (u32, u32)) {
        let size = (size.0.max(1), size.1.max(1));
        if self.drawn.as_ref().is_none_or(|drawn| drawn.size != size) {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("columbina-view drawn"),
                size: wgpu::Extent3d {
                    width: size.0,
                    height: size.1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: MAPPED_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("columbina-view drawn"),
                layout: &self.pipeline.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: self.uniform.as_entire_binding(),
                    },
                ],
            });
            self.drawn = Some(Drawn { size, view, bind });
        }
    }
}

/// The format that a picture to be mapped is drawn in first.
const MAPPED_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// What draws pictures into targets of one format, on one device.
pub(crate) struct Painter {
    device: wgpu::Device,
    queue: wgpu::Queue,
    /// The format of the targets.
    format: wgpu::TextureFormat,
    /// The format that the picture is drawn in by the pipelines below: the targets', or
    /// [`MAPPED_FORMAT`] where it is mapped.
    stage: wgpu::TextureFormat,
    /// How the picture is mapped onto the target, if it is.
    mapping: Option<Mapping>,
    /// Made when the picture is first mapped. In a cell, as [`Painter::draw`] takes the painter
    /// as it is, and makes the texture of the size of its target there.
    display: RefCell<Option<DisplayPass>>,
    /// Whether the device has `R16Unorm` and `Rg16Unorm`, for pictures of words of 16 bits.
    sixteen_norm: bool,
    uniform: wgpu::Buffer,
    float: Pipeline,
    /// Made when a picture of words of 16 bits is first drawn without 16-bit normalized formats.
    uint: Option<Pipeline>,
    planes: Option<Planes>,
    rgb_pipeline: Option<Pipeline>,
    rgb_frame: Option<RgbFrame>,
    rgb_sampler: Option<wgpu::Sampler>,
    viewport_uniform: wgpu::Buffer,
}

impl Painter {
    /// A painter for targets of `format`, which is not sRGB: the picture is gamma-encoded
    /// already, and is written as it is.
    pub(crate) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
    ) -> Self {
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("columbina-view params"),
            size: yuv::PARAMS_SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            viewport_uniform: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("columbina-view viewport"),
                size: 16,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            sixteen_norm: device
                .features()
                .contains(wgpu::Features::TEXTURE_FORMAT_16BIT_NORM),
            float: pipeline(device, format, Texel::Float),
            uint: None,
            device: device.clone(),
            queue: queue.clone(),
            format,
            stage: format,
            mapping: None,
            display: RefCell::new(None),
            uniform,
            planes: None,
            rgb_pipeline: None,
            rgb_frame: None,
            rgb_sampler: None,
        }
    }

    /// Upload the planes of a picture, into the textures of the last one where it is of the
    /// same shape (sizes, format and plane of alpha), to be drawn by the matrix, range, siting
    /// of chroma and alpha that it says (over black, where it has alpha).
    ///
    /// # Errors
    /// The picture is not one that can be drawn: no pixels, planes shorter than it says, or
    /// larger than the card takes. Nothing is uploaded, and the last picture is kept.
    pub(crate) fn upload(&mut self, picture: &NativePicture) -> Result<(), String> {
        let layout = yuv::check(picture)?;
        let most = self.device.limits().max_texture_dimension_2d;
        if picture.width > most || picture.height > most {
            return Err(format!(
                "a picture of {}×{} is larger than the {most}×{most} that the card takes",
                picture.width, picture.height
            ));
        }
        let texel = layout.texel(self.sixteen_norm);
        let reuse = self.planes.as_ref().is_some_and(|planes| {
            let kept = planes.layout;
            (
                kept.width,
                kept.height,
                kept.format,
                kept.alpha_plane(),
                planes.texel,
            ) == (
                layout.width,
                layout.height,
                layout.format,
                layout.alpha_plane(),
                texel,
            )
        });
        if !reuse {
            self.planes = Some(self.planes_for(layout, texel));
        }
        let planes = self.planes.as_mut().expect("made above");
        planes.layout = layout;
        planes.coding = YuvCoding::of_native(picture);
        yuv::upload(&self.queue, &planes.textures, picture)?;
        self.rgb_frame = None;
        Ok(())
    }

    /// Bind a composed RGBA frame on this same device, without a CPU readback or YUV roundtrip:
    /// one of half floats, as the compositor makes them ([`columbina_compose::WORKING_FORMAT`]),
    /// or of 8 bits. It is drawn into the format of the target, values beyond 0..1 clamped.
    pub(crate) fn use_texture(&mut self, texture: &wgpu::Texture) -> Result<(), String> {
        if !matches!(
            texture.format(),
            wgpu::TextureFormat::Rgba16Float | wgpu::TextureFormat::Rgba8Unorm
        ) || texture.dimension() != wgpu::TextureDimension::D2
            || texture.depth_or_array_layers() != 1
            || texture.sample_count() != 1
            || !texture
                .usage()
                .contains(wgpu::TextureUsages::TEXTURE_BINDING)
        {
            return Err(
                "a composed frame must be a sampleable 2D Rgba16Float or Rgba8Unorm texture".into(),
            );
        }
        let pipeline = self
            .rgb_pipeline
            .get_or_insert_with(|| rgb_pipeline(&self.device, self.stage));
        let sampler = self.rgb_sampler.get_or_insert_with(|| {
            self.device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("columbina-view composed sampler"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            })
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("columbina-view composed frame"),
            layout: &pipeline.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.viewport_uniform.as_entire_binding(),
                },
            ],
        });
        self.rgb_frame = Some(RgbFrame {
            width: texture.width(),
            height: texture.height(),
            bind,
        });
        Ok(())
    }

    /// Textures, and what binds them, for planes of a shape.
    fn planes_for(&mut self, layout: Layout, texel: Texel) -> Planes {
        let textures = yuv::plane_textures(
            &self.device,
            layout.width,
            layout.height,
            layout.format,
            layout.alpha_plane(),
            texel,
        );
        let [luma, chroma, alpha] = textures.views();
        let pipeline = match texel {
            Texel::Float => &self.float,
            Texel::Uint => self
                .uint
                .get_or_insert_with(|| pipeline(&self.device, self.stage, Texel::Uint)),
        };
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("columbina-view planes"),
            layout: &pipeline.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniform.as_entire_binding(),
                },
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
        });
        Planes {
            layout,
            coding: YuvCoding::default(),
            texel,
            textures,
            bind,
        }
    }

    /// Forget the picture that was uploaded: what is drawn next is black, until another is.
    pub(crate) fn forget(&mut self) {
        self.planes = None;
        self.rgb_frame = None;
    }

    /// Map what is drawn next onto the target as `mapping` says, or draw it straight into the
    /// target (none). Whether the picture was forgotten, as its pipelines changed, and must be
    /// uploaded again.
    pub(crate) fn set_mapping(&mut self, mapping: Option<Mapping>) -> bool {
        self.mapping = mapping;
        let stage = if mapping.is_some() {
            MAPPED_FORMAT
        } else {
            self.format
        };
        if stage == self.stage {
            return false;
        }
        self.stage = stage;
        self.float = pipeline(&self.device, stage, Texel::Float);
        self.uint = None;
        self.rgb_pipeline = None;
        self.forget();
        true
    }

    /// Draw into `target`, of `size` pixels: black, and the picture that was uploaded last, if
    /// any, fitted into it and turned `rotation` degrees clockwise. Gives the commands, to be
    /// submitted.
    #[cfg(test)]
    pub(crate) fn draw(
        &self,
        target: &wgpu::TextureView,
        size: (u32, u32),
        rotation: i16,
    ) -> wgpu::CommandBuffer {
        self.draw_mapped(
            target,
            size,
            rotation,
            self.viewport_map(size, rotation, Viewport::Fit),
        )
    }

    pub(crate) fn viewport_map(
        &self,
        size: (u32, u32),
        rotation: i16,
        viewport: Viewport,
    ) -> Option<ViewportMap> {
        if let Some(rgb) = &self.rgb_frame {
            viewport.map(size, (rgb.width, rgb.height), 0)
        } else {
            let planes = self.planes.as_ref()?;
            viewport.map(size, (planes.layout.width, planes.layout.height), rotation)
        }
    }

    pub(crate) fn draw_mapped(
        &self,
        target: &wgpu::TextureView,
        size: (u32, u32),
        rotation: i16,
        map: Option<ViewportMap>,
    ) -> wgpu::CommandBuffer {
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("columbina-view"),
            });
        let mut display = self.display.borrow_mut();
        let mapped = self.mapping.map(|mapping| {
            let pass = display.get_or_insert_with(|| DisplayPass::new(&self.device, self.format));
            let params: Vec<u8> = mapping
                .params()
                .iter()
                .flat_map(|value| value.to_le_bytes())
                .collect();
            self.queue.write_buffer(&pass.uniform, 0, &params);
            pass.prepare(&self.device, size);
            let drawn = pass.drawn.as_ref().expect("made by prepare");
            (&pass.pipeline.draw, &drawn.view, &drawn.bind)
        });
        let drawn = self.picture_bind(map, rotation);
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("columbina-view"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: mapped.map_or(target, |(_, view, _)| view),
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
            if let Some((bind, pipeline, map)) = drawn {
                if let Some(placement) = map.fit {
                    #[allow(clippy::cast_precision_loss, reason = "sizes of at most 16384")]
                    pass.set_viewport(
                        placement.x as f32,
                        placement.y as f32,
                        placement.width as f32,
                        placement.height as f32,
                        0.0,
                        1.0,
                    );
                }
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, bind, &[]);
                pass.draw(0..3, 0..1);
            }
        }
        if let Some((pipeline, _, bind)) = mapped {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("columbina-view mapping"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
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
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, bind, &[]);
            pass.draw(0..3, 0..1);
        }
        encoder.finish()
    }

    fn picture_bind(
        &self,
        map: Option<ViewportMap>,
        rotation: i16,
    ) -> Option<(&wgpu::BindGroup, &wgpu::RenderPipeline, ViewportMap)> {
        if let Some(rgb) = &self.rgb_frame {
            map.map(|map| {
                let params: Vec<u8> = map
                    .dest
                    .iter()
                    .flat_map(|value| value.to_le_bytes())
                    .collect();
                self.queue.write_buffer(&self.viewport_uniform, 0, &params);
                (
                    &rgb.bind,
                    selected(
                        self.rgb_pipeline.as_ref().expect("made with the frame"),
                        map,
                    ),
                    map,
                )
            })
        } else {
            self.planes.as_ref().and_then(|planes| {
                map.map(|map| {
                    self.queue
                        .write_buffer(&self.uniform, 0, &params(planes, map.dest, rotation));
                    let pipeline = match planes.texel {
                        Texel::Float => &self.float,
                        Texel::Uint => self.uint.as_ref().expect("made with the planes"),
                    };
                    (&planes.bind, selected(pipeline, map), map)
                })
            })
        }
    }

    /// The queue that the commands of [`Painter::draw_mapped`] are submitted to.
    pub(crate) fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }
}

/// The uniform of the shader (`Params` in `yuv.wgsl`), as bytes: the half of it that the view
/// uses, and nothing in the half that the compositor uses.
#[allow(clippy::cast_precision_loss, reason = "sizes of at most 16384")]
fn params(planes: &Planes, dest: [f32; 4], rotation: i16) -> Vec<u8> {
    // A picture shown by itself (the source monitor) is decoded by the matrix and range that it
    // says, as the compositor decodes it as a layer (the timeline monitor, an export).
    let values = yuv::picture_params(dest, planes.layout, planes.texel, planes.coding, rotation);
    let mut bytes: Vec<u8> = values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect();
    bytes.resize(
        usize::try_from(yuv::PARAMS_SIZE).expect("a small uniform"),
        0,
    );
    bytes
}

/// The pipeline for planes of a kind of texel, drawing into targets of `format`.
fn pipeline(device: &wgpu::Device, format: wgpu::TextureFormat, texel: Texel) -> Pipeline {
    yuv_pipeline(device, format, texel, true)
}

fn selected(pipeline: &Pipeline, map: ViewportMap) -> &wgpu::RenderPipeline {
    if map.fit.is_some() {
        &pipeline.draw
    } else {
        pipeline.viewport.as_ref().expect("viewport pipeline")
    }
}

fn yuv_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    texel: Texel,
    viewport: bool,
) -> Pipeline {
    let source = if viewport {
        format!("{}\n{}", texel.shader(), include_str!("yuv_viewport.wgsl"))
    } else {
        texel.shader()
    };
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("columbina-view yuv"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
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
        label: Some("columbina-view planes"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(yuv::PARAMS_SIZE),
                },
                count: None,
            },
            plane(1),
            plane(2),
            plane(3),
        ],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("columbina-view"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let make = |entry_point| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("columbina-view"),
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
                entry_point: Some(entry_point),
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
    };
    let pipeline = make("fs");
    let viewport = viewport.then(|| make("fs_viewport"));
    Pipeline {
        layout,
        draw: pipeline,
        viewport,
    }
}

fn rgb_pipeline(device: &wgpu::Device, format: wgpu::TextureFormat) -> Pipeline {
    rgb_pipeline_source(device, format, include_str!("rgb.wgsl"), true)
}

fn rgb_pipeline_source(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    source: &str,
    viewport: bool,
) -> Pipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("columbina-view composed frame"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("columbina-view composed frame"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(16),
                },
                count: None,
            },
        ],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("columbina-view composed frame"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let make = |entry_point| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("columbina-view composed frame"),
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
                entry_point: Some(entry_point),
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
    };
    let pipeline = make("fs");
    let viewport = viewport.then(|| make("fs_viewport"));
    Pipeline {
        layout,
        draw: pipeline,
        viewport,
    }
}

/// The pipeline that maps the picture drawn in [`MAPPED_FORMAT`] onto targets of `format`.
fn display_pipeline(device: &wgpu::Device, format: wgpu::TextureFormat) -> Pipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("columbina-view mapping"),
        source: wgpu::ShaderSource::Wgsl(include_str!("display.wgsl").into()),
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("columbina-view mapping"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(display::PARAMS_SIZE),
                },
                count: None,
            },
        ],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("columbina-view mapping"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("columbina-view mapping"),
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
    });
    Pipeline {
        layout,
        draw: pipeline,
        viewport: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_matches_the_original_shaders_and_draw_path_pixel_for_pixel() {
        let Some(gpu) = gpu() else { return };
        for mapped in [false, true] {
            let mut before = Painter::new(&gpu.device, &gpu.queue, wgpu::TextureFormat::Bgra8Unorm);
            let mut now = Painter::new(&gpu.device, &gpu.queue, wgpu::TextureFormat::Bgra8Unorm);
            if mapped {
                let mapping = mapping(
                    Content::Hdr {
                        transfer: HdrTransfer::Pq,
                        peak_nits: 1000.0,
                    },
                    Output::Sdr,
                );
                before.set_mapping(mapping);
                now.set_mapping(mapping);
            }
            before.float = yuv_pipeline(&gpu.device, before.stage, Texel::Float, false);
            for ten in [false, true] {
                let picture = picture(64, 32, ten, |x, y| {
                    codes(BARS[((x / 8 + y / 8) % 8) as usize], ten)
                });
                before.upload(&picture).unwrap();
                now.upload(&picture).unwrap();
                for (size, rotation) in [
                    ((97, 67), 0),
                    ((37, 89), 90),
                    ((99, 71), 180),
                    ((31, 65), 270),
                ] {
                    let old = draw_using(&gpu, size, |target| {
                        before.legacy_draw(target, size, rotation)
                    });
                    assert_eq!(
                        draw(&now, &gpu, size, rotation),
                        old,
                        "YUV {ten} {mapped} {size:?} {rotation}"
                    );
                }
            }
            let values: Vec<[f64; 3]> = (0..32)
                .map(|x| [f64::from(x) / 31.0, 0.25, 1.0 - f64::from(x) / 31.0])
                .collect();
            let (texture, _) = halves(&gpu, &values);
            before.rgb_pipeline = Some(rgb_pipeline_source(
                &gpu.device,
                before.stage,
                include_str!("render/legacy_rgb.wgsl"),
                false,
            ));
            before.use_texture(&texture).unwrap();
            now.use_texture(&texture).unwrap();
            for size in [(53, 17), (31, 65)] {
                assert_eq!(
                    draw(&now, &gpu, size, 0),
                    draw_using(&gpu, size, |target| before.legacy_draw(target, size, 0)),
                    "RGB {mapped} {size:?}"
                );
            }
        }
    }

    #[test]
    fn custom_viewports_clip_rotated_yuv_to_the_pointer_map() {
        let Some(gpu) = gpu() else { return };
        let size = (32, 24);
        let picture = picture(16, 8, false, |_, _| [235, 128, 128]);
        let mut painter = Painter::new(&gpu.device, &gpu.queue, wgpu::TextureFormat::Bgra8Unorm);
        for rotation in [0, 90, 180, 270] {
            painter.upload(&picture).unwrap();
            for viewport in [
                Viewport::Scale {
                    scale: 4.0,
                    center: [0.5, 0.5],
                },
                Viewport::Scale {
                    scale: 1.25,
                    center: [0.0, 0.5],
                },
                Viewport::Scale {
                    scale: 0.5,
                    center: [0.5, 0.5],
                },
                Viewport::Scale {
                    scale: 2.0,
                    center: [-4.0, 3.0],
                },
            ] {
                let map = painter.viewport_map(size, rotation, viewport).unwrap();
                let pixels = draw_using(&gpu, size, |target| {
                    painter.draw_mapped(target, size, rotation, Some(map))
                });
                for y in 0..size.1 {
                    for x in 0..size.0 {
                        let inside = map
                            .on_picture((i32::try_from(x).unwrap(), i32::try_from(y).unwrap()))
                            .1;
                        assert_eq!(
                            pixels[(y * size.0 + x) as usize],
                            if inside { [255; 3] } else { [0; 3] },
                            "{rotation} {viewport:?} ({x},{y})"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn negative_viewports_sample_oriented_pixels_and_keep_the_display_background_black() {
        let Some(gpu) = gpu() else { return };
        let mut painter = Painter::new(&gpu.device, &gpu.queue, wgpu::TextureFormat::Bgra8Unorm);
        // A cropped ramp checks which oriented pixels were sampled, not only the outline.
        let ramp = picture(16, 8, false, |x, y| {
            [16 + u16::try_from(8 * x + 6 * y).unwrap(), 128, 128]
        });
        painter.upload(&ramp).unwrap();
        for rotation in [0, 90, 180, 270] {
            let oriented = if rotation % 180 == 0 {
                (16, 8)
            } else {
                (8, 16)
            };
            let baseline = draw(&painter, &gpu, oriented, rotation);
            let size = (8, 6);
            let map = painter
                .viewport_map(
                    size,
                    rotation,
                    Viewport::Scale {
                        scale: 1.0,
                        center: [0.5, 0.5],
                    },
                )
                .unwrap();
            let cropped = draw_using(&gpu, size, |target| {
                painter.draw_mapped(target, size, rotation, Some(map))
            });
            for y in 0..size.1 {
                for x in 0..size.0 {
                    let source_x = x + (oriented.0 - size.0) / 2;
                    let source_y = y + (oriented.1 - size.1) / 2;
                    assert_eq!(
                        cropped[(y * size.0 + x) as usize],
                        baseline[(source_y * oriented.0 + source_x) as usize],
                        "cropped {rotation} ({x},{y})"
                    );
                }
            }
        }
        // With 100% zoom and negative destination, source pixel centres must be preserved.
        let values: Vec<[f64; 3]> = (0..16).map(|x| [f64::from(x) / 15.0, 0.0, 0.0]).collect();
        let (texture, held) = halves(&gpu, &values);
        painter.use_texture(&texture).unwrap();
        let size = (8, 3);
        let map = painter
            .viewport_map(
                size,
                0,
                Viewport::Scale {
                    scale: 1.0,
                    center: [0.5, 0.5],
                },
            )
            .unwrap();
        assert_eq!(map.dest, [-4.0, 1.0, 16.0, 1.0]);
        let pixels = draw_using(&gpu, size, |target| {
            painter.draw_mapped(target, size, 0, Some(map))
        });
        for (index, pixel) in pixels.iter().enumerate() {
            if index / 8 == 1 {
                assert!((f32::from(pixel[0]) - held[index % 8 + 4][0] * 255.0).abs() <= 0.51);
            } else {
                assert_eq!(*pixel, [0; 3]);
            }
        }
        let mut hdr = Painter::new(&gpu.device, &gpu.queue, wgpu::TextureFormat::Rgba16Float);
        hdr.set_mapping(mapping(
            Content::Sdr,
            Output::Hdr {
                peak_nits: 400.0,
                sdr_white_nits: 160.0,
            },
        ));
        hdr.use_texture(&texture).unwrap();
        let pixels = halves_using(&gpu, size, |target| {
            hdr.draw_mapped(target, size, 0, Some(map))
        });
        for pixel in pixels[..8].iter().chain(&pixels[16..]) {
            assert_eq!(*pixel, [0.0; 3]);
        }
        assert!(pixels[12][0] > 0.0);
    }

    #[test]
    fn the_smallest_normal_scale_keeps_rgb_and_yuv_outside_pixels_black() {
        let Some(gpu) = gpu() else { return };
        let size = (64, 32);
        let viewport = Viewport::Scale {
            scale: f64::from(f32::MIN_POSITIVE),
            center: [0.5, 0.5],
        };
        assert!(viewport.checked().is_ok());
        let map = viewport.map(size, (1, 1), 0).unwrap();
        // The former divide-then-clip expression overflows for this valid map. WGSL does
        // not define the overflow result; a driver may still happen to produce black.
        let old_uv = (0.5_f32 - map.dest[0]) / map.dest[2];
        assert!(!old_uv.is_finite());
        let mut painter = Painter::new(&gpu.device, &gpu.queue, wgpu::TextureFormat::Bgra8Unorm);
        painter
            .upload(&picture(1, 1, false, |_, _| [235, 128, 128]))
            .unwrap();
        let yuv = draw_using(&gpu, size, |target| {
            painter.draw_mapped(target, size, 0, Some(map))
        });
        assert!(yuv.iter().all(|pixel| *pixel == [0; 3]));
        let (texture, _) = halves(&gpu, &[[1.0; 3]]);
        painter.use_texture(&texture).unwrap();
        let rgb = draw_using(&gpu, size, |target| {
            painter.draw_mapped(target, size, 0, Some(map))
        });
        assert!(rgb.iter().all(|pixel| *pixel == [0; 3]));
    }
    use columbina_media::PictureFormat;
    use std::sync::Arc;

    #[test]
    fn a_picture_fills_a_target_of_its_shape() {
        let placement = letterbox((1280, 720), (1920, 1080), 0);
        assert_eq!(
            placement,
            Some(Placement {
                x: 0,
                y: 0,
                width: 1280,
                height: 720
            })
        );
    }

    #[test]
    fn a_picture_wider_than_its_target_has_bars_above_and_below() {
        // 1000 × 1080 / 1920 = 562.5, rounded up.
        assert_eq!(
            letterbox((1000, 1000), (1920, 1080), 0),
            Some(Placement {
                x: 0,
                y: 218,
                width: 1000,
                height: 563
            })
        );
    }

    #[test]
    fn a_picture_narrower_than_its_target_has_bars_on_its_sides() {
        assert_eq!(
            letterbox((1920, 1080), (640, 480), 0),
            Some(Placement {
                x: 240,
                y: 0,
                width: 1440,
                height: 1080
            })
        );
        // 180 degrees changes nothing of where it is.
        assert_eq!(
            letterbox((1920, 1080), (640, 480), 180),
            letterbox((1920, 1080), (640, 480), 0)
        );
    }

    #[test]
    fn a_quarter_turn_swaps_the_sides_of_a_picture() {
        for rotation in [90, 270] {
            assert_eq!(
                letterbox((1000, 1000), (1920, 1080), rotation),
                Some(Placement {
                    x: 218,
                    y: 0,
                    width: 563,
                    height: 1000
                }),
                "{rotation}"
            );
        }
        // A portrait phone video, stored landscape and turned, fills a portrait target.
        assert_eq!(
            letterbox((1080, 1920), (1920, 1080), 90),
            Some(Placement {
                x: 0,
                y: 0,
                width: 1080,
                height: 1920
            })
        );
    }

    #[test]
    fn odd_sizes_are_rounded_and_centred_and_never_lose_a_side() {
        assert_eq!(
            letterbox((2, 2), (3, 1), 0),
            Some(Placement {
                x: 0,
                y: 0,
                width: 2,
                height: 1
            })
        );
        assert_eq!(
            letterbox((101, 51), (3, 3), 0),
            Some(Placement {
                x: 25,
                y: 0,
                width: 51,
                height: 51
            })
        );
        // So thin that it would be less than a pixel: one pixel.
        assert_eq!(
            letterbox((100, 100), (10_000, 1), 0),
            Some(Placement {
                x: 0,
                y: 49,
                width: 100,
                height: 1
            })
        );
        assert_eq!(
            letterbox((16384, 16384), (u32::MAX, 1), 270),
            Some(Placement {
                x: 8191,
                y: 0,
                width: 1,
                height: 16384
            })
        );
    }

    #[test]
    fn nothing_is_drawn_where_there_are_no_pixels() {
        assert_eq!(letterbox((0, 0), (1920, 1080), 0), None);
        assert_eq!(letterbox((100, 0), (1920, 1080), 90), None);
        assert_eq!(letterbox((0, 100), (1920, 1080), 0), None);
        assert_eq!(letterbox((100, 100), (0, 1080), 0), None);
        assert_eq!(letterbox((100, 100), (1920, 0), 270), None);
    }

    #[test]
    fn rotations_are_read_as_quarter_turns() {
        assert_eq!(
            [0, 90, 180, 270, 360, 450, 45, 1000, -90, -180, -450, -1].map(quarter_turns),
            [0, 1, 2, 3, 0, 1, 0, 0, 3, 2, 3, 0]
        );
    }

    // What follows draws with the graphics card, into a texture that is read back. Without an
    // adapter or backend the tests are skipped, unless COLUMBINA_REQUIRE_GPU is 1.

    struct Gpu {
        device: wgpu::Device,
        queue: wgpu::Queue,
    }

    fn gpu() -> Option<Gpu> {
        let gpu = columbina_compose::gpu_for_tests()?;
        Some(Gpu {
            device: gpu.device().clone(),
            queue: gpu.queue().clone(),
        })
    }

    #[test]
    fn composed_rgb_is_letterboxed_without_another_colour_conversion() {
        let Some(gpu) = gpu() else { return };
        let mut painter = Painter::new(&gpu.device, &gpu.queue, wgpu::TextureFormat::Bgra8Unorm);
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: 2,
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
        gpu.queue.write_texture(
            texture.as_image_copy(),
            &[23, 81, 192, 255, 178, 45, 96, 255],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(8),
                rows_per_image: Some(1),
            },
            texture.size(),
        );
        painter
            .upload(&picture(2, 2, false, |_, _| [235, 128, 128]))
            .unwrap();
        painter.use_texture(&texture).unwrap();
        let pixels = draw(&painter, &gpu, (2, 3), 0);
        assert_eq!(
            pixels,
            [
                [0, 0, 0],
                [0, 0, 0],
                [23, 81, 192],
                [178, 45, 96],
                [0, 0, 0],
                [0, 0, 0]
            ]
        );
        painter.forget();
        assert!(draw(&painter, &gpu, (2, 3), 0)
            .iter()
            .all(|pixel| *pixel == [0, 0, 0]));
        painter
            .upload(&picture(2, 2, false, |_, _| [235, 128, 128]))
            .unwrap();
        assert!(draw(&painter, &gpu, (2, 2), 0)
            .iter()
            .all(|pixel| *pixel == [255, 255, 255]));
    }

    #[test]
    fn a_composed_frame_of_half_floats_is_drawn_into_8_bits_rounded_and_clamped() {
        let Some(gpu) = gpu() else { return };
        let mut painter = Painter::new(&gpu.device, &gpu.queue, wgpu::TextureFormat::Bgra8Unorm);
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: 2,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        // Half floats: 0.25, 0.375, 0.75, 1; and 0.125, 0, 2 (beyond white), 1. None is half way
        // between two codes, which a card rounds either way.
        let halves: [u16; 8] = [
            0x3400, 0x3600, 0x3A00, 0x3C00, 0x3000, 0x0000, 0x4000, 0x3C00,
        ];
        let bytes: Vec<u8> = halves.iter().flat_map(|half| half.to_le_bytes()).collect();
        gpu.queue.write_texture(
            texture.as_image_copy(),
            &bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(16),
                rows_per_image: Some(1),
            },
            texture.size(),
        );
        painter.use_texture(&texture).unwrap();
        let pixels = draw(&painter, &gpu, (2, 1), 0);
        assert_eq!(pixels, [[64, 96, 191], [32, 0, 255]]);
    }

    /// The pixels drawn into a target of `size`, as R'G'B', a row after another.
    fn draw(painter: &Painter, gpu: &Gpu, size: (u32, u32), rotation: i16) -> Vec<[u8; 3]> {
        draw_using(gpu, size, |target| painter.draw(target, size, rotation))
    }

    fn draw_using(
        gpu: &Gpu,
        size: (u32, u32),
        commands: impl FnOnce(&wgpu::TextureView) -> wgpu::CommandBuffer,
    ) -> Vec<[u8; 3]> {
        let (width, height) = size;
        let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Bgra8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let drawn = commands(&view);
        let row = (width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(row * height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        encoder.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(height),
                },
            },
            target.size(),
        );
        gpu.queue.submit([drawn, encoder.finish()]);
        let (mapped, mapping) = std::sync::mpsc::channel();
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                mapped.send(result).unwrap();
            });
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        mapping.recv().unwrap().unwrap();
        let bytes = buffer.slice(..).get_mapped_range().unwrap().to_vec();
        let mut pixels = Vec::new();
        for y in 0..height as usize {
            for x in 0..width as usize {
                let at = y * row as usize + x * 4;
                pixels.push([bytes[at + 2], bytes[at + 1], bytes[at]]);
            }
        }
        pixels
    }

    /// A picture whose pixel at (x, y) has the codes of Y', Cb and Cr that `pixel` gives; the
    /// chroma of each block of 2 × 2 is that of its top left pixel. Rows are longer than they
    /// need to be, as a decoder's are.
    fn picture(
        width: u32,
        height: u32,
        ten: bool,
        pixel: impl Fn(u32, u32) -> [u16; 3],
    ) -> NativePicture {
        let bytes = if ten { 2 } else { 1 };
        let stride = (width as usize * bytes).next_multiple_of(64) + 64;
        let rows = height as usize;
        let mut luma = vec![0xEE; stride * rows];
        let mut chroma = vec![0xEE; stride * rows.div_ceil(2)];
        let put = |plane: &mut Vec<u8>, at: usize, code: u16| {
            if ten {
                // P010: the 10 bits in the high bits of a little-endian word.
                plane[at..at + 2].copy_from_slice(&(code << 6).to_le_bytes());
            } else {
                plane[at] = u8::try_from(code).unwrap();
            }
        };
        for y in 0..height {
            for x in 0..width {
                let [luma_code, cb, cr] = pixel(x, y);
                put(
                    &mut luma,
                    y as usize * stride + x as usize * bytes,
                    luma_code,
                );
                if x % 2 == 0 && y % 2 == 0 {
                    let at = (y / 2) as usize * stride + (x / 2) as usize * 2 * bytes;
                    put(&mut chroma, at, cb);
                    put(&mut chroma, at + bytes, cr);
                }
            }
        }
        yuv::native_of(columbina_media::Picture {
            frame: 0,
            width,
            height,
            bits: if ten { 10 } else { 8 },
            luma,
            chroma,
            luma_stride: stride,
            chroma_stride: stride,
        })
    }

    /// R'G'B' from 0 to 1 of codes of BT.709 of limited range, as the shader should make it.
    fn rgb(codes: [u16; 3], ten: bool) -> [f64; 3] {
        let (black, luma_range, middle, chroma_range) = if ten {
            (64.0, 876.0, 512.0, 896.0)
        } else {
            (16.0, 219.0, 128.0, 224.0)
        };
        let [y, cb, cr] = codes.map(f64::from);
        let y = (y - black) / luma_range;
        let (cb, cr) = ((cb - middle) / chroma_range, (cr - middle) / chroma_range);
        let (kr, kb) = (0.2126, 0.0722);
        let kg = 1.0 - kr - kb;
        [
            y + 2.0 * (1.0 - kr) * cr,
            y - 2.0 * kb * (1.0 - kb) / kg * cb - 2.0 * kr * (1.0 - kr) / kg * cr,
            y + 2.0 * (1.0 - kb) * cb,
        ]
        .map(|value| value.clamp(0.0, 1.0))
    }

    /// The codes of BT.709 of limited range of R'G'B' from 0 to 1.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn codes(rgb: [f64; 3], ten: bool) -> [u16; 3] {
        let (kr, kb) = (0.2126, 0.0722);
        let [r, g, b] = rgb;
        let y = kr * r + (1.0 - kr - kb) * g + kb * b;
        let cb = (b - y) / (2.0 * (1.0 - kb));
        let cr = (r - y) / (2.0 * (1.0 - kr));
        let (black, luma_range, middle, chroma_range) = if ten {
            (64.0, 876.0, 512.0, 896.0)
        } else {
            (16.0, 219.0, 128.0, 224.0)
        };
        [
            black + luma_range * y,
            middle + chroma_range * cb,
            middle + chroma_range * cr,
        ]
        .map(|code: f64| code.round() as u16)
    }

    /// 75% colour bars: white, yellow, cyan, green, magenta, red, blue, black.
    const BARS: [[f64; 3]; 8] = [
        [0.75, 0.75, 0.75],
        [0.75, 0.75, 0.0],
        [0.0, 0.75, 0.75],
        [0.0, 0.75, 0.0],
        [0.75, 0.0, 0.75],
        [0.75, 0.0, 0.0],
        [0.0, 0.0, 0.75],
        [0.0, 0.0, 0.0],
    ];

    #[test]
    fn the_bars_are_coded_as_bt_709_codes_them() {
        // The 8-bit codes of 75% bars of BT.709, as test signal generators make them.
        assert_eq!(
            BARS.map(|bar| codes(bar, false)),
            [
                [180, 128, 128],
                [168, 44, 136],
                [145, 147, 44],
                [133, 63, 52],
                [63, 193, 204],
                [51, 109, 212],
                [28, 212, 120],
                [16, 128, 128],
            ]
        );
    }

    /// Check that the bars drawn at their own size are the colours of their codes, within
    /// 1/255, and the colours they stand for, within 2/255.
    fn bars_are_drawn(gpu: &Gpu, painter: &mut Painter, ten: bool) {
        let (width, height) = (64, 16);
        let bar = |x: u32| (x / 8) as usize;
        let picture = picture(width, height, ten, |x, _| codes(BARS[bar(x)], ten));
        painter.upload(&picture).unwrap();
        let pixels = draw(painter, gpu, (width, height), 0);
        for (index, nominal) in BARS.iter().enumerate() {
            let exact = rgb(codes(*nominal, ten), ten);
            for (x, y) in [(index * 8 + 3, 7), (index * 8 + 4, 8)] {
                let drawn = pixels[y * width as usize + x];
                for channel in 0..3 {
                    let value = f64::from(drawn[channel]);
                    assert!(
                        (value - exact[channel] * 255.0).abs() <= 1.0
                            && (value - nominal[channel] * 255.0).abs() <= 2.0,
                        "bar {index} in {} bits at ({x}, {y}): {drawn:?}, not {:?} ({nominal:?})",
                        if ten { 10 } else { 8 },
                        exact.map(|value| value * 255.0),
                    );
                }
            }
        }
    }

    #[test]
    fn colour_bars_of_nv12_and_p010_are_drawn_in_their_colours() {
        let Some(gpu) = gpu() else { return };
        let mut painter = Painter::new(&gpu.device, &gpu.queue, wgpu::TextureFormat::Bgra8Unorm);
        bars_are_drawn(&gpu, &mut painter, false);
        bars_are_drawn(&gpu, &mut painter, true);
        // And with 16-bit integers, as on a card without 16-bit normalized formats.
        painter.sixteen_norm = false;
        bars_are_drawn(&gpu, &mut painter, true);
        assert!(painter.uint.is_some());
    }

    /// Codes of P010 that differ in their low bits, with what each must be drawn as.
    fn distinct_codes() -> Vec<[u16; 3]> {
        vec![
            [64, 512, 512],
            [65, 512, 512],
            [100, 512, 512],
            [129, 512, 512],
            [257, 512, 512],
            [301, 512, 512],
            [511, 512, 512],
            [512, 512, 512],
            [700, 512, 512],
            [701, 512, 512],
            [940, 512, 512],
            [502, 400, 655],
            [502, 655, 400],
            [333, 64, 960],
            [777, 960, 64],
            [600, 513, 511],
        ]
    }

    #[test]
    fn the_ten_bits_of_p010_are_read_from_the_high_bits() {
        let Some(gpu) = gpu() else { return };
        let mut painter = Painter::new(&gpu.device, &gpu.queue, wgpu::TextureFormat::Bgra8Unorm);
        let codes = distinct_codes();
        let width = u32::try_from(codes.len()).unwrap() * 2;
        let picture = picture(width, 2, true, |x, _| codes[(x / 2) as usize]);
        for sixteen_norm in [true, false] {
            painter.sixteen_norm = sixteen_norm && painter.sixteen_norm;
            painter.upload(&picture).unwrap();
            let pixels = draw(&painter, &gpu, (width, 2), 0);
            for (index, code) in codes.iter().enumerate() {
                let exact = rgb(*code, true).map(|value| value * 255.0);
                for y in 0..2 {
                    let drawn = pixels[y * width as usize + index * 2];
                    for channel in 0..3 {
                        assert!(
                            (f64::from(drawn[channel]) - exact[channel]).abs() <= 0.6,
                            "{code:?} drawn as {drawn:?}, not {exact:?}"
                        );
                    }
                }
            }
            // Grey of 10 bits, drawn in 8: 64 and 65 are both black, 700 and 701 differ by a
            // quarter of a step.
            let grey = |index: usize| pixels[index * 2][0];
            assert_eq!(grey(0), 0);
            assert_eq!(grey(1), 0);
            assert_eq!(grey(10), 255);
            assert_eq!(grey(6), 130);
        }
    }

    #[test]
    fn a_picture_is_turned_and_fitted_with_black_bars() {
        let Some(gpu) = gpu() else { return };
        let mut painter = Painter::new(&gpu.device, &gpu.queue, wgpu::TextureFormat::Bgra8Unorm);
        // Stored 8 × 4: its left half red, its right half blue.
        let (red, blue) = (
            codes([0.75, 0.0, 0.0], false),
            codes([0.0, 0.0, 0.75], false),
        );
        let picture = picture(8, 4, false, |x, _| if x < 4 { red } else { blue });
        painter.upload(&picture).unwrap();
        let is = |pixel: [u8; 3], colour: &str| match colour {
            "red" => pixel[0] > 150 && pixel[1] < 30 && pixel[2] < 30,
            "blue" => pixel[0] < 30 && pixel[1] < 30 && pixel[2] > 150,
            _ => pixel == [0, 0, 0],
        };
        let at = |pixels: &[[u8; 3]], x: usize, y: usize| pixels[y * 40 + x];
        // Turned a quarter clockwise into 40 × 40: 20 × 40 in the middle, its left on top.
        for (rotation, top, bottom) in [(90, "red", "blue"), (270, "blue", "red")] {
            let pixels = draw(&painter, &gpu, (40, 40), rotation);
            assert!(is(at(&pixels, 20, 5), top), "{rotation}");
            assert!(is(at(&pixels, 20, 35), bottom), "{rotation}");
            for x in [0, 9, 30, 39] {
                assert!(is(at(&pixels, x, 20), "black"), "{rotation}: {x}");
            }
            // The first and last columns of the picture.
            assert!(is(at(&pixels, 10, 10), top), "{rotation}");
            assert!(is(at(&pixels, 29, 30), bottom), "{rotation}");
        }
        // Upside down into 40 × 40: 40 × 20 in the middle, its left on the right.
        let pixels = draw(&painter, &gpu, (40, 40), 180);
        assert!(is(at(&pixels, 5, 20), "blue"));
        assert!(is(at(&pixels, 35, 20), "red"));
        for y in [0, 9, 30, 39] {
            assert!(is(at(&pixels, 20, y), "black"), "{y}");
        }
        // Into nothing, nothing, and no panic; then black, once it is forgotten.
        let _ = painter.draw(
            &gpu.device
                .create_texture(&wgpu::TextureDescriptor {
                    label: None,
                    size: wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Bgra8Unorm,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default()),
            (0, 0),
            0,
        );
        painter.forget();
        let pixels = draw(&painter, &gpu, (40, 40), 0);
        assert!(pixels.iter().all(|pixel| *pixel == [0, 0, 0]));
    }

    #[test]
    fn a_picture_of_odd_size_is_drawn_whole() {
        let Some(gpu) = gpu() else { return };
        let mut painter = Painter::new(&gpu.device, &gpu.queue, wgpu::TextureFormat::Bgra8Unorm);
        let white = codes([0.75, 0.75, 0.75], true);
        for (width, height) in [(1, 1), (3, 5), (17, 9)] {
            painter
                .upload(&picture(width, height, true, |_, _| white))
                .unwrap();
            let pixels = draw(&painter, &gpu, (width, height), 0);
            assert!(
                pixels
                    .iter()
                    .all(|pixel| pixel.iter().all(|value| value.abs_diff(191) <= 1)),
                "{width}×{height}: {pixels:?}"
            );
        }
    }

    /// A picture of `format` of `bits` bits, said to be of `coding`, whose pixel at (x, y) has
    /// the codes that `pixel` gives, its chroma that of the first pixel of the pixels that a
    /// sample of chroma is for.
    fn native(
        (width, height): (u32, u32),
        format: PictureFormat,
        bits: u8,
        coding: YuvCoding,
        pixel: impl Fn(u32, u32) -> [u16; 3],
    ) -> NativePicture {
        let bytes = format.bytes();
        let (across, down) = format.chroma_shift();
        let (chroma_width, chroma_height) =
            (width.div_ceil(1 << across), height.div_ceil(1 << down));
        let luma_stride = (width as usize * bytes).next_multiple_of(64) + 64;
        let chroma_stride = (chroma_width as usize * 2 * bytes).next_multiple_of(64);
        let mut luma = vec![0; luma_stride * height as usize];
        let mut chroma = vec![0; chroma_stride * chroma_height as usize];
        let put = |plane: &mut Vec<u8>, at: usize, code: u16| {
            if bytes == 2 {
                let word = u16::try_from(u32::from(code) << (16 - bits)).unwrap();
                plane[at..at + 2].copy_from_slice(&word.to_le_bytes());
            } else {
                plane[at] = u8::try_from(code).unwrap();
            }
        };
        for y in 0..height {
            for x in 0..width {
                put(
                    &mut luma,
                    y as usize * luma_stride + x as usize * bytes,
                    pixel(x, y)[0],
                );
            }
        }
        for y in 0..chroma_height {
            for x in 0..chroma_width {
                let [_, cb, cr] = pixel(x << across, y << down);
                let at = y as usize * chroma_stride + x as usize * 2 * bytes;
                put(&mut chroma, at, cb);
                put(&mut chroma, at + bytes, cr);
            }
        }
        NativePicture {
            frame: 0,
            width,
            height,
            format,
            bits,
            matrix: coding.matrix,
            range: coding.range,
            chroma_location: 1,
            luma,
            chroma,
            luma_stride,
            chroma_stride,
            ..NativePicture::default()
        }
    }

    /// A value from 0 to 1 as a half float, its bits below those of a half float dropped.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn half(value: f64) -> u16 {
        let value = value as f32;
        if value < 2f32.powi(-14) {
            return 0;
        }
        let bits = value.to_bits();
        let exponent = ((bits >> 23) & 0xff).cast_signed() - 127 + 15;
        ((exponent as u32) << 10 | ((bits >> 13) & 0x3ff)) as u16
    }

    /// The pixels of the picture composed by itself into a preview of its size, as R'G'B'.
    fn composed(gpu: &Arc<columbina_compose::Gpu>, picture: &NativePicture) -> Vec<[u8; 3]> {
        use columbina_compose::{Compositor, LayerColor, Scene, SceneLayer, Source, Target};
        let mut compositor = Compositor::new(Arc::clone(gpu));
        let target = Target::preview(gpu, picture.width, picture.height).unwrap();
        let scene = Scene {
            width: picture.width,
            height: picture.height,
            layers: vec![SceneLayer {
                source: Source::Picture(Arc::new(picture.clone()), 0),
                transform: columbina_compose::Transform::NONE,
                opacity: 1.0,
                color: LayerColor {
                    coding: YuvCoding::of_native(picture),
                    ..LayerColor::default()
                },
            }],
            ..Scene::default()
        };
        compositor.render(&scene, &target).unwrap();
        target
            .read_rgba()
            .unwrap()
            .as_chunks::<4>()
            .0
            .iter()
            .map(|pixel| [pixel[0], pixel[1], pixel[2]])
            .collect()
    }

    /// Add to `pictures` (of the sizes of `size`, made by the test below, in its order) ones
    /// of chroma sited centre, of alpha of a plane of its own, and of R'G'B'A with alpha.
    fn add_sited_and_alpha(pictures: &mut Vec<NativePicture>, size: (u32, u32)) {
        use columbina_compose::ColorRange;
        // Chroma sited elsewhere than left: of 4:2:0 and 4:2:2, centred.
        for index in [0, 2] {
            let mut centred = pictures[index].clone();
            centred.chroma_location = 2;
            pictures.push(centred);
        }
        // Alpha of 4:4:4 in a plane of its own, straight, rising across: over black.
        let mut prores = pictures[3].clone();
        prores.alpha = columbina_media::Alpha::Straight;
        prores.alpha_plane = vec![0; prores.luma.len()];
        for y in 0..size.1 as usize {
            for x in 0..size.0 as usize {
                let word = u16::try_from(x * 65535 / 31).unwrap();
                let at = y * prores.luma_stride + 2 * x;
                prores.alpha_plane[at..at + 2].copy_from_slice(&word.to_le_bytes());
            }
        }
        pictures.push(prores);
        // R'G'B'A: of words at full scale, straight, and of half floats, premultiplied.
        let rgba = |x: u32, y: u32| {
            let alpha = f64::from(x + y) / 46.0;
            [f64::from(x) / 31.0, f64::from(y) / 15.0, 0.5, alpha]
        };
        for (format, alpha) in [
            (PictureFormat::Rgba64, columbina_media::Alpha::Straight),
            (
                PictureFormat::RgbaF16,
                columbina_media::Alpha::Premultiplied,
            ),
        ] {
            let stride = size.0 as usize * 8;
            let mut plane = vec![0; stride * size.1 as usize];
            for y in 0..size.1 {
                for x in 0..size.0 {
                    let [r, g, b, a] = rgba(x, y);
                    let values = if format == PictureFormat::Rgba64 {
                        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                        [r, g, b, a].map(|value| (value * 65535.0).round() as u16)
                    } else {
                        [r * a, g * a, b * a, a].map(half)
                    };
                    let at = y as usize * stride + x as usize * 8;
                    for (channel, value) in values.into_iter().enumerate() {
                        plane[at + 2 * channel..at + 2 * channel + 2]
                            .copy_from_slice(&value.to_le_bytes());
                    }
                }
            }
            pictures.push(NativePicture {
                width: size.0,
                height: size.1,
                format,
                bits: 16,
                range: ColorRange::Full,
                alpha,
                luma: plane,
                luma_stride: stride,
                ..NativePicture::default()
            });
        }
    }

    #[test]
    fn a_picture_is_drawn_by_its_own_sampling_matrix_and_range_as_the_compositor_draws_it() {
        use columbina_compose::{ColorRange, YuvMatrix};
        let Some(shared) = columbina_compose::gpu_for_tests() else {
            return;
        };
        let gpu = Gpu {
            device: shared.device().clone(),
            queue: shared.queue().clone(),
        };
        let mut painter = Painter::new(&gpu.device, &gpu.queue, wgpu::TextureFormat::Bgra8Unorm);
        let size = (32, 16);
        let coding = |matrix, range| YuvCoding { matrix, range };
        let mut pictures = Vec::new();
        for (format, bits, coded) in [
            (
                PictureFormat::Nv12,
                8,
                coding(YuvMatrix::Bt601, ColorRange::Full),
            ),
            (
                PictureFormat::P010,
                10,
                coding(YuvMatrix::Bt709, ColorRange::Limited),
            ),
            (
                PictureFormat::P216,
                10,
                coding(YuvMatrix::Bt601, ColorRange::Limited),
            ),
            (
                PictureFormat::P416,
                12,
                coding(YuvMatrix::Bt2020, ColorRange::Full),
            ),
            (
                PictureFormat::P416,
                8,
                coding(YuvMatrix::Bt709, ColorRange::Limited),
            ),
        ] {
            let [black, luma_range, middle, chroma_range] = yuv::levels(coded, bits);
            // Luma that rises across, and chroma that changes at every pixel across and every
            // row down, so that where each sample of chroma lies shows.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let pixel = move |x: u32, y: u32| {
                let luma = black + luma_range * f64::from(x) / 31.0;
                let cb = middle + chroma_range * 0.4 * (f64::from(y % 5) / 4.0 - 0.5);
                let cr = middle + chroma_range * 0.4 * (f64::from((x * 7) % 11) / 10.0 - 0.5);
                [luma, cb, cr].map(|code| code.round() as u16)
            };
            pictures.push(native(size, format, bits, coded, pixel));
        }
        add_sited_and_alpha(&mut pictures, size);
        for picture in &pictures {
            let wanted = composed(&shared, picture);
            for sixteen_norm in [true, false] {
                painter.sixteen_norm = sixteen_norm && shared.has_sixteen_norm();
                painter.upload(picture).unwrap();
                let drawn = draw(&painter, &gpu, size, 0);
                for (index, (drawn, wanted)) in drawn.iter().zip(&wanted).enumerate() {
                    for channel in 0..3 {
                        assert!(
                            drawn[channel].abs_diff(wanted[channel]) <= 1,
                            "{:?} of {} bits, {:?} {:?}, pixel {index}: drawn {drawn:?}, not \
                             {wanted:?}",
                            picture.format,
                            picture.bits,
                            picture.matrix,
                            picture.range,
                        );
                    }
                }
            }
        }
        // Of full range and BT.601: white and black are 255 and 0, and R' 1, G' 0, B' 0 is red.
        let full = coding(YuvMatrix::Bt601, ColorRange::Full);
        let red = native((8, 4), PictureFormat::Nv12, 8, full, |x, _| match x / 2 {
            0 => [255, 128, 128],
            1 => [0, 128, 128],
            _ => [76, 85, 255],
        });
        painter.upload(&red).unwrap();
        let drawn = draw(&painter, &gpu, (8, 4), 0);
        // The even columns of the second row, where the samples of chroma lie.
        assert_eq!(drawn[8], [255, 255, 255]);
        assert_eq!(drawn[10], [0, 0, 0]);
        assert!(
            drawn[14][0] >= 253 && drawn[14][1] <= 2 && drawn[14][2] <= 2,
            "{drawn:?}"
        );
    }

    #[test]
    fn a_picture_whose_planes_are_short_is_refused() {
        let Some(gpu) = gpu() else { return };
        let mut painter = Painter::new(&gpu.device, &gpu.queue, wgpu::TextureFormat::Bgra8Unorm);
        let good = picture(16, 8, false, |_, _| [16, 128, 128]);
        let mut short = good.clone();
        short.chroma.truncate(good.chroma_stride * 3);
        assert!(painter.upload(&short).is_err());
        let mut narrow = good.clone();
        narrow.luma_stride = 8;
        assert!(painter.upload(&narrow).is_err());
        let mut empty = good.clone();
        empty.width = 0;
        assert!(painter.upload(&empty).is_err());
        assert!(painter.upload(&good).is_ok());
    }

    // The display mapping (display.rs): SDR into SDR is drawn as it always was, bit for bit;
    // everything else is mapped on the card as `Mapping::map` maps it on the processor.

    use crate::display::{mapping, Content, HdrTransfer, Output};

    /// A half float as an f32.
    fn from_half(bits: u16) -> f32 {
        let sign = if bits & 0x8000 == 0 { 1.0 } else { -1.0 };
        let exponent = i32::from((bits >> 10) & 0x1f);
        let fraction = f32::from(bits & 0x3ff);
        sign * match exponent {
            0 => fraction * 2f32.powi(-24),
            31 => f32::INFINITY,
            _ => (1.0 + fraction / 1024.0) * 2f32.powi(exponent - 15),
        }
    }

    /// A texture of half floats of `width` × 1, of the R'G'B' that `pixels` gives, alpha 1.
    fn halves(gpu: &Gpu, pixels: &[[f64; 3]]) -> (wgpu::Texture, Vec<[f32; 3]>) {
        let width = u32::try_from(pixels.len()).unwrap();
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let bits: Vec<[u16; 4]> = pixels
            .iter()
            .map(|[r, g, b]| [half(*r), half(*g), half(*b), 0x3C00])
            .collect();
        let bytes: Vec<u8> = bits
            .iter()
            .flatten()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        gpu.queue.write_texture(
            texture.as_image_copy(),
            &bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 8),
                rows_per_image: Some(1),
            },
            texture.size(),
        );
        // What the texture holds, exactly.
        let held = bits
            .iter()
            .map(|[r, g, b, _]| [from_half(*r), from_half(*g), from_half(*b)])
            .collect();
        (texture, held)
    }

    /// The pixels drawn into a target of half floats of `size`, a row after another.
    fn draw_halves(painter: &Painter, gpu: &Gpu, size: (u32, u32)) -> Vec<[f32; 3]> {
        halves_using(gpu, size, |target| painter.draw(target, size, 0))
    }

    fn halves_using(
        gpu: &Gpu,
        size: (u32, u32),
        commands: impl FnOnce(&wgpu::TextureView) -> wgpu::CommandBuffer,
    ) -> Vec<[f32; 3]> {
        let (width, height) = size;
        let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let drawn = commands(&view);
        let row = (width * 8).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(row * height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        encoder.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(height),
                },
            },
            target.size(),
        );
        gpu.queue.submit([drawn, encoder.finish()]);
        let (mapped, mapping) = std::sync::mpsc::channel();
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                mapped.send(result).unwrap();
            });
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        mapping.recv().unwrap().unwrap();
        let bytes = buffer.slice(..).get_mapped_range().unwrap().to_vec();
        let mut pixels = Vec::new();
        for y in 0..height as usize {
            for x in 0..width as usize {
                let at = y * row as usize + x * 8;
                let channel = |c: usize| {
                    from_half(u16::from_le_bytes([
                        bytes[at + 2 * c],
                        bytes[at + 2 * c + 1],
                    ]))
                };
                pixels.push([channel(0), channel(1), channel(2)]);
            }
        }
        pixels
    }

    #[test]
    fn sdr_into_sdr_is_drawn_as_it_always_was_bit_for_bit() {
        let Some(gpu) = gpu() else { return };
        let mut before = Painter::new(&gpu.device, &gpu.queue, wgpu::TextureFormat::Bgra8Unorm);
        let mut now = Painter::new(&gpu.device, &gpu.queue, wgpu::TextureFormat::Bgra8Unorm);
        // SDR into SDR is no mapping; one that was mapped and is not any more draws as before.
        assert!(!now.set_mapping(mapping(Content::Sdr, Output::Sdr)));
        let pq = mapping(
            Content::Hdr {
                transfer: HdrTransfer::Pq,
                peak_nits: 1000.0,
            },
            Output::Sdr,
        );
        assert!(now.set_mapping(pq));
        assert!(!now.set_mapping(pq));
        assert!(now.set_mapping(mapping(Content::Sdr, Output::Sdr)));
        let (width, height) = (64, 16);
        let bars = picture(width, height, true, |x, _| {
            codes(BARS[(x / 8) as usize], true)
        });
        // A picture, fitted with black bars and turned, and a composed frame beyond 0..1.
        let ramp: Vec<[f64; 3]> = (0..32)
            .map(|step| {
                let value = f64::from(step) / 24.0 - 0.1;
                [value, 1.0 - value, value * value]
            })
            .collect();
        let (frame, _) = halves(&gpu, &ramp);
        for painter in [&mut before, &mut now] {
            painter.upload(&bars).unwrap();
        }
        for (size, rotation) in [((width, height), 0), ((50, 70), 90), ((33, 17), 180)] {
            assert_eq!(
                draw(&before, &gpu, size, rotation),
                draw(&now, &gpu, size, rotation),
                "{size:?} {rotation}"
            );
        }
        for painter in [&mut before, &mut now] {
            painter.use_texture(&frame).unwrap();
        }
        for size in [(32, 1), (40, 9)] {
            assert_eq!(draw(&before, &gpu, size, 0), draw(&now, &gpu, size, 0));
        }
    }

    /// Whether `drawn` is what `wanted` is, within the rounding of half floats and the
    /// approximations of the card's `pow` and `exp`.
    fn near_mapped(drawn: [f32; 3], wanted: [f32; 3]) -> bool {
        drawn
            .iter()
            .zip(wanted)
            .all(|(drawn, wanted)| (drawn - wanted).abs() <= 1e-3 + 3e-3 * wanted.abs())
    }

    #[test]
    fn sdr_into_scrgb_puts_white_on_the_white_of_sdr_as_the_processor_does() {
        let Some(gpu) = gpu() else { return };
        let mut painter = Painter::new(&gpu.device, &gpu.queue, wgpu::TextureFormat::Rgba16Float);
        let output = Output::Hdr {
            peak_nits: 400.0,
            sdr_white_nits: 240.0,
        };
        let sdr = mapping(Content::Sdr, output).unwrap();
        assert!(!painter.set_mapping(Some(sdr)));
        let values: Vec<[f64; 3]> = (0..=20)
            .map(|step| {
                let value = f64::from(step) / 20.0;
                [value, value * 0.5, 1.0 - value]
            })
            .chain([[1.5, -0.25, 0.04]])
            .collect();
        let (frame, held) = halves(&gpu, &values);
        painter.use_texture(&frame).unwrap();
        let drawn = draw_halves(&painter, &gpu, (22, 1));
        for (drawn, held) in drawn.iter().zip(&held) {
            let wanted = sdr.map(*held);
            assert!(
                near_mapped(*drawn, wanted),
                "{held:?}: {drawn:?}, not {wanted:?}"
            );
        }
        // White is 3 (240 nits), black 0.
        assert!((drawn[20][0] - 3.0).abs() < 2e-3, "{:?}", drawn[20]);
        assert!(
            drawn[0][0] == 0.0 && (drawn[0][2] - 3.0).abs() < 2e-3,
            "{:?}",
            drawn[0]
        );
        // A picture of a source, through the same mapping: as its SDR drawing, mapped.
        let mut sdr_painter =
            Painter::new(&gpu.device, &gpu.queue, wgpu::TextureFormat::Bgra8Unorm);
        let (width, height) = (64, 16);
        let bars = picture(width, height, false, |x, _| {
            codes(BARS[(x / 8) as usize], false)
        });
        sdr_painter.upload(&bars).unwrap();
        painter.upload(&bars).unwrap();
        let plain = draw(&sdr_painter, &gpu, (width, height), 0);
        let mapped = draw_halves(&painter, &gpu, (width, height));
        for (index, (plain, mapped)) in plain.iter().zip(&mapped).enumerate() {
            let wanted = sdr.map(plain.map(|value| f32::from(value) / 255.0));
            assert!(
                mapped
                    .iter()
                    .zip(wanted)
                    .all(|(mapped, wanted)| (mapped - wanted).abs() <= 0.02),
                "pixel {index}: {mapped:?}, not {wanted:?}"
            );
        }
    }

    #[test]
    fn hdr_content_is_mapped_on_the_card_as_on_the_processor() {
        use crate::display::{pq_oetf, srgb_oetf};
        let Some(gpu) = gpu() else { return };
        // PQ codes of greys and colours, from black beyond the peak of the content.
        let mut values: Vec<[f64; 3]> = [0.0, 0.5, 10.0, 100.0, 203.0, 300.0, 400.0, 800.0]
            .into_iter()
            .chain([1000.0, 4000.0])
            .map(|nits| [pq_oetf(nits); 3])
            .collect();
        values.extend([
            [pq_oetf(900.0), pq_oetf(50.0), pq_oetf(5.0)],
            [pq_oetf(20.0), pq_oetf(600.0), pq_oetf(100.0)],
            [pq_oetf(0.0), pq_oetf(30.0), pq_oetf(700.0)],
        ]);
        let hlg: Vec<[f64; 3]> = (0..=12)
            .map(|step| {
                let value = f64::from(step) / 12.0;
                [value, value * value, 1.0 - value]
            })
            .collect();
        let pq = Content::Hdr {
            transfer: HdrTransfer::Pq,
            peak_nits: 1000.0,
        };
        let hlg_content = Content::Hdr {
            transfer: HdrTransfer::Hlg,
            peak_nits: 1000.0,
        };
        let scrgb = Output::Hdr {
            peak_nits: 400.0,
            sdr_white_nits: 240.0,
        };
        let mut painter = Painter::new(&gpu.device, &gpu.queue, wgpu::TextureFormat::Rgba16Float);
        for (content, values) in [(pq, &values), (hlg_content, &hlg)] {
            let map = mapping(content, scrgb).unwrap();
            painter.set_mapping(Some(map));
            let (frame, held) = halves(&gpu, values);
            painter.use_texture(&frame).unwrap();
            let width = u32::try_from(values.len()).unwrap();
            let drawn = draw_halves(&painter, &gpu, (width, 1));
            for (drawn, held) in drawn.iter().zip(&held) {
                let wanted = map.map(*held);
                assert!(
                    near_mapped(*drawn, wanted),
                    "{content:?}, {held:?}: {drawn:?}, not {wanted:?}"
                );
                // Never beyond the peak of the monitor.
                assert!(drawn.iter().all(|value| *value * 80.0 <= 400.0 * 1.003));
            }
        }
        // PQ into SDR, onto 8 bits: within a code of the processor's.
        let mut sdr = Painter::new(&gpu.device, &gpu.queue, wgpu::TextureFormat::Bgra8Unorm);
        let map = mapping(pq, Output::Sdr).unwrap();
        assert!(sdr.set_mapping(Some(map)));
        let (frame, held) = halves(&gpu, &values);
        sdr.use_texture(&frame).unwrap();
        let width = u32::try_from(values.len()).unwrap();
        let drawn = draw(&sdr, &gpu, (width, 1), 0);
        for (drawn, held) in drawn.iter().zip(&held) {
            let wanted = map.map(*held).map(|value| value * 255.0);
            assert!(
                drawn
                    .iter()
                    .zip(wanted)
                    .all(|(drawn, wanted)| (f32::from(*drawn) - wanted).abs() <= 0.6),
                "{held:?}: {drawn:?}, not {wanted:?}"
            );
        }
        // 10 nits of grey is a tenth of the white of SDR.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let tenth = (srgb_oetf(0.1) * 255.0).round() as u8;
        assert!(drawn[2][0].abs_diff(tenth) <= 1, "{:?}", drawn[2]);
        assert_eq!(drawn[8], [255, 255, 255]);
    }
}

#[cfg(test)]
#[path = "render/legacy.rs"]
mod legacy;
