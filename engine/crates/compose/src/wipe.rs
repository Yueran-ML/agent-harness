//! The comparisons of the viewer of the Color page: a wipe of the frame shown against a
//! reference, and a split screen of several pictures in a grid (RM20 p2999–3006, p3260–3261).
//!
//! Both take pictures already composed ([`crate::Target::texture`], of [`WORKING_FORMAT`]) and
//! draw the picture that the viewer shows into another texture of that format. They are of the
//! preview alone: an export never goes through them, so what is rendered is the timeline as it
//! is, whatever the viewer compares it with.
//!
//! # Wipes
//!
//! The current picture is A, the reference B. Where a wipe is on, each pixel shows A, B, or a
//! blend of them, by a weight of B that is 0 on A's side and 1 on B's:
//!
//! - **Horizontal**, **Vertical** and **Diagonal** (p3000) are a straight border: upright (A on
//!   the left), level (A above), or at `angle` degrees from upright, turned clockwise (A on the
//!   side that the upright border has on the left). `position` moves it along its normal across
//!   the whole canvas and its softness beyond: at 1 the canvas is all A, at 0 all B, as dragging
//!   it to the end shows the reference full screen (p3260).
//! - **Box** (p3000): A in a box about the centre, B about it; `position` is the share of the
//!   canvas that the box spans each way, its softness beyond (1: all A, 0: all B).
//! - **Mix** (p3000): `position` is the share of B everywhere.
//! - **Difference** (p3000): `|A − B| · gain` of each channel, opaque. The manual says it shows
//!   "only the pixels that change": where A and B are alike it is black. It ignores `position`
//!   and `invert`. The gain is not in the manual, which names no control of it.
//! - **Venetian Blind** and **Checkerboard** (p3000): strips across, and checks, of A and B in
//!   turn, the first of A. `position` sets the height of a strip (from a pixel at 0 to half the
//!   canvas at 1, "drag up or down to change the size"), and the width of a check (from a pixel
//!   to half the canvas, "squeezing each check horizontally"); a check is an eighth of the
//!   canvas high.
//! - **Invert** (Alt+W, p3001) swaps the sides: the weight of B becomes `1 − w`.
//!
//! Softness and a border line are not in the manual for the viewer (only for the transitions of
//! the Edit page, p1031); they are here for the line styles and the box only, and off unless
//! set. `softness` is the width of a straight ramp of the weight across the border, as a share of
//! the shorter side of the canvas; `border` the width of a line drawn over it, in pixels, edged
//! by a pixel. A weight of exactly 0 or 1 shows A or B as they are, bit for bit, so a mix at 0
//! or 1 and a wipe dragged to an end are the picture itself.
//!
//! The Alpha style (p3000) needs the alpha output of the node graph, which the grade does not
//! give yet, and is not done.
//!
//! # Split screen
//!
//! The pictures, up to 16 (the most of every mode of p3004–3006), are laid in a grid in reading
//! order ([`tiles`]): as many columns as the square root of their number rounded up, as many rows
//! as they then need. Each is fitted into its cell keeping its shape, about the cell's centre,
//! black about it; the one selected is outlined in grey (Show Outline, on by default, p3004).
//! What the cells hold (selected clips, stills, neighbour clips, playheads) is for the caller.
//!
//! The scale constants of the styles whose maths the manual does not give are kept together
//! below ([`CHECK_ROWS`], [`OUTLINE`], [`OUTLINE_GREY`], [`MOST_TAPS`]), to be matched against
//! `DaVinci Resolve` itself.

use std::sync::Arc;

use crate::{ComposeError, Gpu, WORKING_FORMAT};

const SHADER: &str = include_str!("wipe.wgsl");
/// The same shape and reference-weight functions used by the image wipe and view controls.
/// Bindless WGSL, to prepend to a shader using [`Wipe::coverage_params`].
pub const COVERAGE_WGSL: &str = include_str!("wipe_coverage.wgsl");

