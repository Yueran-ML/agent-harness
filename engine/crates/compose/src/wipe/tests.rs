#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::float_cmp,
    clippy::manual_midpoint,
    reason = "pixels of small canvases, and positions that are exactly 0 or 1"
)]

use std::sync::Arc;
use std::time::Instant;

use super::*;
use crate::gpu_for_tests;

/// The bits of a half float nearest `value` (ties to even), for values of pictures (no NaN).
fn half_bits(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = u16::try_from((bits >> 16) & 0x8000).unwrap();
    let exponent = i32::try_from((bits >> 23) & 0xFF).unwrap() - 127 + 15;
    let mantissa = bits & 0x7F_FFFF;
    if exponent >= 0x1F {
        return sign | 0x7C00;
    }
    let (kept, shift) = if exponent <= 0 {
        if exponent < -10 {
            return sign;
        }
        (mantissa | 0x80_0000, u32::try_from(14 - exponent).unwrap())
    } else {
        (mantissa, 13)
    };
    let mut half = kept >> shift;
    let rest = kept & ((1 << shift) - 1);
    let middle = 1 << (shift - 1);
    if rest > middle || (rest == middle && half & 1 == 1) {
        half += 1;
    }
    let exponent = u32::try_from(exponent.max(0)).unwrap();
    sign | u16::try_from((exponent << 10) + half).unwrap()
}

fn half_value(half: u16) -> f32 {
    let exponent = i32::from((half >> 10) & 0x1F);
    let fraction = f32::from(half & 0x3FF);
    let magnitude = match exponent {
        0 => fraction * 2.0_f32.powi(-24),
        31 => f32::INFINITY,
        _ => (1.0 + fraction / 1024.0) * 2.0_f32.powi(exponent - 15),
    };
    if half & 0x8000 == 0 {
        magnitude
    } else {
        -magnitude
    }
}

/// A picture of half floats on the card, that can be read, drawn into and copied.
fn texture(gpu: &Gpu, width: u32, height: u32, pixels: &[[f32; 4]]) -> wgpu::Texture {
    let texture = gpu.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("wipe test"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: WORKING_FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    if !pixels.is_empty() {
        let bytes: Vec<u8> = pixels
            .iter()
            .flatten()
            .flat_map(|value| half_bits(*value).to_le_bytes())
            .collect();
        gpu.queue().write_texture(
            texture.as_image_copy(),
            &bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 8),
                rows_per_image: Some(height),
            },
            texture.size(),
        );
    }
    texture
}

/// The half floats of a picture on the card, four a pixel, by rows from the top.
fn read(gpu: &Gpu, texture: &wgpu::Texture) -> Vec<[u16; 4]> {
    let (width, height) = (texture.width(), texture.height());
    let row = (width * 8).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buffer = gpu.device().create_buffer(&wgpu::BufferDescriptor {
        label: Some("wipe test read"),
        size: u64::from(row) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(height),
            },
        },
        texture.size(),
    );
    gpu.queue().submit([encoder.finish()]);
    buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    gpu.device()
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    let mapped = buffer.slice(..).get_mapped_range().unwrap();
    let mut pixels = Vec::new();
    for line in mapped.chunks(row as usize) {
        for pixel in line[..width as usize * 8].as_chunks::<8>().0 {
            pixels.push(
                [0, 1, 2, 3].map(|channel| {
                    u16::from_le_bytes([pixel[channel * 2], pixel[channel * 2 + 1]])
                }),
            );
        }
    }
    pixels
}

/// A picture of `width` × `height` whose values are those of half floats, so that what the card
/// is given is what the reference is.
fn picture(width: u32, height: u32, value: impl Fn(u32, u32) -> [f32; 3]) -> Vec<[f32; 4]> {
    (0..height)
        .flat_map(|y| (0..width).map(move |x| (x, y)))
        .map(|(x, y)| {
            let [r, g, b] = value(x, y);
            [r, g, b, 1.0].map(|channel| half_value(half_bits(channel)))
        })
        .collect()
}

