//! A few pixels of a frame read back from the card as they are, in half floats made `f32`: what a
//! picker of colour reads where it is put on the picture (`view.sample`).
//!
//! The pixels are copied into a small buffer and read once the card has copied them, asking the
//! card again and again a moment apart rather than blocking in it, as the scopes read theirs (a
//! poll that waits holds a lock of the device that the views take to present).

use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::{ComposeError, Gpu, WORKING_FORMAT};

/// How long the card is waited for to copy the pixels.
const READ_WITHIN: Duration = Duration::from_secs(20);

/// How long to wait between two asks.
const READ_POLL: Duration = Duration::from_millis(1);

/// The most pixels on a side that are read at once.
pub const MOST: u32 = 64;

/// A rectangle of a texture, in pixels from its top left corner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// The pixels of `region` of `texture` (a canvas of [`WORKING_FORMAT`], as the targets of the
/// compositor are), R, G, B and A, a row after another, as they are: beyond 0 and 1 where they
/// are.
///
/// # Errors
/// The texture is of another format, the region is empty, larger than [`MOST`] on a side or not
/// within the texture, or the card could not give the pixels back (it was lost).
pub fn read_region(
    gpu: &Gpu,
    texture: &wgpu::Texture,
    region: Region,
) -> Result<Vec<[f32; 4]>, ComposeError> {
    if texture.format() != WORKING_FORMAT {
        return Err(ComposeError::Target(format!(
            "pixels are read from a canvas of {WORKING_FORMAT:?}, not {:?}",
            texture.format()
        )));
    }
    let within = region
        .x
        .checked_add(region.width)
        .is_some_and(|right| right <= texture.width())
        && region
            .y
            .checked_add(region.height)
            .is_some_and(|bottom| bottom <= texture.height());
    if region.width == 0
        || region.height == 0
        || region.width > MOST
        || region.height > MOST
        || !within
    {
        return Err(ComposeError::Target(format!(
            "{}×{} pixels at ({}, {}) cannot be read from a canvas of {}×{}",
            region.width,
            region.height,
            region.x,
            region.y,
            texture.width(),
            texture.height()
        )));
    }
    if let Some(why) = gpu.lost_reason() {
        return Err(ComposeError::Lost(why));
    }
    let device = gpu.device();
    let row = (region.width * 8).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("columbina sample"),
        size: u64::from(row) * u64::from(region.height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("columbina sample"),
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d {
                x: region.x,
                y: region.y,
                z: 0,
            },
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(region.height),
            },
        },
        wgpu::Extent3d {
            width: region.width,
            height: region.height,
            depth_or_array_layers: 1,
        },
    );
    gpu.queue().submit([encoder.finish()]);
    let bytes = mapped(gpu, &buffer)?;
    let width = region.width as usize * 8;
    let mut pixels = Vec::with_capacity(region.width as usize * region.height as usize);
    for line in bytes.chunks(row as usize) {
        for pixel in line[..width].as_chunks::<8>().0 {
            pixels
                .push([0, 2, 4, 6].map(|at| half(u16::from_le_bytes([pixel[at], pixel[at + 1]]))));
        }
    }
    Ok(pixels)
}

/// The mean of `pixels`, R, G and B; none where there are none.
#[must_use]
#[allow(clippy::cast_precision_loss, reason = "at most 4096 pixels")]
pub fn mean(pixels: &[[f32; 4]]) -> Option<[f32; 3]> {
    if pixels.is_empty() {
        return None;
    }
    let mut sum = [0.0_f64; 3];
    for pixel in pixels {
        for (total, value) in sum.iter_mut().zip(pixel) {
            *total += f64::from(*value);
        }
    }
    let count = pixels.len() as f64;
    #[allow(clippy::cast_possible_truncation, reason = "a mean of f32")]
    Some(sum.map(|total| (total / count) as f32))
}

/// Map `buffer` for reading, wait until it is, and take its bytes.
fn mapped(gpu: &Gpu, buffer: &wgpu::Buffer) -> Result<Vec<u8>, ComposeError> {
    let (sent, mapping) = mpsc::channel();
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            let _ = sent.send(result);
        });
    let deadline = Instant::now() + READ_WITHIN;
    loop {
        gpu.device()
            .poll(wgpu::PollType::Poll)
            .map_err(|error| ComposeError::Read(error.to_string()))?;
        match mapping.try_recv() {
            Ok(Ok(())) => break,
            Ok(Err(error)) => return Err(ComposeError::Read(error.to_string())),
            Err(mpsc::TryRecvError::Disconnected) => {
                return Err(ComposeError::Read("the mapping was dropped".into()));
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }
        if let Some(why) = gpu.lost_reason() {
            return Err(ComposeError::Lost(why));
        }
        if Instant::now() > deadline {
            return Err(ComposeError::Read(
                "the card did not copy the pixels".into(),
            ));
        }
        std::thread::sleep(READ_POLL);
    }
    let bytes = buffer
        .slice(..)
        .get_mapped_range()
        .map_err(|error| ComposeError::Read(error.to_string()))?
        .to_vec();
    buffer.unmap();
    Ok(bytes)
}

/// The value of a half float; 0 for one that is not a number.
fn half(bits: u16) -> f32 {
    let exponent = i32::from((bits >> 10) & 0x1F);
    let fraction = f32::from(bits & 0x3FF);
    let magnitude = match exponent {
        0 => fraction * 2.0_f32.powi(-24),
        31 if fraction == 0.0 => f32::INFINITY,
        31 => return 0.0,
        _ => (1.0 + fraction / 1024.0) * 2.0_f32.powi(exponent - 15),
    };
    if bits & 0x8000 == 0 {
        magnitude
    } else {
        -magnitude
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halves_are_read_as_they_are() {
        assert_eq!(half(0x3C00), 1.0);
        assert_eq!(half(0x3800), 0.5);
        assert_eq!(half(0xC000), -2.0);
        assert_eq!(half(0x0000), 0.0);
        assert_eq!(half(0x7C00), f32::INFINITY);
        assert_eq!(half(0x7E00), 0.0);
        assert_eq!(half(0x0001), 2.0_f32.powi(-24));
        assert_eq!(mean(&[]), None);
        assert_eq!(
            mean(&[[1.0, 0.0, 0.5, 1.0], [0.0, 0.0, 1.5, 1.0]]),
            Some([0.5, 0.0, 1.0])
        );
    }
}