/// The most pictures in a split screen.
pub const MOST_TILES: usize = 16;
/// How many checks of a checkerboard there are down the canvas.
pub const CHECK_ROWS: f32 = 8.0;
/// The width of the outline of the selected picture of a split screen, in pixels.
pub const OUTLINE: f32 = 2.0;
/// The grey of the outline (p3003: "highlighted in gray").
pub const OUTLINE_GREY: f32 = 0.5;
/// The most taps each way that a pixel of a tile of a split screen averages of its picture.
pub const MOST_TAPS: u32 = 4;

/// The words of the parameters of a wipe (`WipeParams` of `wipe.wgsl`).
const WIPE_PARAMS: usize = 24;
/// The bytes of the parameters of a tile (`TileParams`), each at its own offset.
const TILE_PARAMS: u32 = 48;

/// A style of wipe (p3000–3001).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Style {
    #[default]
    Horizontal,
    Vertical,
    Diagonal,
    Mix,
    Difference,
    Box,
    VenetianBlind,
    Checkerboard,
}

/// A wipe of A against B, as the viewer shows it. See the module for what each field does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Wipe {
    pub style: Style,
    /// Where the border is, the size of the box, the share of B of a mix, or the size of the
    /// strips and checks: from 0 to 1.
    pub position: f32,
    /// The angle of the border of a diagonal wipe, in degrees clockwise from upright.
    pub angle: f32,
    /// The width of the ramp across the border, as a share of the shorter side: from 0 to 1.
    pub softness: f32,
    /// The width of a line drawn on the border, in pixels: 0 for none, at most 64.
    pub border: f32,
    /// The colour of the line, R'G'B' of the canvas.
    pub border_colour: [f32; 3],
    /// Whether the sides are swapped (Invert Wipe).
    pub invert: bool,
    /// What the difference is made larger by: from 0 to 100.
    pub gain: f32,
}

impl Default for Wipe {
    /// A horizontal wipe at the middle (the default style and place of the viewer), hard, with
    /// no line; a diagonal at 45°, and a gain of 1.
    fn default() -> Self {
        Wipe {
            style: Style::Horizontal,
            position: 0.5,
            angle: 45.0,
            softness: 0.0,
            border: 0.0,
            border_colour: [1.0, 1.0, 1.0],
            invert: false,
            gain: 1.0,
        }
    }
}

impl Wipe {
    /// Whether current picture A contributes at this canvas pixel centre. Difference always
    /// permits editing A; a soft mix permits it until its weight is entirely B.
    #[must_use]
    pub fn has_current(&self, canvas: (u32, u32), at: [f32; 2]) -> bool {
        self.style == Style::Difference
            || Shape::of(self, canvas.0, canvas.1).weight(at[0], at[1]).0 < 1.0
    }

    /// Whether the clamped bilinear footprint at a canvas sample contains any current A.
    /// Only texels with positive sampling weight count; their wipe weights are evaluated
    /// at their centres, as in the composed image. Difference permits editing everywhere.
    #[must_use]
    #[allow(clippy::cast_precision_loss, reason = "dimensions of a canvas")]
    pub fn sample_has_current(&self, canvas: (u32, u32), at: [f32; 2]) -> bool {
        if canvas.0 == 0 || canvas.1 == 0 {
            return false;
        }
        if self.style == Style::Difference {
            return true;
        }
        let taps = |sample: f32, side: u32| {
            let sample = sample.clamp(0.5, side as f32 - 0.5);
            let low = (sample - 0.5).floor() + 0.5;
            let fraction = sample - low;
            [(low, 1.0 - fraction), (low + 1.0, fraction)]
        };
        let shape = Shape::of(self, canvas.0, canvas.1);
        taps(at[0], canvas.0).into_iter().any(|(x, x_weight)| {
            x_weight > 0.0
                && taps(at[1], canvas.1)
                    .into_iter()
                    .any(|(y, y_weight)| y_weight > 0.0 && shape.weight(x, y).0 < 1.0)
        })
    }