#[allow(clippy::cast_precision_loss, reason = "small canvases")]
fn gradients(width: u32, height: u32) -> (Vec<[f32; 4]>, Vec<[f32; 4]>) {
    let (w, h) = (width as f32, height as f32);
    let a = picture(width, height, |x, y| {
        [
            x as f32 / w,
            y as f32 / h,
            0.25 + 0.5 * ((x + y) % 7) as f32 / 7.0,
        ]
    });
    let b = picture(width, height, |x, y| {
        [
            1.0 - y as f32 / h,
            0.8 * x as f32 / w,
            1.2 - 0.1 * (x % 5) as f32,
        ]
    });
    (a, b)
}

fn every_style() -> [Style; 8] {
    [
        Style::Horizontal,
        Style::Vertical,
        Style::Diagonal,
        Style::Mix,
        Style::Difference,
        Style::Box,
        Style::VenetianBlind,
        Style::Checkerboard,
    ]
}

/// The pre-overlay shader is kept verbatim, so extraction cannot silently change image math.
fn legacy_wiper(gpu: &Arc<Gpu>) -> Wiper {
    let mut wiper = Wiper::new(Arc::clone(gpu));
    let device = gpu.device();
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("pre-overlay wipe"),
        source: wgpu::ShaderSource::Wgsl(include_str!("legacy.wgsl").into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[Some(&wiper.wipe_layout)],
        immediate_size: 0,
    });
    wiper.wipe = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: None,
        layout: Some(&layout),
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
            entry_point: Some("fs_wipe"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: WORKING_FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    });
    wiper
}

#[test]
fn extracting_overlay_coverage_keeps_the_image_wipe_bitwise_unchanged() {
    let Some(gpu) = gpu_for_tests() else { return };
    let before = legacy_wiper(&gpu);
    let after = Wiper::new(Arc::clone(&gpu));
    let (width, height) = (97, 61);
    let (a, b) = gradients(width, height);
    let (a, b) = (
        texture(&gpu, width, height, &a),
        texture(&gpu, width, height, &b),
    );
    let old = texture(&gpu, width, height, &[]);
    let new = texture(&gpu, width, height, &[]);
    for style in every_style() {
        for invert in [false, true] {
            for position in [0.0, 0.37, 1.0] {
                for softness in [0.0, 0.25] {
                    let wipe = Wipe {
                        style,
                        invert,
                        position,
                        softness,
                        angle: -31.0,
                        border: 3.0,
                        border_colour: [0.7, 0.2, 0.9],
                        gain: 4.0,
                    };
                    before.wipe(&a, &b, &wipe, &old).unwrap();
                    after.wipe(&a, &b, &wipe, &new).unwrap();
                    assert_eq!(read(&gpu, &old), read(&gpu, &new), "{wipe:?}");
                }
            }
        }
    }
}

