// The pre-viewport draw path, kept only as the GPU regression oracle.
use super::*;

impl Painter {
    #[allow(
        clippy::too_many_lines,
        clippy::cast_precision_loss,
        reason = "the frozen pre-viewport oracle, whose sizes are at most 16384"
    )]
    pub(super) fn legacy_draw(
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
        let drawn = if let Some(rgb) = &self.rgb_frame {
            letterbox(size, (rgb.width, rgb.height), 0).map(|placement| {
                (
                    &rgb.bind,
                    &self
                        .rgb_pipeline
                        .as_ref()
                        .expect("made with the frame")
                        .draw,
                    placement,
                )
            })
        } else {
            self.planes.as_ref().and_then(|planes| {
                let shape = (planes.layout.width, planes.layout.height);
                letterbox(size, shape, rotation).map(|placement| {
                    self.queue.write_buffer(
                        &self.uniform,
                        0,
                        &params(
                            planes,
                            [
                                placement.x as f32,
                                placement.y as f32,
                                placement.width as f32,
                                placement.height as f32,
                            ],
                            rotation,
                        ),
                    );
                    let pipeline = match planes.texel {
                        Texel::Float => &self.float,
                        Texel::Uint => self.uint.as_ref().expect("made with the planes"),
                    };
                    (&planes.bind, &pipeline.draw, placement)
                })
            })
        };
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
}