    /// The unchanged `WipeParams` uniform layout of [`COVERAGE_WGSL`], shared with the image.
    #[must_use]
    pub fn coverage_params(&self, canvas: (u32, u32)) -> Vec<u8> {
        Shape::of(self, canvas.0, canvas.1).bytes()
    }

    /// Check that each value is in its range.
    ///
    /// # Errors
    /// A value is not a number, or out of its range: what, said for a person.
    pub fn check(&self) -> Result<(), String> {
        let within = |name: &str, value: f32, low: f32, high: f32| {
            if value.is_finite() && (low..=high).contains(&value) {
                Ok(())
            } else {
                Err(format!("{name} is from {low} to {high}, not {value}"))
            }
        };
        within("position", self.position, 0.0, 1.0)?;
        within("angle", self.angle, -360.0, 360.0)?;
        within("softness", self.softness, 0.0, 1.0)?;
        within("border", self.border, 0.0, 64.0)?;
        within("gain", self.gain, 0.0, 100.0)?;
        for value in self.border_colour {
            within("a channel of border_colour", value, 0.0, 1.0)?;
        }
        Ok(())
    }
}

/// What a wipe is made of, worked out for a canvas: the parameters of the shader, and of the
/// reference on the processor alike.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Shape {
    /// 0 a line, 1 a box, 2 a mix, 3 a difference, 4 blinds, 5 a checkerboard.
    code: u32,
    invert: bool,
    centre: [f32; 2],
    normal: [f32; 2],
    /// How far the border of a line is from the centre along its normal.
    offset: f32,
    /// The width of the ramp, in pixels.
    soft: f32,
    half_box: [f32; 2],
    /// The share of B of a mix, or the gain of a difference.
    amount: f32,
    border: f32,
    colour: [f32; 3],
    cell: [f32; 2],
}

impl Shape {
    #[allow(clippy::cast_precision_loss, reason = "sizes of a canvas")]
    fn of(wipe: &Wipe, width: u32, height: u32) -> Shape {
        let (w, h) = (width as f32, height as f32);
        let angle = match wipe.style {
            Style::Vertical => 90.0_f32,
            Style::Diagonal => wipe.angle,
            _ => 0.0,
        };
        let (sin, cos) = angle.to_radians().sin_cos();
        let normal = match wipe.style {
            // Exact, so that an upright or level border falls between the same pixels on the
            // card as on the processor.
            Style::Horizontal => [1.0, 0.0],
            Style::Vertical => [0.0, 1.0],
            _ => [cos, sin],
        };
        // The canvas spans this along the normal; the border crosses all of it.
        let extent = normal[0].abs() * w + normal[1].abs() * h;
        let code = match wipe.style {
            Style::Horizontal | Style::Vertical | Style::Diagonal => 0,
            Style::Box => 1,
            Style::Mix => 2,
            Style::Difference => 3,
            Style::VenetianBlind => 4,
            Style::Checkerboard => 5,
        };
        let edged = code <= 1;
        // The border goes as far as the ramp beyond the canvas at either end, so that an end
        // shows all of A or all of B however soft it is.
        let soft = wipe.softness * w.min(h);
        Shape {
            code,
            invert: wipe.invert,
            centre: [w * 0.5, h * 0.5],
            normal,
            offset: (wipe.position - 0.5) * (extent + soft),
            soft,
            half_box: [
                wipe.position * (w * 0.5 + soft) - soft * 0.5,
                wipe.position * (h * 0.5 + soft) - soft * 0.5,
            ],
            amount: if code == 3 { wipe.gain } else { wipe.position },
            border: if edged { wipe.border } else { 0.0 },
            colour: wipe.border_colour,
            cell: [
                (wipe.position * w * 0.5).max(1.0),
                if code == 5 {
                    h / CHECK_ROWS
                } else {
                    (wipe.position * h * 0.5).max(1.0)
                },
            ],
        }
    }