#[test]
fn a_split_screen_is_a_grid_of_cells_in_reading_order_each_picture_fitted_in_its_own() {
    let (width, height) = (1920, 1080);
    let shape = (1920, 1080);
    // Columns and rows: the square root rounded up, and the rows that the pictures then need.
    for (count, columns, rows) in [
        (1, 1, 1),
        (2, 2, 1),
        (3, 2, 2),
        (4, 2, 2),
        (5, 3, 2),
        (6, 3, 2),
        (7, 3, 3),
        (9, 3, 3),
        (10, 4, 3),
        (12, 4, 3),
        (13, 4, 4),
        (16, 4, 4),
    ] {
        let cells = tiles(count, width, height, shape);
        assert_eq!(cells.len(), count);
        let lefts: std::collections::BTreeSet<u32> = cells.iter().map(|t| t.cell[0]).collect();
        let tops: std::collections::BTreeSet<u32> = cells.iter().map(|t| t.cell[1]).collect();
        assert_eq!(lefts.len(), columns.min(count), "{count}");
        assert_eq!(tops.len(), rows, "{count}");
        // Reading order: across, then down.
        for pair in cells.windows(2) {
            let (first, next) = (pair[0].cell, pair[1].cell);
            assert!(next[1] > first[1] || (next[1] == first[1] && next[0] > first[0]));
        }
        // A full grid covers the canvas exactly, cell by cell, with no gaps or overlaps.
        if count == columns * rows {
            let area: u64 = cells
                .iter()
                .map(|t| u64::from(t.cell[2]) * u64::from(t.cell[3]))
                .sum();
            assert_eq!(area, u64::from(width) * u64::from(height), "{count}");
        }
        for tile in &cells {
            let [x, y, w, h] = tile.cell.map(f64::from);
            let [left, top, right, bottom] = tile.image.map(f64::from);
            // Inside its cell, about its centre, as large as it fits, of the picture's shape.
            assert!(left >= x - 1e-3 && top >= y - 1e-3);
            assert!(right <= x + w + 1e-3 && bottom <= y + h + 1e-3);
            assert!(((left + right) / 2.0 - (x + w / 2.0)).abs() < 1e-3);
            assert!(((top + bottom) / 2.0 - (y + h / 2.0)).abs() < 1e-3);
            assert!((right - left - w).abs() < 1e-3 || (bottom - top - h).abs() < 1e-3);
            assert!(((right - left) / (bottom - top) - 1920.0 / 1080.0).abs() < 1e-3);
        }
    }
    // Four pictures fill the canvas in quarters; two are side by side, letterboxed.
    let four = tiles(4, width, height, shape);
    assert_eq!(four[3].cell, [960, 540, 960, 540]);
    assert_eq!(four[3].image, [960.0, 540.0, 1920.0, 1080.0]);
    let two = tiles(2, width, height, shape);
    assert_eq!(two[1].cell, [960, 0, 960, 1080]);
    assert_eq!(two[1].image, [960.0, 270.0, 1920.0, 810.0]);
    // An odd canvas is cut at whole pixels, the cells differing by a pixel at most.
    let odd = tiles(9, 1001, 563, (1001, 563));
    assert_eq!(
        odd.iter().map(|t| t.cell[2]).collect::<Vec<_>>(),
        [333, 334, 334, 333, 334, 334, 333, 334, 334]
    );
    // Nothing for nothing, and no more than 16.
    assert_eq!(tiles(0, width, height, shape), []);
    assert_eq!(tiles(40, width, height, shape).len(), MOST_TILES);
}

#[test]
fn each_style_shows_all_of_a_or_all_of_b_at_its_ends_and_invert_swaps_them() {
    let (width, height) = (33, 21);
    let (a, b) = gradients(width, height);
    for style in [
        Style::Horizontal,
        Style::Vertical,
        Style::Diagonal,
        Style::Box,
        Style::Mix,
    ] {
        for angle in [0.0, 30.0, 135.0, -60.0] {
            for position in [1.0, 0.0] {
                // A mix is all B at 1; the others all A.
                let all_a = (position == 1.0) != (style == Style::Mix);
                for invert in [false, true] {
                    let wipe = Wipe {
                        style,
                        position,
                        angle,
                        invert,
                        ..Wipe::default()
                    };
                    let shown = reference(&wipe, width, height, &a, &b);
                    let expected = if all_a == invert { &b } else { &a };
                    assert_eq!(&shown, expected, "{wipe:?}");
                }
            }
        }
    }
    // Half and half: A on the left of an upright border, above a level one, inside a box.
    let half = |style, invert| {
        reference(
            &Wipe {
                style,
                invert,
                ..Wipe::default()
            },
            width,
            height,
            &a,
            &b,
        )
    };
    let at = |x: u32, y: u32| (y * width + x) as usize;
    let horizontal = half(Style::Horizontal, false);
    assert_eq!(horizontal[at(3, 10)], a[at(3, 10)]);
    assert_eq!(horizontal[at(30, 10)], b[at(30, 10)]);
    let inverted = half(Style::Horizontal, true);
    assert_eq!(inverted[at(3, 10)], b[at(3, 10)]);
    let vertical = half(Style::Vertical, false);
    assert_eq!(vertical[at(16, 2)], a[at(16, 2)]);
    assert_eq!(vertical[at(16, 19)], b[at(16, 19)]);
    let boxed = half(Style::Box, false);
    assert_eq!(boxed[at(16, 10)], a[at(16, 10)]);
    assert_eq!(boxed[at(1, 1)], b[at(1, 1)]);
}

