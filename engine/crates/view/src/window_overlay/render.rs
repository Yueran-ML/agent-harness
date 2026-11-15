use super::{sample_scale, Primitive, WindowOverlay};
use crate::display::{mapping, Content};
use crate::marks::{Marks, HALO, LINE_WIDTH};
use crate::{Output, ViewportMap};
use columbina_compose::wipe::{Style, Wipe, COVERAGE_WGSL};
use wgpu::util::DeviceExt;

/// Bytes of the per-draw uniform: dest, sampling and surface, style, then the wipe.
const GLOBALS: u64 = 48 + 96;
/// Bytes of one primitive in the storage buffer: five `vec4<f32>`.
const PRIMITIVE: u64 = 80;

/// The comparison of a frame that is all current picture: nothing of a reference to keep clear.
fn all_current() -> Wipe {
    Wipe {
        style: Style::Mix,
        position: 0.0,
        ..Wipe::default()
    }
}

pub(crate) struct OverlayPainter {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
}

impl OverlayPainter {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("columbina-view Window overlay"),
            source: wgpu::ShaderSource::Wgsl(
                format!("{COVERAGE_WGSL}\n{}", include_str!("overlay.wgsl")).into(),
            ),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("columbina-view Window overlay"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(GLOBALS),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(PRIMITIVE),
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("columbina-view Window overlay"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("columbina-view Window overlay"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_overlay"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_overlay"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        Self { layout, pipeline }
    }

    pub fn encode(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        overlay: &WindowOverlay,
        map: ViewportMap,
        output: Output,
    ) {
        let wipe = overlay.coverage.unwrap_or_else(all_current);
        // Selected is last, so the same shape wins drawing and pointer-down overlaps.
        let primitives = overlay
            .ordered()
            .flat_map(|shape| overlay.primitives(shape, map));
        self.paint(
            device,
            encoder,
            target,
            primitives,
            overlay.style.width,
            wipe,
            map,
            output,
        );
    }

    /// Draw the marks of a page over the whole picture: the dark ones first, with wider lines,
    /// and the light ones over them.
    pub fn encode_marks(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        marks: &Marks,
        map: ViewportMap,
        output: Output,
    ) {
        for (under, width) in [(true, LINE_WIDTH + 2.0 * HALO), (false, LINE_WIDTH)] {
            self.paint(
                device,
                encoder,
                target,
                marks.primitives(map, under).into_iter(),
                width,
                all_current(),
                map,
                output,
            );
        }
    }

    /// One pass over the display surface: one instance per primitive, each rasterizing only
    /// its own device quad, in list order, with strokes `width` device pixels wide, where
    /// `wipe` shows the current picture.
    #[allow(
        clippy::too_many_arguments,
        reason = "the surface, what is drawn on it and how, each from another hand"
    )]
    fn paint(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        primitives: impl Iterator<Item = Primitive>,
        width: f32,
        wipe: Wipe,
        map: ViewportMap,
        output: Output,
    ) {
        let coverage = wipe.coverage_params(map.picture);
        let sdr = mapping(Content::Sdr, output);
        let mut instances: Vec<u8> = vec![];
        for primitive in primitives {
            let Some(quad) = primitive.quad(map, width) else {
                continue;
            };
            let shape = primitive.geometry;
            let colour = sdr.map_or(primitive.colour, |mapping| mapping.map(primitive.colour));
            instances.extend(
                shape
                    .inverse
                    .into_iter()
                    .chain([
                        shape.centre[0],
                        shape.centre[1],
                        shape.scale,
                        primitive.kind,
                    ])
                    .chain([colour[0], colour[1], colour[2], 0.0])
                    .chain(quad)
                    .chain([0.0; 2])
                    .flat_map(f32::to_le_bytes),
            );
        }
        if instances.is_empty() {
            return;
        }
        #[allow(clippy::cast_precision_loss, reason = "surface sizes of at most 16384")]
        let mut globals: Vec<u8> = map
            .dest
            .into_iter()
            .chain(sample_scale(map))
            .chain([map.surface.0 as f32, map.surface.1 as f32])
            .chain([width, 0.0, 0.0, 0.0])
            .flat_map(f32::to_le_bytes)
            .collect();
        globals.extend_from_slice(&coverage);
        let globals = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("columbina-view Window overlay globals"),
            contents: &globals,
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let primitives = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("columbina-view Window primitives"),
            contents: &instances,
            usage: wgpu::BufferUsages::STORAGE,
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("columbina-view Window overlay"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: globals.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: primitives.as_entire_binding(),
                },
            ],
        });
        let count = u32::try_from(instances.len() as u64 / PRIMITIVE).unwrap_or(u32::MAX);
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("columbina-view Window overlay"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..6, 0..count);
        }
    }
}