    /// The parameters of the shader, as `WipeParams` lays them out.
    fn bytes(&self) -> Vec<u8> {
        let floats = [
            self.centre[0] * 2.0,
            self.centre[1] * 2.0,
            self.centre[0],
            self.centre[1],
            self.normal[0],
            self.normal[1],
            self.offset,
            self.soft,
            self.half_box[0],
            self.half_box[1],
            self.amount,
            self.border,
            self.cell[0],
            self.cell[1],
            0.0,
            0.0,
            self.colour[0],
            self.colour[1],
            self.colour[2],
            1.0,
        ];
        let mut words: Vec<u32> = floats.iter().map(|value| value.to_bits()).collect();
        words.extend([self.code, u32::from(self.invert), 0, 0]);
        debug_assert_eq!(words.len(), WIPE_PARAMS);
        words.into_iter().flat_map(u32::to_le_bytes).collect()
    }

    /// The weight of B at the centre of a pixel, and its signed distance from the border where
    /// there is one; as the shader works them out.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the number of a strip or check of a canvas, not negative"
    )]
    fn weight(&self, x: f32, y: f32) -> (f32, Option<f32>) {
        let ramp = |d: f32| {
            if self.soft > 0.0 {
                (d / self.soft + 0.5).clamp(0.0, 1.0)
            } else if d >= 0.0 {
                1.0
            } else {
                0.0
            }
        };
        let from = [x - self.centre[0], y - self.centre[1]];
        let (w, d) = match self.code {
            0 => {
                let d = from[0] * self.normal[0] + from[1] * self.normal[1] - self.offset;
                (ramp(d), Some(d))
            }
            1 => {
                let d = (from[0].abs() - self.half_box[0]).max(from[1].abs() - self.half_box[1]);
                (ramp(d), Some(d))
            }
            2 => (self.amount, None),
            4 => (odd((y / self.cell[1]).floor() as u32), None),
            _ => {
                let across = (x / self.cell[0]).floor() as u32;
                let down = (y / self.cell[1]).floor() as u32;
                (odd(across + down), None)
            }
        };
        (if self.invert { 1.0 - w } else { w }, d)
    }

    /// A pixel of the wipe of `current` (A) against `reference` (B), at the centre `at`.
    fn pixel(&self, at: (f32, f32), current: [f32; 4], reference: [f32; 4]) -> [f32; 4] {
        if self.code == 3 {
            let gain = self.amount;
            return [
                (current[0] - reference[0]).abs() * gain,
                (current[1] - reference[1]).abs() * gain,
                (current[2] - reference[2]).abs() * gain,
                1.0,
            ];
        }
        let (share, distance) = self.weight(at.0, at.1);
        let mut shown = if share >= 1.0 {
            reference
        } else if share > 0.0 {
            [0, 1, 2, 3]
                .map(|channel| current[channel] + (reference[channel] - current[channel]) * share)
        } else {
            current
        };
        if let Some(distance) = distance.filter(|_| self.border > 0.0) {
            let cover = (self.border * 0.5 - distance.abs() + 0.5).clamp(0.0, 1.0);
            for (value, colour) in shown.iter_mut().zip(self.colour) {
                *value += (colour - *value) * cover;
            }
        }
        shown
    }
}

/// 1 for an odd number, 0 for an even one: of the strips and checks of B.
fn odd(number: u32) -> f32 {
    if number & 1 == 1 {
        1.0
    } else {
        0.0
    }
}