#[test]
fn values_out_of_their_ranges_are_refused() {
    assert_eq!(Wipe::default().check(), Ok(()));
    for wipe in [
        Wipe {
            position: 1.5,
            ..Wipe::default()
        },
        Wipe {
            position: f32::NAN,
            ..Wipe::default()
        },
        Wipe {
            softness: -0.1,
            ..Wipe::default()
        },
        Wipe {
            border: 65.0,
            ..Wipe::default()
        },
        Wipe {
            gain: f32::INFINITY,
            ..Wipe::default()
        },
        Wipe {
            border_colour: [0.0, 2.0, 0.0],
            ..Wipe::default()
        },
    ] {
        assert!(wipe.check().is_err(), "{wipe:?}");
    }
}

/// Whether two half floats are within a step or two of each other, as the card's fused
/// arithmetic and rounding may leave them.
fn near(card: u16, expected: f32) -> bool {
    let value = half_value(card);
    (value - expected).abs() <= 2.0e-3 * expected.abs().max(1.0)
}

#[test]
fn the_card_draws_each_style_as_the_reference_does_at_several_places_and_softnesses() {
    let Some(gpu) = gpu_for_tests() else { return };
    let wiper = Wiper::new(Arc::clone(&gpu));
    let (width, height) = (97, 61);
    let (a, b) = gradients(width, height);
    let (a_card, b_card) = (
        texture(&gpu, width, height, &a),
        texture(&gpu, width, height, &b),
    );
    let out = texture(&gpu, width, height, &[]);
    let mut checked = 0;
    for style in every_style() {
        for position in [0.0, 0.13, 0.5, 0.77, 1.0] {
            for softness in [0.0, 0.05, 0.4] {
                for (angle, invert, border) in [(45.0, false, 0.0), (-20.0, true, 3.0)] {
                    let wipe = Wipe {
                        style,
                        position,
                        angle,
                        softness,
                        border,
                        border_colour: [1.0, 0.25, 0.0],
                        invert,
                        gain: 4.0,
                    };
                    wiper.wipe(&a_card, &b_card, &wipe, &out).unwrap();
                    let card = read(&gpu, &out);
                    let expected = reference(&wipe, width, height, &a, &b);
                    let mut differ = Vec::new();
                    for (index, (card, expected)) in card.iter().zip(&expected).enumerate() {
                        let (x, y) = (index as u32 % width, index as u32 / width);
                        // A hard border may fall on either side of a pixel whose centre is
                        // on it, as the card rounds.
                        let on_edge = distance(&wipe, width, height, x, y).is_some_and(|d| {
                            softness == 0.0
                                && (d.abs() < 1e-3 || (d.abs() - border * 0.5).abs() < 1e-3)
                        });
                        if !on_edge && !(0..4).all(|channel| near(card[channel], expected[channel]))
                        {
                            differ.push((x, y, card.map(half_value), *expected));
                        }
                        checked += 1;
                    }
                    assert!(
                        differ.is_empty(),
                        "{wipe:?}: {:?}",
                        &differ[..differ.len().min(4)]
                    );
                }
            }
        }
    }
    assert_eq!(checked, 8 * 5 * 3 * 2 * width as usize * height as usize);
}

#[test]
fn the_difference_of_a_picture_and_itself_is_black() {
    let Some(gpu) = gpu_for_tests() else { return };
    let wiper = Wiper::new(Arc::clone(&gpu));
    let (width, height) = (64, 36);
    let (a, _) = gradients(width, height);
    let a_card = texture(&gpu, width, height, &a);
    let same = texture(&gpu, width, height, &a);
    let out = texture(&gpu, width, height, &[]);
    for gain in [1.0, 100.0] {
        let wipe = Wipe {
            style: Style::Difference,
            gain,
            ..Wipe::default()
        };
        wiper.wipe(&a_card, &same, &wipe, &out).unwrap();
        let black = half_bits(1.0);
        assert!(
            read(&gpu, &out)
                .iter()
                .all(|pixel| pixel[..3] == [0, 0, 0] && pixel[3] == black),
            "gain {gain}"
        );
    }
}

