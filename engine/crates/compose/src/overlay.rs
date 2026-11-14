//! What is written over a picture for a view alone, as a word of the viewer is (the name of a LUT
//! that cannot be read, Reference Manual 20, p. 3413): a line of text drawn on the processor as
//! subtitles are ([`label`]), and a frame copied with a block of pixels written over it on the
//! card ([`copy_with`]), so that nothing of the frame is read back.

use crate::{ComposeError, Gpu, Target};

/// A line of text, white letters outlined in black as subtitles are, `size` pixels high: its
/// width, height and pixels (R'G'B'A of 8 bits, premultiplied, row after row; none where it
/// would be more than 4096 × 2048 pixels).
#[must_use]
pub fn label(text: &str, size: f32) -> (u32, u32, Vec<[u8; 4]>) {
    crate::text::label(text, size)
}

/// `from` copied into `into`, a target of its size on its card, and the block of `size` pixels
/// of `halves` (half floats R, G, B, A, row after row) written over it at `at`, its left and top.
///
/// # Errors
/// The targets are not of a size, are of another card, or the block does not lie on the frame
/// or is not of its size.
pub fn copy_with(
    gpu: &Gpu,
    from: &Target,
    into: &Target,
    at: (u32, u32),
    size: (u32, u32),
    halves: &[u16],
) -> Result<(), ComposeError> {
    let frame = from.size();
    if into.size() != frame
        || !std::ptr::eq(from.gpu().as_ref(), gpu)
        || !std::ptr::eq(into.gpu().as_ref(), gpu)
    {
        return Err(ComposeError::Target(
            "a frame is copied into a target of its size, on its card".into(),
        ));
    }
    let within = |start: u32, length: u32, whole: u32| {
        start.checked_add(length).is_some_and(|end| end <= whole)
    };
    if size.0 == 0
        || size.1 == 0
        || !within(at.0, size.0, frame.0)
        || !within(at.1, size.1, frame.1)
        || halves.len() != size.0 as usize * size.1 as usize * 4
    {
        return Err(ComposeError::Target(format!(
            "a block of {}×{} at {}, {} does not lie on a frame of {}×{}, or is not of its size",
            size.0, size.1, at.0, at.1, frame.0, frame.1
        )));
    }
    let extent = |width, height| wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    };
    let mut encoder = gpu
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("columbina overlay"),
        });
    encoder.copy_texture_to_texture(
        from.texture().as_image_copy(),
        into.texture().as_image_copy(),
        extent(frame.0, frame.1),
    );
    gpu.queue().submit([encoder.finish()]);
    let bytes: Vec<u8> = halves.iter().flat_map(|half| half.to_le_bytes()).collect();
    gpu.queue().write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: into.texture(),
            mip_level: 0,
            origin: wgpu::Origin3d {
                x: at.0,
                y: at.1,
                z: 0,
            },
            aspect: wgpu::TextureAspect::All,
        },
        &bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(size.0 * 8),
            rows_per_image: Some(size.1),
        },
        extent(size.0, size.1),
    );
    Ok(())
}
