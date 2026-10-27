//! The picture drawn on the graphics card: its planes of Y'CbCr uploaded as they are, turned
//! into R'G'B' as they are drawn (`yuv.wgsl`), and fitted into the target with black around it.
//!
//! Nothing here knows of windows: [`Painter`] draws into any texture of the format it was made
//! for, a window's or one that the tests read back.

use columbina_media::Picture;

/// The shader, for planes of floats (8-bit and 16-bit normalized textures).
const SHADER: &str = include_str!("yuv.wgsl");

/// Where a picture is drawn in a target: in pixels, from the top left corner of the target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// The quarter turns clockwise of a rotation in degrees clockwise, as the media has it: 0 to 3,
/// taken modulo 360 (so that -90 is 3). A rotation that is not a multiple of 90 is taken as
/// none (the model has only 0, 90, 180 and 270).
#[must_use]
pub fn quarter_turns(rotation: i16) -> u32 {
    match rotation.rem_euclid(360) {
        90 => 1,
        180 => 2,
        270 => 3,
        _ => 0,
    }
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

/// How the texels of a plane are read in the shader.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Texel {
    /// Normalized, as floats from 0 to 1.
    Float,
    /// 16-bit integers, where the card has no 16-bit normalized formats.
    Uint,
}

/// A pipeline for planes of one kind of texel.
struct Pipeline {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
}

/// The textures of the planes of the last picture, kept for the next one of the same shape.
struct Planes {
    width: u32,
    height: u32,
    ten: bool,
    texel: Texel,
    luma: wgpu::Texture,
    chroma: wgpu::Texture,
    bind: wgpu::BindGroup,
}

/// What draws pictures into targets of one format, on one device.
pub(crate) struct Painter {
    device: wgpu::Device,
    queue: wgpu::Queue,
    format: wgpu::TextureFormat,
    /// Whether the device has `R16Unorm` and `Rg16Unorm`, for pictures of P010.
    sixteen_norm: bool,
    uniform: wgpu::Buffer,
    float: Pipeline,
    /// Made when a picture of P010 is first drawn without 16-bit normalized formats.
    uint: Option<Pipeline>,
    planes: Option<Planes>,
}

/// The features that a painter uses where the adapter has them.
pub(crate) fn wanted_features(adapter: &wgpu::Adapter) -> wgpu::Features {
    adapter.features() & wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
}