#[test]
fn a_mix_of_0_or_1_and_a_wipe_dragged_to_an_end_are_exactly_a_or_b() {
    let Some(gpu) = gpu_for_tests() else { return };
    let wiper = Wiper::new(Arc::clone(&gpu));
    let (width, height) = (80, 45);
    let (a, b) = gradients(width, height);
    let (a_card, b_card) = (
        texture(&gpu, width, height, &a),
        texture(&gpu, width, height, &b),
    );
    let (a_halves, b_halves) = (read(&gpu, &a_card), read(&gpu, &b_card));
    let out = texture(&gpu, width, height, &[]);
    for style in [
        Style::Mix,
        Style::Horizontal,
        Style::Vertical,
        Style::Diagonal,
        Style::Box,
    ] {
        for (position, invert) in [(0.0, false), (1.0, false), (0.0, true), (1.0, true)] {
            // A mix shows B by its position; the other styles show A at 1, B at 0.
            let shows_b = (position == 1.0) == (style == Style::Mix);
            let expected = if shows_b == invert {
                &a_halves
            } else {
                &b_halves
            };
            let wipe = Wipe {
                style,
                position,
                invert,
                softness: 0.2,
                ..Wipe::default()
            };
            wiper.wipe(&a_card, &b_card, &wipe, &out).unwrap();
            assert!(read(&gpu, &out) == *expected, "{wipe:?}");
        }
    }
}

#[test]
#[allow(clippy::cast_precision_loss, reason = "small canvases")]
fn a_split_screen_on_the_card_puts_each_picture_in_its_cell_and_outlines_the_one_selected() {
    let Some(gpu) = gpu_for_tests() else { return };
    let wiper = Wiper::new(Arc::clone(&gpu));
    let (width, height) = (160, 90);
    let colours = [
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 1.0],
    ];
    let pictures: Vec<wgpu::Texture> = colours
        .iter()
        .map(|colour| texture(&gpu, width, height, &picture(width, height, |_, _| *colour)))
        .collect();
    // A still of another shape (square) is fitted by its own.
    let square = texture(&gpu, 40, 40, &picture(40, 40, |_, _| [1.0, 0.0, 1.0]));
    let out = texture(&gpu, width, height, &[]);
    let mut shown: Vec<&wgpu::Texture> = pictures.iter().collect();
    shown.push(&square);
    let split = Split {
        selected: Some(1),
        outline: true,
    };
    wiper.split(&shown, split, &out).unwrap();
    let pixels = read(&gpu, &out);
    let at = |x: f32, y: f32| pixels[y as usize * width as usize + x as usize].map(half_value);
    let cells = tiles(6, width, height, (width, height));
    assert_eq!(cells.len(), 6);
    let grey = OUTLINE_GREY;
    for (index, tile) in cells.iter().enumerate() {
        let [left, top, right, bottom] = tile.image;
        let (cx, cy) = ((left + right) / 2.0, (top + bottom) / 2.0);
        let expected = if index == 5 {
            [1.0, 0.0, 1.0]
        } else {
            colours[index]
        };
        assert_eq!(at(cx, cy)[..3], expected, "the middle of tile {index}");
        if index == 5 {
            continue;
        }
        // Letterboxed above and below (three across, two down: cells of 53 × 45).
        assert!(top >= 1.0, "{tile:?}");
        assert_eq!(at(cx, top - 1.0)[..3], [0.0; 3], "above tile {index}");
        // The selected one has a grey edge, the others none.
        let edge = at(left + 1.0, cy);
        if index == 1 {
            assert_eq!(edge[..3], [grey; 3]);
        } else {
            assert_eq!(edge[..3], expected, "the edge of tile {index}");
        }
    }
    // The square still fills its cell's height and is pillarboxed: black beside it.
    let square = tiles(6, width, height, (40, 40))[5];
    let cell = square.cell;
    assert_eq!(square.image[1], cell[1] as f32);
    assert!(square.image[0] > cell[0] as f32 + 2.0);
    let middle = (square.image[1] + square.image[3]) / 2.0;
    assert_eq!(at(cell[0] as f32 + 1.0, middle)[..3], [0.0; 3]);
    assert_eq!(at(square.image[0] + 1.0, middle)[..3], [1.0, 0.0, 1.0]);
    // No outline where it is off.
    wiper
        .split(
            &shown,
            Split {
                selected: Some(1),
                outline: false,
            },
            &out,
        )
        .unwrap();
    let pixels = read(&gpu, &out);
    let [left, top, _, bottom] = cells[1].image;
    let edge = pixels[((top + bottom) / 2.0) as usize * width as usize + (left + 1.0) as usize];
    assert_eq!(
        edge[..3].iter().map(|h| half_value(*h)).collect::<Vec<_>>(),
        [0.0, 1.0, 0.0]
    );
    // Nothing, or too much, is refused.
    assert!(wiper.split(&[], split, &out).is_err());
    let many: Vec<&wgpu::Texture> = std::iter::repeat_n(&pictures[0], 17).collect();
    assert!(wiper.split(&many, split, &out).is_err());
}

