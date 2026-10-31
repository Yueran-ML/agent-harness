//! The picture drawn on the graphics card: its planes of Y'CbCr uploaded as they are, turned
//! into R'G'B' as they are drawn (`yuv.wgsl` of `columbina-compose`, which composes the layers
//! of a timeline with the same shader), and fitted into the target with black around it.
//!
//! Nothing here knows of windows: [`Painter`] draws into any texture of the format it was made
//! for, a window's or one that the tests read back.

use columbina_compose::yuv::{self, Layout, Texel};
use columbina_compose::YuvCoding;
use columbina_media::NativePicture;

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
    pipeline: wgpu::RenderPipeline,
}

/// The textures of the planes of the last picture, kept for the next one of the same shape, and
/// how that picture is read: its layout and its coding.
struct Planes {
    layout: Layout,
    coding: YuvCoding,
    texel: Texel,
    luma: wgpu::Texture,
    chroma: wgpu::Texture,
    bind: wgpu::BindGroup,
}

/// An already composed frame, still on the shared graphics device.
struct RgbFrame {
    width: u32,
    height: u32,
    bind: wgpu::BindGroup,
}

/// What draws pictures into targets of one format, on one device.
pub(crate) struct Painter {
    device: wgpu::Device,
    queue: wgpu::Queue,
    format: wgpu::TextureFormat,
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
            sixteen_norm: device
                .features()
                .contains(wgpu::Features::TEXTURE_FORMAT_16BIT_NORM),
            float: pipeline(device, format, Texel::Float),
            uint: None,
            device: device.clone(),
            queue: queue.clone(),
            format,
            uniform,
            planes: None,
            rgb_pipeline: None,
            rgb_frame: None,
            rgb_sampler: None,
        }
    }

    /// Upload the planes of a picture, into the textures of the last one where it is of the
    /// same shape (sizes and format), to be drawn by the matrix and range that it says.
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
        let texel = Texel::for_picture(layout.wide(), self.sixteen_norm);
        let reuse = self.planes.as_ref().is_some_and(|planes| {
            let kept = planes.layout;
            (kept.width, kept.height, kept.format, planes.texel)
                == (layout.width, layout.height, layout.format, texel)
        });
        if !reuse {
            self.planes = Some(self.planes_for(layout, texel));
        }
        let planes = self.planes.as_mut().expect("made above");
        planes.layout = layout;
        planes.coding = YuvCoding::of_native(picture);
        yuv::upload(&self.queue, &planes.luma, &planes.chroma, picture)?;
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
            .get_or_insert_with(|| rgb_pipeline(&self.device, self.format));
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
        let (luma, chroma) = yuv::plane_textures(
            &self.device,
            layout.width,
            layout.height,
            layout.format,
            texel,
        );
        let pipeline = match texel {
            Texel::Float => &self.float,
            Texel::Uint => self
                .uint
                .get_or_insert_with(|| pipeline(&self.device, self.format, Texel::Uint)),
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
                    resource: wgpu::BindingResource::TextureView(
                        &luma.create_view(&wgpu::TextureViewDescriptor::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(
                        &chroma.create_view(&wgpu::TextureViewDescriptor::default()),
                    ),
                },
            ],
        });
        Planes {
            layout,
            coding: YuvCoding::default(),
            texel,
            luma,
            chroma,
            bind,
        }
    }

    /// Forget the picture that was uploaded: what is drawn next is black, until another is.
    pub(crate) fn forget(&mut self) {
        self.planes = None;
        self.rgb_frame = None;
    }

    /// Draw into `target`, of `size` pixels: black, and the picture that was uploaded last, if
    /// any, fitted into it and turned `rotation` degrees clockwise. Gives the commands, to be
    /// submitted.
    pub(crate) fn draw(
        &self,
        target: &wgpu::TextureView,
        size: (u32, u32),
        rotation: i16,
    ) -> wgpu::CommandBuffer {
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("columbina-view"),
            });
        let drawn = if let Some(rgb) = &self.rgb_frame {
            letterbox(size, (rgb.width, rgb.height), 0).map(|placement| {
                (
                    &rgb.bind,
                    &self
                        .rgb_pipeline
                        .as_ref()
                        .expect("made with the frame")
                        .pipeline,
                    placement,
                )
            })
        } else {
            self.planes.as_ref().and_then(|planes| {
                let shape = (planes.layout.width, planes.layout.height);
                letterbox(size, shape, rotation).map(|placement| {
                    self.queue
                        .write_buffer(&self.uniform, 0, &params(planes, placement, rotation));
                    let pipeline = match planes.texel {
                        Texel::Float => &self.float,
                        Texel::Uint => self.uint.as_ref().expect("made with the planes"),
                    };
                    (&planes.bind, &pipeline.pipeline, placement)
                })
            })
        };
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("columbina-view"),
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
            if let Some((bind, pipeline, placement)) = drawn {
                #[allow(clippy::cast_precision_loss, reason = "sizes of at most 16384")]
                pass.set_viewport(
                    placement.x as f32,
                    placement.y as f32,
                    placement.width as f32,
                    placement.height as f32,
                    0.0,
                    1.0,
                );
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, bind, &[]);
                pass.draw(0..3, 0..1);
            }
        }
        encoder.finish()
    }

    /// The queue that the commands of [`Painter::draw`] are submitted to.
    pub(crate) fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }
}

/// The uniform of the shader (`Params` in `yuv.wgsl`), as bytes: the half of it that the view
/// uses, and nothing in the half that the compositor uses.
#[allow(clippy::cast_precision_loss, reason = "sizes of at most 16384")]
fn params(planes: &Planes, placement: Placement, rotation: i16) -> Vec<u8> {
    let dest = [
        placement.x as f32,
        placement.y as f32,
        placement.width as f32,
        placement.height as f32,
    ];
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
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("columbina-view yuv"),
        source: wgpu::ShaderSource::Wgsl(texel.shader().into()),
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
        ],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("columbina-view"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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
    Pipeline { layout, pipeline }
}

fn rgb_pipeline(device: &wgpu::Device, format: wgpu::TextureFormat) -> Pipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("columbina-view composed frame"),
        source: wgpu::ShaderSource::Wgsl(include_str!("rgb.wgsl").into()),
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
        ],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("columbina-view composed frame"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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
    Pipeline { layout, pipeline }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let drawn = painter.draw(&view, size, rotation);
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
        }
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
}
