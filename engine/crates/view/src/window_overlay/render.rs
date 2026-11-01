use super::{sample_scale, WindowOverlay};
use crate::display::{mapping, Content};
use crate::{Output, ViewportMap};
use columbina_compose::wipe::{Style, Wipe, COVERAGE_WGSL};
use wgpu::util::DeviceExt;

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
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(176),
                },
                count: None,
            }],
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
        let wipe = overlay.coverage.unwrap_or(Wipe {
            style: Style::Mix,
            position: 0.0,
            ..Wipe::default()
        });
        let coverage = wipe.coverage_params(map.picture);
        // Selected is last, so the same shape wins drawing and pointer-down overlaps.
        let binds: Vec<_> = overlay
            .ordered()
            .flat_map(|shape| overlay.primitives(shape, map))
            .map(|primitive| {
                let shape = primitive.geometry;
                let colour = primitive.colour;
                let colour =
                    mapping(Content::Sdr, output).map_or(colour, |mapping| mapping.map(colour));
                let mut params: Vec<u8> = shape
                    .inverse
                    .into_iter()
                    .chain([
                        shape.centre[0],
                        shape.centre[1],
                        shape.scale,
                        overlay.style.width,
                    ])
                    .chain(map.dest)
                    .chain(sample_scale(map))
                    .chain([0.0; 2])
                    .chain([colour[0], colour[1], colour[2], primitive.kind])
                    .flat_map(f32::to_le_bytes)
                    .collect();
                params.extend_from_slice(&coverage);
                let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("columbina-view Window geometry"),
                    contents: &params,
                    usage: wgpu::BufferUsages::UNIFORM,
                });
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("columbina-view Window geometry"),
                    layout: &self.layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: buffer.as_entire_binding(),
                    }],
                })
            })
            .collect();
        if !binds.is_empty() {
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
            for bind in &binds {
                pass.set_bind_group(0, bind, &[]);
                pass.draw(0..3, 0..1);
            }
        }
    }
}