#[test]
fn pictures_of_other_sizes_are_refused() {
    let Some(gpu) = gpu_for_tests() else { return };
    let wiper = Wiper::new(Arc::clone(&gpu));
    let a = texture(&gpu, 64, 36, &[]);
    let b = texture(&gpu, 32, 18, &[]);
    let out = texture(&gpu, 64, 36, &[]);
    let error = wiper.wipe(&a, &b, &Wipe::default(), &out).unwrap_err();
    assert_eq!(error.reason(), "target");
}

#[test]
#[ignore = "a timing, run by hand with --release"]
fn how_long_a_wipe_of_4k_takes() {
    let Some(gpu) = gpu_for_tests() else { return };
    eprintln!("on {} ({})", gpu.name(), gpu.backend());
    let wiper = Wiper::new(Arc::clone(&gpu));
    let (width, height) = (3840, 2160);
    let (a, b) = gradients(width, height);
    let (a_card, b_card) = (
        texture(&gpu, width, height, &a),
        texture(&gpu, width, height, &b),
    );
    let out = texture(&gpu, width, height, &[]);
    let wait = || {
        gpu.device()
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
    };
    for style in every_style() {
        let wipe = Wipe {
            style,
            softness: 0.05,
            border: 2.0,
            ..Wipe::default()
        };
        wiper.wipe(&a_card, &b_card, &wipe, &out).unwrap();
        wait();
        // One at a time, waited for, as the monitor draws a frame; and many in a row.
        let frames = 60;
        let started = Instant::now();
        for _ in 0..frames {
            wiper.wipe(&a_card, &b_card, &wipe, &out).unwrap();
            wait();
        }
        let alone = started.elapsed() / frames;
        let started = Instant::now();
        for _ in 0..frames {
            wiper.wipe(&a_card, &b_card, &wipe, &out).unwrap();
        }
        wait();
        let in_a_row = started.elapsed() / frames;
        eprintln!("{style:?} of 4K: {alone:?} each waited for, {in_a_row:?} each in a row");
        assert!(in_a_row.as_secs_f64() < 1e-3, "{style:?}: {in_a_row:?}");
    }
    let four: Vec<&wgpu::Texture> = vec![&a_card, &b_card, &a_card, &b_card];
    let started = Instant::now();
    for _ in 0..60 {
        wiper
            .split(
                &four,
                Split {
                    selected: Some(0),
                    outline: true,
                },
                &out,
            )
            .unwrap();
    }
    wait();
    eprintln!(
        "a split screen of four 4K pictures: {:?} each",
        started.elapsed() / 60
    );
}