/// The wipe of `a` against `b`, two pictures of `width` × `height` given by rows from the top,
/// as the card draws it: for the tests, and as the definition of each style.
///
/// # Panics
/// `a` or `b` is not of `width × height` pixels.
#[must_use]
#[allow(clippy::cast_precision_loss, reason = "places in a canvas")]
pub fn reference(
    wipe: &Wipe,
    width: u32,
    height: u32,
    a: &[[f32; 4]],
    b: &[[f32; 4]],
) -> Vec<[f32; 4]> {
    let count = width as usize * height as usize;
    assert!(
        a.len() == count && b.len() == count,
        "pictures of the canvas"
    );
    let shape = Shape::of(wipe, width, height);
    (0..count)
        .map(|index| {
            let x = (index % width as usize) as f32 + 0.5;
            let y = (index / width as usize) as f32 + 0.5;
            shape.pixel((x, y), a[index], b[index])
        })
        .collect()
}

/// How far the centre of pixel `(x, y)` is from the border of `wipe` on a canvas of `width` ×
/// `height`, in pixels; none for a style without one. For the tests, which leave out the pixels
/// that the card and the processor may put on either side of a hard border.
#[must_use]
#[allow(clippy::cast_precision_loss, reason = "places in a canvas")]
pub fn distance(wipe: &Wipe, width: u32, height: u32, x: u32, y: u32) -> Option<f32> {
    let shape = Shape::of(wipe, width, height);
    shape.weight(x as f32 + 0.5, y as f32 + 0.5).1
}

/// A cell of a split screen and where its picture is drawn in it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tile {
    /// The cell: left, top, width and height, in whole pixels of the canvas.
    pub cell: [u32; 4],
    /// The picture: left, top, right and bottom, in pixels of the canvas.
    pub image: [f32; 4],
}

/// The cells of a split screen of `count` pictures (at most [`MOST_TILES`]) of the shape
/// `picture` (width, height), on a canvas of `width` × `height`, in reading order.
#[must_use]
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a grid of at most 16 cells of a canvas"
)]
pub fn tiles(count: usize, width: u32, height: u32, picture: (u32, u32)) -> Vec<Tile> {
    let count = count.min(MOST_TILES);
    if count == 0 {
        return Vec::new();
    }
    let columns = (count as f64).sqrt().ceil() as u32;
    let rows = (count as u32).div_ceil(columns);
    let edge = |index: u32, of: u32, length: u32| {
        u32::try_from(u64::from(index) * u64::from(length) / u64::from(of)).unwrap_or(length)
    };
    let (shape_w, shape_h) = (picture.0.max(1) as f32, picture.1.max(1) as f32);
    (0..count as u32)
        .map(|index| {
            let (column, row) = (index % columns, index / columns);
            let left = edge(column, columns, width);
            let top = edge(row, rows, height);
            let cell_w = edge(column + 1, columns, width) - left;
            let cell_h = edge(row + 1, rows, height) - top;
            let scale = (cell_w as f32 / shape_w).min(cell_h as f32 / shape_h);
            let (image_w, image_h) = (shape_w * scale, shape_h * scale);
            let x = left as f32 + (cell_w as f32 - image_w) * 0.5;
            let y = top as f32 + (cell_h as f32 - image_h) * 0.5;
            Tile {
                cell: [left, top, cell_w, cell_h],
                image: [x, y, x + image_w, y + image_h],
            }
        })
        .collect()
}

/// A split screen: which picture is selected, and whether it is outlined.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Split {
    pub selected: Option<usize>,
    pub outline: bool,
}

/// The pipelines of the comparisons on a card, made once.
pub struct Wiper {
    gpu: Arc<Gpu>,
    wipe_layout: wgpu::BindGroupLayout,
    tile_layout: wgpu::BindGroupLayout,
    wipe: wgpu::RenderPipeline,
    tile: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    params: wgpu::Buffer,
    tile_params: wgpu::Buffer,
    /// The bytes from one tile's parameters to the next.
    tile_stride: u32,
}

impl std::fmt::Debug for Wiper {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Wiper").finish_non_exhaustive()
    }
}