/// The limits that a painter asks for: the defaults, with textures as large as the adapter
/// takes, for pictures beyond 8K.
pub(crate) fn wanted_limits(adapter: &wgpu::Adapter) -> wgpu::Limits {
    wgpu::Limits {
        max_texture_dimension_2d: adapter.limits().max_texture_dimension_2d,
        ..wgpu::Limits::default()
    }
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
            size: PARAMS_SIZE,
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
        }
    }

    /// Upload the planes of a picture, into the textures of the last one where it is of the
    /// same shape.
    ///
    /// # Errors
    /// The picture is not one that can be drawn: no pixels, planes shorter than it says, or
    /// larger than the card takes. Nothing is uploaded, and the last picture is kept.
    pub(crate) fn upload(&mut self, picture: &Picture) -> Result<(), String> {
        let shape = Shape::of(picture)?;
        let most = self.device.limits().max_texture_dimension_2d;
        if picture.width > most || picture.height > most {
            return Err(format!(
                "a picture of {}×{} is larger than the {most}×{most} that the card takes",
                picture.width, picture.height
            ));
        }
        let texel = if !shape.ten || self.sixteen_norm {
            Texel::Float
        } else {
            Texel::Uint
        };
        let reuse = self.planes.as_ref().is_some_and(|planes| {
            (planes.width, planes.height, planes.ten, planes.texel)
                == (picture.width, picture.height, shape.ten, texel)
        });
        if !reuse {
            self.planes = Some(self.planes_for(picture.width, picture.height, shape.ten, texel));
        }
        let planes = self.planes.as_ref().expect("made above");
        for (texture, data, stride, size) in [
            (
                &planes.luma,
                &picture.luma,
                picture.luma_stride,
                planes.luma.size(),
            ),
            (
                &planes.chroma,
                &picture.chroma,
                picture.chroma_stride,
                planes.chroma.size(),
            ),
        ] {
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(u32::try_from(stride).map_err(|_| "a stride too long")?),
                    rows_per_image: Some(size.height),
                },
                size,
            );
        }
        Ok(())
    }

    /// Textures, and what binds them, for planes of a shape.
    fn planes_for(&mut self, width: u32, height: u32, ten: bool, texel: Texel) -> Planes {
        let (luma_format, chroma_format) = match (ten, texel) {
            (false, _) => (wgpu::TextureFormat::R8Unorm, wgpu::TextureFormat::Rg8Unorm),
            (true, Texel::Float) => (
                wgpu::TextureFormat::R16Unorm,
                wgpu::TextureFormat::Rg16Unorm,
            ),
            (true, Texel::Uint) => (wgpu::TextureFormat::R16Uint, wgpu::TextureFormat::Rg16Uint),
        };
        let texture = |label, width, height, format| {
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        };
        let luma = texture("columbina-view luma", width, height, luma_format);
        let chroma = texture(
            "columbina-view chroma",
            width.div_ceil(2),
            height.div_ceil(2),
            chroma_format,
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
            width,
            height,
            ten,
            texel,
            luma,
            chroma,
            bind,
        }
    }

    /// Forget the picture that was uploaded: what is drawn next is black, until another is.
    pub(crate) fn forget(&mut self) {
        self.planes = None;
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
        let drawn = self.planes.as_ref().and_then(|planes| {
            letterbox(size, (planes.width, planes.height), rotation)
                .map(|placement| (planes, placement))
        });
        if let Some((planes, placement)) = drawn {
            self.queue
                .write_buffer(&self.uniform, 0, &params(planes, placement, rotation));
        }
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
            if let Some((planes, placement)) = drawn {
                let pipeline = match planes.texel {
                    Texel::Float => &self.float,
                    Texel::Uint => self.uint.as_ref().expect("made with the planes"),
                };
                #[allow(clippy::cast_precision_loss, reason = "sizes of at most 16384")]
                pass.set_viewport(
                    placement.x as f32,
                    placement.y as f32,
                    placement.width as f32,
                    placement.height as f32,
                    0.0,
                    1.0,
                );
                pass.set_pipeline(&pipeline.pipeline);
                pass.set_bind_group(0, &planes.bind, &[]);
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

/// The bytes of the uniform of the shader.
const PARAMS_SIZE: u64 = 64;

/// The uniform of the shader (`Params` in `yuv.wgsl`), as bytes.
#[allow(clippy::cast_precision_loss, reason = "sizes of at most 16384")]
fn params(planes: &Planes, placement: Placement, rotation: i16) -> Vec<u8> {
    // What a texel is multiplied by to be a code: 8 bits as 0..=255; P010, whose 10 bits are
    // the high bits of 16, as 0..=65535 / 64 normalized, or as the integer / 64.
    let scale = match (planes.ten, planes.texel) {
        (false, _) => 255.0,
        (true, Texel::Float) => 65535.0 / 64.0,
        (true, Texel::Uint) => 1.0 / 64.0,
    };
    let (black, luma_range, middle, chroma_range) = if planes.ten {
        (64.0, 876.0, 512.0, 896.0)
    } else {
        (16.0, 219.0, 128.0, 224.0)
    };
    let values: [f32; 16] = [
        placement.x as f32,
        placement.y as f32,
        placement.width as f32,
        placement.height as f32,
        planes.width as f32,
        planes.height as f32,
        planes.width.div_ceil(2) as f32,
        planes.height.div_ceil(2) as f32,
        scale,
        black,
        luma_range,
        quarter_turns(rotation) as f32,
        middle,
        chroma_range,
        0.0,
        0.0,
    ];
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

/// What a picture holds, once it is checked that its planes hold it.
struct Shape {
    ten: bool,
}

impl Shape {
    fn of(picture: &Picture) -> Result<Shape, String> {
        let (width, height) = (picture.width as usize, picture.height as usize);
        if width == 0 || height == 0 {
            return Err(format!(
                "a picture of {}×{} has no pixels",
                picture.width, picture.height
            ));
        }
        let ten = picture.bits > 8;
        let bytes = if ten { 2 } else { 1 };
        for (name, plane, stride, row, rows) in [
            (
                "luma",
                &picture.luma,
                picture.luma_stride,
                width * bytes,
                height,
            ),
            (
                "chroma",
                &picture.chroma,
                picture.chroma_stride,
                width.div_ceil(2) * 2 * bytes,
                height.div_ceil(2),
            ),
        ] {
            let needed = stride
                .checked_mul(rows - 1)
                .and_then(|before| before.checked_add(row));
            if stride < row || needed.is_none_or(|needed| plane.len() < needed) {
                return Err(format!(
                    "the {name} plane of a picture of {}×{} in {} bits is {} bytes in rows of {stride}",
                    picture.width,
                    picture.height,
                    picture.bits,
                    plane.len()
                ));
            }
        }
        Ok(Shape { ten })
    }
}

/// The pipeline for planes of a kind of texel, drawing into targets of `format`.
fn pipeline(device: &wgpu::Device, format: wgpu::TextureFormat, texel: Texel) -> Pipeline {
    let (source, sample_type) = match texel {
        Texel::Float => (
            SHADER.to_owned(),
            wgpu::TextureSampleType::Float { filterable: false },
        ),
        Texel::Uint => (
            SHADER.replacen("alias Texel = f32;", "alias Texel = u32;", 1),
            wgpu::TextureSampleType::Uint,
        ),
    };
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("columbina-view yuv"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let plane = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type,
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
                    min_binding_size: wgpu::BufferSize::new(PARAMS_SIZE),
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

#[cfg(test)]
mod tests {
    use super::*;

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
    // adapter the tests are skipped, unless COLUMBINA_REQUIRE_GPU is 1.

    struct Gpu {
        device: wgpu::Device,
        queue: wgpu::Queue,
    }

    fn gpu() -> Option<Gpu> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = match pollster::block_on(
            instance.request_adapter(&wgpu::RequestAdapterOptions::default()),
        ) {
            Ok(adapter) => adapter,
            Err(error) => {
                assert!(
                    std::env::var("COLUMBINA_REQUIRE_GPU").as_deref() != Ok("1"),
                    "COLUMBINA_REQUIRE_GPU is 1, and there is no graphics adapter: {error}"
                );
                eprintln!("skipped: no graphics adapter: {error}");
                return None;
            }
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: None,
            required_features: wanted_features(&adapter),
            required_limits: wanted_limits(&adapter),
            ..wgpu::DeviceDescriptor::default()
        }))
        .unwrap();
        Some(Gpu { device, queue })
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
    ) -> Picture {
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
        Picture {
            frame: 0,
            width,
            height,
            bits: if ten { 10 } else { 8 },
            luma,
            chroma,
            luma_stride: stride,
            chroma_stride: stride,
        }
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