impl Wiper {
    /// Make the pipelines on `gpu`.
    #[must_use]
    #[allow(
        clippy::too_many_lines,
        reason = "the layouts and pipelines of the wipe and the split, made together"
    )]
    pub fn new(gpu: Arc<Gpu>) -> Wiper {
        let device = gpu.device();
        let texture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let uniform = |binding, dynamic| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: dynamic,
                min_binding_size: None,
            },
            count: None,
        };
        let wipe_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("columbina wipe"),
            entries: &[texture(0), texture(1), uniform(2, false)],
        });
        let tile_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("columbina split"),
            entries: &[
                texture(3),
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                uniform(5, true),
            ],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("columbina wipe"),
            source: wgpu::ShaderSource::Wgsl(format!("{COVERAGE_WGSL}\n{SHADER}").into()),
        });
        let pipeline = |layout: &wgpu::BindGroupLayout, entry: &str, label: &str| {
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: &[Some(layout)],
                immediate_size: 0,
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
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
                    entry_point: Some(entry),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: WORKING_FORMAT,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let wipe = pipeline(&wipe_layout, "fs_wipe", "columbina wipe");
        let tile = pipeline(&tile_layout, "fs_tile", "columbina split");
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("columbina split"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..wgpu::SamplerDescriptor::default()
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("columbina wipe"),
            size: (WIPE_PARAMS * 4) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let tile_stride =
            TILE_PARAMS.next_multiple_of(device.limits().min_uniform_buffer_offset_alignment);
        let tile_params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("columbina split"),
            size: u64::from(tile_stride) * MOST_TILES as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Wiper {
            gpu,
            wipe_layout,
            tile_layout,
            wipe,
            tile,
            sampler,
            params,
            tile_params,
            tile_stride,
        }
    }

    #[must_use]
    pub fn gpu(&self) -> &Arc<Gpu> {
        &self.gpu
    }

    /// Check that the card is there and that `out` can be drawn into.
    fn check_out(&self, out: &wgpu::Texture) -> Result<(), ComposeError> {
        if let Some(why) = self.gpu.lost_reason() {
            return Err(ComposeError::Lost(why));
        }
        if out.format() != WORKING_FORMAT
            || !out.usage().contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
        {
            return Err(ComposeError::Target(
                "a comparison is drawn into a canvas of the preview".into(),
            ));
        }
        Ok(())
    }

    /// Draw the wipe of `a` (the current picture) against `b` (the reference) into `out`. All
    /// three are of one size; `a` and `b` are read (any format of floats that can be sampled),
    /// `out` is a canvas of [`WORKING_FORMAT`], as of a preview target.
    ///
    /// # Errors
    /// The card was lost; the pictures are of other sizes, or `out` cannot be drawn into; or
    /// the wipe is out of range ([`Wipe::check`]).
    pub fn wipe(
        &self,
        a: &wgpu::Texture,
        b: &wgpu::Texture,
        wipe: &Wipe,
        out: &wgpu::Texture,
    ) -> Result<(), ComposeError> {
        self.check_out(out)?;
        wipe.check().map_err(ComposeError::Target)?;
        if a.size() != out.size() || b.size() != out.size() {
            return Err(ComposeError::Target(format!(
                "a wipe is of pictures of one size, not {}×{}, {}×{} into {}×{}",
                a.width(),
                a.height(),
                b.width(),
                b.height(),
                out.width(),
                out.height()
            )));
        }
        let device = self.gpu.device();
        let shape = Shape::of(wipe, out.width(), out.height());
        self.gpu
            .queue()
            .write_buffer(&self.params, 0, &shape.bytes());
        let view =
            |texture: &wgpu::Texture| texture.create_view(&wgpu::TextureViewDescriptor::default());
        let (a_view, b_view, out_view) = (view(a), view(b), view(out));
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("columbina wipe"),
            layout: &self.wipe_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&a_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&b_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.params.as_entire_binding(),
                },
            ],
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("columbina wipe"),
        });
        {
            let mut pass = begin(&mut encoder, &out_view, "columbina wipe");
            pass.set_pipeline(&self.wipe);
            pass.set_bind_group(0, &group, &[]);
            pass.draw(0..3, 0..1);
        }
        self.gpu.queue().submit([encoder.finish()]);
        Ok(())
    }

    /// Draw a split screen of `pictures` (at most [`MOST_TILES`]) into `out`, laid out by
    /// [`tiles`], each fitted by its own shape, black about them.
    ///
    /// # Errors
    /// The card was lost; there are no pictures or too many; or `out` cannot be drawn into.
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "sizes of pictures and a few taps"
    )]
    pub fn split(
        &self,
        pictures: &[&wgpu::Texture],
        split: Split,
        out: &wgpu::Texture,
    ) -> Result<(), ComposeError> {
        self.check_out(out)?;
        if pictures.is_empty() || pictures.len() > MOST_TILES {
            return Err(ComposeError::Target(format!(
                "a split screen is of 1 to {MOST_TILES} pictures, not {}",
                pictures.len()
            )));
        }
        let device = self.gpu.device();
        let (width, height) = (out.width(), out.height());
        let shape = (pictures[0].width(), pictures[0].height());
        let cells = tiles(pictures.len(), width, height, shape);
        let mut bytes = vec![0_u8; self.tile_stride as usize * pictures.len()];
        let mut groups = Vec::with_capacity(pictures.len());
        let mut views = Vec::with_capacity(pictures.len());
        for (index, (picture, cell)) in pictures.iter().zip(&cells).enumerate() {
            // Each picture fitted by its own shape in the cell of the grid.
            let own = tiles(
                pictures.len(),
                width,
                height,
                (picture.width(), picture.height()),
            );
            let image = own[index].image;
            let across = picture.width() as f32 / (image[2] - image[0]).max(1.0);
            let down = picture.height() as f32 / (image[3] - image[1]).max(1.0);
            let taps = (across.max(down).ceil() as u32).clamp(1, MOST_TAPS);
            let outline = if split.outline && split.selected == Some(index) {
                OUTLINE
            } else {
                0.0
            };
            let floats = [
                image[0],
                image[1],
                image[2],
                image[3],
                taps as f32,
                outline,
                0.0,
                0.0,
                OUTLINE_GREY,
                OUTLINE_GREY,
                OUTLINE_GREY,
                1.0,
            ];
            let at = index * self.tile_stride as usize;
            for (word, value) in floats.iter().enumerate() {
                bytes[at + word * 4..at + word * 4 + 4].copy_from_slice(&value.to_le_bytes());
            }
            views.push((
                picture.create_view(&wgpu::TextureViewDescriptor::default()),
                cell.cell,
            ));
        }
        self.gpu.queue().write_buffer(&self.tile_params, 0, &bytes);
        for (view, _) in &views {
            groups.push(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("columbina split"),
                layout: &self.tile_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &self.tile_params,
                            offset: 0,
                            size: wgpu::BufferSize::new(u64::from(TILE_PARAMS)),
                        }),
                    },
                ],
            }));
        }
        let out_view = out.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("columbina split"),
        });
        {
            let mut pass = begin(&mut encoder, &out_view, "columbina split");
            pass.set_pipeline(&self.tile);
            for (index, (group, (_, cell))) in groups.iter().zip(&views).enumerate() {
                if cell[2] == 0 || cell[3] == 0 {
                    continue;
                }
                let offset = index as u32 * self.tile_stride;
                pass.set_scissor_rect(cell[0], cell[1], cell[2], cell[3]);
                pass.set_bind_group(0, group, &[offset]);
                pass.draw(0..3, 0..1);
            }
        }
        self.gpu.queue().submit([encoder.finish()]);
        Ok(())
    }
}

/// A pass that draws into `view`, cleared to opaque black first.
fn begin<'e>(
    encoder: &'e mut wgpu::CommandEncoder,
    view: &'e wgpu::TextureView,
    label: &str,
) -> wgpu::RenderPass<'e> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
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
    })
}

#[cfg(test)]
mod tests;
