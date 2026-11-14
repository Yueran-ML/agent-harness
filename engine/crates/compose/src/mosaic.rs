//! The Mosaic Blur of Resolve FX Blur (Reference Manual 20, p. 3465): "a simple, pixelated blur
//! suitable for hiding the face of anonymous witnesses", on the processor: the reference that a
//! shader is to be held to. The frame is laid out in a grid of cells, and each cell is shown in
//! one colour, the mean of the picture under it.
//!
//! Only square cells are done (the daily tier); Hexagon and Triangle are left to the advanced
//! one ([`CellShape`]).
//!
//! # The controls
//!
//! - **Pixel Frequency** ([`Settings::frequency`]): "the size of each pixel, thus determining the
//!   density and resolution of the resulting grid of pixels". The manual gives it no unit. Here
//!   it is how many cells lie across the width of the frame: in a frame `W` pixels wide a cell
//!   is `W / frequency` pixels wide, and never less than a pixel.
//! - **Aspect** ([`Settings::aspect`]): "the aspect ratio of the cells", here the width of a cell
//!   against its height: a cell is `W / frequency / aspect` pixels high, and never less than a
//!   pixel.
//! - **X Offset** and **Y Offset** ([`Settings::offset`]): "shifts the grid left or right" and "up
//!   or down", here by that many pixels of the frame, to the right and down.
//! - **Whole Cells Only** ([`Settings::whole_cells_only`]): "limits the showing of cells to those
//!   cells that are completely inside of a window only" (see the key, below).
//! - **Cell Shape** ([`Settings::shape`]): Square alone.
//!
//! Of the others of p. 3465: Aliasing ("how tightly each cell samples the area underneath it")
//! is not done, and a cell here takes all of the area under it; Antialias Result ("cleaner lines
//! between the cells") has nothing to do for squares, whose sides part whole pixels; and Blur
//! Alpha is as if ticked, the picture being premultiplied (below).
//!
//! # The grid
//!
//! Places are pixels of the frame, `[x, y]` with y down; the pixel `p` of an axis lies from `p`
//! to `p + 1`, and its centre is at `p + 0.5`. A pixel is of the cell that its centre lies in,
//! and a centre that lies on a side of cells is of the later cell.
//!
//! The grid is anchored at the centre of the frame: with no offset, a side of cells lies on
//! `W / 2` and one on `H / 2`. So the grid is alike left and right, and above and below; a
//! change of Pixel Frequency grows the cells about the middle of the picture; and an even number
//! of cells across fills the width with whole cells (at first, 32 by 18 of them in a frame of
//! 16:9). An odd number shows a half of a cell at either edge, and a Pixel Frequency of 1 shows
//! two halves, as 2 does: one cell over the whole frame is a Pixel Frequency of 1 with an offset
//! of half the frame. (A cell centred on the centre of the frame would do the same to the even
//! numbers.)
//!
//! All of it is counted in whole numbers, so that the card and the processor put every pixel in
//! the same cell whatever the card makes of a division of floats. A pixel is [`STEPS`] steps,
//! and along an axis of a frame `L` pixels long:
//!
//! - `size`, the size of a cell in steps: `round(STEPS · w)`, `w` its size in pixels as above,
//!   from 1 pixel to [`MOST_CELL`] of them.
//! - `lead`, how many steps the centre of the pixel 0 is past the side of cells at or before it:
//!   `(STEPS / 2 − (L · STEPS / 2 + round(STEPS · offset))) mod size`, from 0 to `size − 1`.
//! - The cell of the pixel `p` ([`Grid::cell`]): `(STEPS · p + lead) / size`, the division of
//!   whole numbers. Cells are counted from 0, the cell of the pixel 0, at the left and the top.
//! - The pixels of the cell `q` ([`Grid::rect`]): from `ceil((q · size − lead) / STEPS)`, or 0
//!   where `q · size` is not past `lead`, up to (and not with) that of the cell `q + 1`, both
//!   at most `L`: a cell at an edge of the frame is what of it is inside the frame.
//!
//! So the size of a cell and an offset are kept to 1/256 of a pixel, cells of a size that is no
//! whole number of pixels are of the whole number below it or above, in turn, and an offset of
//! the size of a cell puts the grid where it was.
//!
//! # The colour of a cell
//!
//! The mean of the pixels of the picture in the cell ([`cell_mean`]), of each of R, G, B and A.
//! The picture is premultiplied, so that a mean of it is the colour that the cell shows, and a
//! pixel that is not seen weighs nothing. A cell at an edge is the mean of what of it is inside
//! the frame: nothing beyond the frame is read. A pixel that is no number makes the colour of
//! its cell no number, and of no other.
//!
//! # The key
//!
//! Where the mosaic is of a node, it is where the Blur palette is (`grade/blur.rs`): the whole
//! picture that the node made is laid out in cells, and then mixed with the input by the key of
//! the node, so that the mean of a cell is of all its pixels, those outside a window too. With a
//! key, each pixel is `s + (m − s) · k` ([`mixed`]): `s` the source, `m` the colour of its cell,
//! `k` the key at the pixel within 0 and 1 ([`level`]; a key that is no number is 0). A key of
//! 0 is the source to the bit, and of 1 the colour of the cell to the bit. A cell that a window
//! crosses is so shown in part, cut by the edge of the window.
//!
//! With Whole Cells Only, a cell is shown only where the key is 1 at every pixel of it
//! ([`cell_least`]), and then all of it is; every other cell is the source to the bit, the pixels
//! of it that the key covers too. So the soft edge of a window shows no cell. A cell that an
//! edge of the frame cuts is whole where the key is 1 over what of it is inside the frame.
//! Without a key the whole frame is keyed, and Whole Cells Only changes nothing.
//!
//! # On the card
//!
//! A shader must not read every pixel of a cell for every pixel that it draws. It is to make
//! the colours of the cells first, in a texture of a texel for each cell ([`Grid::cells`] across
//! and down; [`cell_means`] is what it holds), and then draw each pixel from the texel of its
//! cell. Given `size` and `lead` as whole numbers (unsigned: `neighbourhood.wgsl` tells of the
//! compiler of Direct3D 12 and signed ones), with `textureLoad` throughout and no sampler:
//!
//! ```text
//! fn mosaic_cell(pixel: vec2<u32>) -> vec2<u32> {
//!     return (pixel * 256u + lead) / size;
//! }
//! // The first pixel of the cell `index` along an axis `length` long.
//! fn mosaic_edge(index: u32, size: u32, lead: u32, length: u32) -> u32 {
//!     let side = index * size;
//!     return min((max(side, lead) - lead + 255u) / 256u, length);
//! }
//! ```
//!
//! 1. Across, into a texture of `cells.x` by `H`: the texel `(i, y)` is the mean of the pixels
//!    from `mosaic_edge(i)` to `mosaic_edge(i + 1)` of the row `y`.
//! 2. Down, into a texture of `cells.x` by `cells.y`: the texel `(i, j)` is the mean of the
//!    texels `(i, y)` of the first, `y` from `mosaic_edge(j)` to `mosaic_edge(j + 1)`. Every row
//!    of a cell has as many pixels, so that the mean of the means of its rows is its mean. For
//!    Whole Cells Only, the least of the key is taken the same way, across and down
//!    ([`cell_leasts`]).
//! 3. Each pixel of the frame: `textureLoad` of the texel `mosaic_cell(pixel)`, and the mix by
//!    the key, with `select` for a key of 0 and of 1.
//!
//! A fragment of the first two reads a row or a column of one cell, the frame's width at most,
//! and all of them together read each pixel of the picture once.
//!
//! How close the card is to this reference:
//!
//! - Which pixels are of which cell: exactly, the places being whole numbers (of 32 bits, which
//!   hold `STEPS · p + lead` and `q · size` of any frame that a card takes).
//! - The colour of a cell: here it is the exact mean (summed in `f64`, and rounded to `f32`
//!   once), and the card sums in `f32`, some `(a + b) · 2^-24` of the greatest value of the cell
//!   off for a cell of `a` by `b` pixels (3e-5 of it at 256 by 256). Through two textures of
//!   32-bit floats (`Rgba32Float`, which a card renders to, and loads from without filtering)
//!   that is all: a test may hold a picture within 0 and 1 to 1e-4. Through the half floats of
//!   the neighbourhoods (`WORKING_FORMAT`) each of the two writes rounds to a half float, to the
//!   nearest (2^-11 of the value) or towards 0 (2^-10, as the RTX 4080 does under Direct3D 12,
//!   `grade/blur.rs`): to 2e-3 of the value, and a flat picture may come out a step or two of
//!   half floats lower.
//! - A pixel under a key of 0 is the source exactly either way, and under a key of 1 it is as
//!   close as the colour of its cell.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    reason = "places of pixels and counts of steps, far within the types that they are cast to"
)]

/// The steps that a pixel is divided into: the grid is laid out in whole numbers of them, so
/// that the card and the processor agree on the cell of every pixel (see the module).
pub const STEPS: u32 = 256;

/// Half a pixel in steps: how far the centre of a pixel is past its near side.
const HALF: u32 = STEPS / 2;

/// The most pixels that a cell is across or down: its size in steps is then 2^24, the last
/// whole number that an `f32` of the parameters of an operation keeps apart from the next.
pub const MOST_CELL: u32 = 65_536;

/// The range of a setting, and what it is at first.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    /// What a new mosaic has.
    pub default: f32,
    /// The lowest value.
    pub low: f32,
    /// The highest value.
    pub high: f32,
}

impl Bounds {
    /// Check that `value` is a number from `low` to `high`.
    fn check(self, what: &'static str, value: f32) -> Result<(), OutOfRange> {
        if (self.low..=self.high).contains(&value) {
            Ok(())
        } else {
            Err(OutOfRange { what })
        }
    }

    /// `value` as the grid takes it, checked or not: the nearest to it within the range, and the
    /// default where it is no number.
    fn within(self, value: f32) -> f32 {
        if value.is_nan() {
            self.default
        } else {
            value.clamp(self.low, self.high)
        }
    }
}

/// Pixel Frequency (p. 3465): how many cells lie across the frame. The manual gives neither a
/// range nor what it is at first. Ours: 32 at first (cells of 60 pixels in a frame 1920 wide, 32
/// by 18 whole ones in 16:9), to be set against `DaVinci`; from 1, a cell as wide as the frame,
/// to the 16384 pixels of the widest frame, beyond which a cell is a pixel anyway.
pub const FREQUENCY: Bounds = Bounds {
    default: 32.0,
    low: 1.0,
    high: 16_384.0,
};

/// Aspect (p. 3465): the width of a cell against its height. Ours: 1 at first, the squares of
/// the Square shape, and from a sixteenth to 16.
pub const ASPECT: Bounds = Bounds {
    default: 1.0,
    low: 0.0625,
    high: 16.0,
};

/// X Offset and Y Offset (p. 3465), in pixels. Ours: none at first, and as far either way as the
/// widest frame is wide (the grid is where it was after the size of a cell).
pub const OFFSET: Bounds = Bounds {
    default: 0.0,
    low: -16_384.0,
    high: 16_384.0,
};

/// The Cell Shape of a mosaic (p. 3465: "The options are Square, Hexagon, and Triangle").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CellShape {
    /// Squares, or rectangles where Aspect is not 1.
    #[default]
    Square,
    // Hexagon and Triangle are of the advanced tier and are not done: each needs its own cell of
    // a pixel, its own pixels of a cell (no rectangle) and so its own first pass on the card.
}

/// A setting that [`Settings::check`] refuses: no number, or one outside its range.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("the setting {what} is not a number within its range")]
pub struct OutOfRange {
    /// The setting, as a path: `mosaic.frequency`, `mosaic.aspect`, `mosaic.offset.x` or
    /// `mosaic.offset.y`.
    pub what: &'static str,
}

/// The settings of a mosaic (see the module).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// Pixel Frequency: how many cells lie across the width of the frame ([`FREQUENCY`]).
    pub frequency: f32,
    /// Cell Shape.
    pub shape: CellShape,
    /// Aspect: how many times as wide as high a cell is ([`ASPECT`]).
    pub aspect: f32,
    /// X Offset and Y Offset: how many pixels the grid is moved to the right and down
    /// ([`OFFSET`]).
    pub offset: [f32; 2],
    /// Whole Cells Only: a cell is shown only where the key covers all of it.
    pub whole_cells_only: bool,
}

impl Default for Settings {
    /// The manual shows no setting of a new Mosaic Blur. Ours, to be checked against `DaVinci`:
    /// 32 square cells across, no offset, and Whole Cells Only off.
    fn default() -> Self {
        Self {
            frequency: FREQUENCY.default,
            shape: CellShape::Square,
            aspect: ASPECT.default,
            offset: [OFFSET.default; 2],
            whole_cells_only: false,
        }
    }
}

impl Settings {
    /// Check that each setting is a number within its range.
    ///
    /// # Errors
    ///
    /// The first of Pixel Frequency, Aspect, X Offset and Y Offset that is not.
    pub fn check(&self) -> Result<(), OutOfRange> {
        FREQUENCY.check("mosaic.frequency", self.frequency)?;
        ASPECT.check("mosaic.aspect", self.aspect)?;
        OFFSET.check("mosaic.offset.x", self.offset[0])?;
        OFFSET.check("mosaic.offset.y", self.offset[1])
    }
}

/// The pixels of a cell that are inside the frame: the columns from `x0` up to `x1` and the rows
/// from `y0` up to `y1`, the first of each within and the second one past.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    /// The first column.
    pub x0: u32,
    /// The first row.
    pub y0: u32,
    /// One past the last column.
    pub x1: u32,
    /// One past the last row.
    pub y1: u32,
}

impl Rect {
    /// How many columns it has.
    #[must_use]
    pub fn width(self) -> u32 {
        self.x1.saturating_sub(self.x0)
    }

    /// How many rows it has.
    #[must_use]
    pub fn height(self) -> u32 {
        self.y1.saturating_sub(self.y0)
    }

    /// How many pixels it has.
    #[must_use]
    pub fn area(self) -> u64 {
        u64::from(self.width()) * u64::from(self.height())
    }

    /// Whether it has no pixel.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.area() == 0
    }

    /// Whether the pixel `(x, y)` is one of its own.
    #[must_use]
    pub fn contains(self, x: u32, y: u32) -> bool {
        (self.x0..self.x1).contains(&x) && (self.y0..self.y1).contains(&y)
    }
}

/// `pixels` in steps, to the nearest.
fn steps(pixels: f64) -> i64 {
    (pixels * f64::from(STEPS)).round() as i64
}

/// The grid of a mosaic over a frame, in whole numbers of steps (see the module): what the card
/// is given of the settings. `[across, down]` each.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Grid {
    /// The width and height of the frame, in pixels.
    pub frame: [u32; 2],
    /// The width and height of a cell, in steps: from [`STEPS`] to [`MOST_CELL`] times it.
    pub size: [u32; 2],
    /// How many steps the centre of the first pixel is past the side of cells at or before it:
    /// less than `size`.
    pub lead: [u32; 2],
}

impl Grid {
    /// The grid of `settings` over a frame of `width` by `height` pixels. Settings that
    /// [`Settings::check`] refuses are taken as the nearest that it allows, and one that is no
    /// number as it is at first, so that there is a grid whatever is given.
    #[must_use]
    pub fn new(width: u32, height: u32, settings: &Settings) -> Self {
        let frequency = f64::from(FREQUENCY.within(settings.frequency));
        let aspect = f64::from(ASPECT.within(settings.aspect));
        let most = f64::from(MOST_CELL);
        let across = (f64::from(width) / frequency).clamp(1.0, most);
        let down = (across / aspect).clamp(1.0, most);
        let frame = [width, height];
        let size = [across, down].map(steps);
        let lead = [0, 1].map(|axis: usize| {
            let offset = steps(f64::from(OFFSET.within(settings.offset[axis])));
            // A side of cells lies on the centre of the frame, moved by the offset.
            let anchor = i64::from(frame[axis]) * i64::from(HALF) + offset;
            (i64::from(HALF) - anchor).rem_euclid(size[axis])
        });
        Self {
            frame,
            size: size.map(|value| value as u32),
            lead: lead.map(|value| value as u32),
        }
    }

    /// The width and height of a cell, in pixels.
    #[must_use]
    pub fn cell_size(&self) -> [f32; 2] {
        self.size
            .map(|size| (f64::from(size) / f64::from(STEPS)) as f32)
    }

    /// How many pixels the frame has.
    fn pixels(&self) -> usize {
        self.frame[0] as usize * self.frame[1] as usize
    }

    /// The cell of the pixel `pixel` along `axis` (0 across, 1 down), counted from that of the
    /// first pixel.
    fn index(&self, axis: usize, pixel: u32) -> u32 {
        let place = u64::from(pixel) * u64::from(STEPS) + u64::from(self.lead[axis]);
        (place / u64::from(self.size[axis].max(1))) as u32
    }

    /// The first pixel along `axis` whose cell is `index` or one after it: the length of the
    /// frame where there is none.
    fn edge(&self, axis: usize, index: u32) -> u32 {
        let side = u64::from(index) * u64::from(self.size[axis]);
        let pixel = side
            .saturating_sub(u64::from(self.lead[axis]))
            .div_ceil(u64::from(STEPS));
        pixel.min(u64::from(self.frame[axis])) as u32
    }

    /// The cell that the pixel `(x, y)` falls in: how many cells across and down it is from the
    /// cell of the pixel at the left and the top, which is `[0, 0]`. It is the texel of the cell
    /// in the texture of the colours of cells.
    #[must_use]
    pub fn cell(&self, x: u32, y: u32) -> [u32; 2] {
        [self.index(0, x), self.index(1, y)]
    }

    /// The pixels of `cell` that are inside the frame; none of a cell beyond it.
    #[must_use]
    pub fn rect(&self, cell: [u32; 2]) -> Rect {
        Rect {
            x0: self.edge(0, cell[0]),
            y0: self.edge(1, cell[1]),
            x1: self.edge(0, cell[0].saturating_add(1)),
            y1: self.edge(1, cell[1].saturating_add(1)),
        }
    }

    /// How many cells across and down hold a pixel of the frame: the size of the texture of the
    /// colours of cells.
    #[must_use]
    pub fn cells(&self) -> [u32; 2] {
        [0, 1].map(|axis: usize| match self.frame[axis] {
            0 => 0,
            length => self.index(axis, length - 1).saturating_add(1),
        })
    }
}

/// A key as a mosaic takes it: within 0 and 1, and 0 where it is no number.
#[must_use]
pub fn level(key: f32) -> f32 {
    if key > 0.0 {
        key.min(1.0)
    } else {
        0.0
    }
}

/// A pixel under a key: `source` where the key is 0 and `cell`, the colour of its cell, where it
/// is 1, both to the bit, and `source + (cell − source) · key` between.
#[must_use]
pub fn mixed(source: [f32; 4], cell: [f32; 4], key: f32) -> [f32; 4] {
    let k = level(key);
    if k <= 0.0 {
        return source;
    }
    if k >= 1.0 {
        return cell;
    }
    std::array::from_fn(|i| source[i] + (cell[i] - source[i]) * k)
}

/// The colour of a cell: the mean of the pixels of `rect` in `src`, a picture `width` pixels
/// wide of premultiplied R, G, B and A, row after row. It is the exact mean, summed in `f64` and
/// rounded to `f32` once; of no pixel, it is nothing.
///
/// # Panics
///
/// Where `rect` reaches beyond the picture.
#[must_use]
pub fn cell_mean(src: &[[f32; 4]], width: u32, rect: Rect) -> [f32; 4] {
    if rect.is_empty() {
        return [0.0; 4];
    }
    let mut sum = [0.0_f64; 4];
    for y in rect.y0..rect.y1 {
        let row = y as usize * width as usize;
        for pixel in &src[row + rect.x0 as usize..row + rect.x1 as usize] {
            for (total, value) in sum.iter_mut().zip(pixel) {
                *total += f64::from(*value);
            }
        }
    }
    let count = rect.area() as f64;
    sum.map(|total| (total / count) as f32)
}

/// The least of `key` (as [`level`] takes it) over the pixels of `rect`, `key` a matte `width`
/// pixels wide, row after row: 1 where the key covers the whole cell. Of no pixel, it is 0.
///
/// # Panics
///
/// Where `rect` reaches beyond the matte.
#[must_use]
pub fn cell_least(key: &[f32], width: u32, rect: Rect) -> f32 {
    if rect.is_empty() {
        return 0.0;
    }
    let mut least = 1.0_f32;
    for y in rect.y0..rect.y1 {
        let row = y as usize * width as usize;
        for k in &key[row + rect.x0 as usize..row + rect.x1 as usize] {
            least = least.min(level(*k));
        }
    }
    least
}

/// What `of` makes of the pixels of each cell of `grid`, row of cells after row.
fn of_cells<T>(grid: &Grid, of: impl Fn(Rect) -> T) -> Vec<T> {
    let [across, down] = grid.cells();
    let mut all = Vec::with_capacity(across as usize * down as usize);
    for j in 0..down {
        for i in 0..across {
            all.push(of(grid.rect([i, j])));
        }
    }
    all
}

/// The colours of the cells of `grid` over `src` ([`cell_mean`] of each), row of cells after
/// row, [`Grid::cells`] across and down: the texture that the first pass of the card makes.
///
/// # Panics
///
/// Where `src` is not of the size of the frame of `grid`.
#[must_use]
pub fn cell_means(src: &[[f32; 4]], grid: &Grid) -> Vec<[f32; 4]> {
    assert_eq!(
        src.len(),
        grid.pixels(),
        "a picture of the size of the frame"
    );
    of_cells(grid, |rect| cell_mean(src, grid.frame[0], rect))
}

/// The least of `key` over each cell of `grid` ([`cell_least`] of each), row of cells after row:
/// what the first pass of the card makes of the key for Whole Cells Only.
///
/// # Panics
///
/// Where `key` is not of the size of the frame of `grid`.
#[must_use]
pub fn cell_leasts(key: &[f32], grid: &Grid) -> Vec<f32> {
    assert_eq!(key.len(), grid.pixels(), "a key of the size of the frame");
    of_cells(grid, |rect| cell_least(key, grid.frame[0], rect))
}

/// A frame through a mosaic: `src` a picture of `width` by `height` pixels of premultiplied R,
/// G, B and A, row after row, and `key` a matte of the same size, from 0 to 1, or none for the
/// whole frame. Each pixel is the colour of its cell ([`cell_mean`]) mixed with the source by
/// the key at the pixel ([`mixed`]), or, with Whole Cells Only, the colour of its cell where the
/// key is 1 all over the cell and the source elsewhere.
///
/// It is done as the card does it: the colours of the cells first ([`cell_means`]), and then
/// each pixel from the texel of its cell ([`Grid::cell`]).
///
/// # Panics
///
/// Where `src` or `key` is not of `width` by `height` pixels.
#[must_use]
pub fn mosaic(
    src: &[[f32; 4]],
    width: u32,
    height: u32,
    settings: &Settings,
    key: Option<&[f32]>,
) -> Vec<[f32; 4]> {
    let grid = Grid::new(width, height, settings);
    if let Some(key) = key {
        assert_eq!(key.len(), grid.pixels(), "a key of the size of the frame");
    }
    // The first pass: a texel for each cell.
    let means = cell_means(src, &grid);
    let leasts = key
        .filter(|_| settings.whole_cells_only)
        .map(|key| cell_leasts(key, &grid));
    // The second: each pixel of the frame, from the texel of its cell.
    let across = grid.cells()[0] as usize;
    let mut out = Vec::with_capacity(src.len());
    for y in 0..height {
        for x in 0..width {
            let at = y as usize * width as usize + x as usize;
            let [i, j] = grid.cell(x, y);
            let cell = j as usize * across + i as usize;
            let share = match (key, leasts.as_deref()) {
                (None, _) => 1.0,
                (Some(_), Some(leasts)) => {
                    if leasts[cell] >= 1.0 {
                        1.0
                    } else {
                        0.0
                    }
                }
                (Some(key), None) => key[at],
            };
            out.push(mixed(src[at], means[cell], share));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::float_cmp,
        reason = "means of numbers that floats keep exactly, and sources kept to the bit"
    )]

    use super::*;

    /// A picture no two neighbours of which are alike, of numbers that floats keep exactly.
    fn picture(width: u32, height: u32) -> Vec<[f32; 4]> {
        (0..height)
            .flat_map(|y| (0..width).map(move |x| (x, y)))
            .map(|(x, y)| {
                let v = ((x * 7 + y * 3) % 11) as f32 / 8.0;
                [v, v * 0.5, 2.0 - v, 1.0]
            })
            .collect()
    }

    /// The settings at first, but with `frequency` cells across.
    fn of(frequency: f32) -> Settings {
        Settings {
            frequency,
            ..Settings::default()
        }
    }

    /// The pixels along an axis at which a cell begins, but the first of the frame.
    fn sides(grid: &Grid, axis: usize) -> Vec<u32> {
        let cell = |pixel: u32| {
            if axis == 0 {
                grid.cell(pixel, 0)[0]
            } else {
                grid.cell(0, pixel)[1]
            }
        };
        (1..grid.frame[axis])
            .filter(|&pixel| cell(pixel) != cell(pixel - 1))
            .collect()
    }

    #[test]
    fn a_mosaic_is_at_first_of_32_square_cells_across_with_no_offset() {
        let settings = Settings::default();
        assert_eq!(settings.frequency, 32.0);
        assert_eq!(settings.shape, CellShape::Square);
        assert_eq!(settings.aspect, 1.0);
        assert_eq!(settings.offset, [0.0, 0.0]);
        assert!(!settings.whole_cells_only);
        assert_eq!(settings.check(), Ok(()));
    }

    #[test]
    fn pixel_frequency_is_how_many_cells_lie_across_the_frame() {
        let grid = Grid::new(1920, 1080, &of(32.0));
        assert_eq!(grid.cell_size(), [60.0, 60.0]);
        assert_eq!(grid.cells(), [32, 18]);
        assert_eq!(
            grid.rect([0, 0]),
            Rect {
                x0: 0,
                y0: 0,
                x1: 60,
                y1: 60
            }
        );
        assert_eq!(
            grid.rect([31, 17]),
            Rect {
                x0: 1860,
                y0: 1020,
                x1: 1920,
                y1: 1080
            }
        );
        // A cell beyond the frame has no pixel.
        assert!(grid.rect([32, 0]).is_empty());
        // 26 across 1920 pixels: cells of 73.85 pixels, which are of 73 or of 74.
        let grid = Grid::new(1920, 1080, &of(26.0));
        assert_eq!(grid.cells()[0], 26);
        let mut widths = 0;
        for i in 0..26 {
            let width = grid.rect([i, 0]).width();
            assert!(width == 73 || width == 74, "{i}: {width}");
            widths += width;
        }
        assert_eq!(widths, 1920);
    }

    #[test]
    fn the_centre_of_the_frame_is_a_side_of_cells() {
        let grid = Grid::new(1920, 1080, &of(32.0));
        assert_eq!(grid.cell(959, 539), [15, 8]);
        assert_eq!(grid.cell(960, 540), [16, 9]);
        // Three cells across twelve pixels are of four, and half of one shows at either edge.
        let grid = Grid::new(12, 12, &of(3.0));
        let across: Vec<(u32, u32)> = (0..grid.cells()[0])
            .map(|i| grid.rect([i, 0]))
            .map(|rect| (rect.x0, rect.x1))
            .collect();
        assert_eq!(across, [(0, 2), (2, 6), (6, 10), (10, 12)]);
        assert_eq!(grid.cell(5, 5), [1, 1]);
        assert_eq!(grid.cell(6, 6), [2, 2]);
        // The centre of a frame five pixels wide is the centre of its pixel 2, which is of the
        // later cell.
        let grid = Grid::new(5, 1, &of(2.5));
        let across: Vec<(u32, u32)> = (0..grid.cells()[0])
            .map(|i| grid.rect([i, 0]))
            .map(|rect| (rect.x0, rect.x1))
            .collect();
        assert_eq!(across, [(0, 2), (2, 4), (4, 5)]);
    }

    #[test]
    fn a_flat_picture_is_unchanged() {
        let flat = vec![[0.1, 0.45, 0.8, 1.0]; 13 * 7];
        let moved = Settings {
            frequency: 4.5,
            aspect: 2.0,
            offset: [1.5, -2.25],
            ..Settings::default()
        };
        for settings in [of(3.0), of(1.0), moved] {
            assert_eq!(mosaic(&flat, 13, 7, &settings, None), flat, "{settings:?}");
        }
    }

    #[test]
    fn every_pixel_of_a_cell_is_the_mean_of_the_cell() {
        // Four cells of two pixels by two.
        let src: Vec<[f32; 4]> = (0..16_u8)
            .map(|number| {
                let value = f32::from(number);
                [value, value * 0.5, 16.0 - value, 1.0]
            })
            .collect();
        let out = mosaic(&src, 4, 4, &of(2.0), None);
        let means = [
            [2.5, 1.25, 13.5, 1.0],
            [4.5, 2.25, 11.5, 1.0],
            [10.5, 5.25, 5.5, 1.0],
            [12.5, 6.25, 3.5, 1.0],
        ];
        for (at, pixel) in out.iter().enumerate() {
            let (x, y) = (at % 4, at / 4);
            assert_eq!(*pixel, means[(y / 2) * 2 + x / 2], "{x}, {y}");
        }
        // Cells of a size that does not divide the frame, moved and twice as wide as high.
        let (width, height) = (13, 7);
        let src = picture(width, height);
        let settings = Settings {
            frequency: 3.0,
            aspect: 2.0,
            offset: [1.5, -2.25],
            ..Settings::default()
        };
        let grid = Grid::new(width, height, &settings);
        let out = mosaic(&src, width, height, &settings, None);
        assert_ne!(out, src);
        for y in 0..height {
            for x in 0..width {
                let rect = grid.rect(grid.cell(x, y));
                assert!(rect.contains(x, y), "{x}, {y}: {rect:?}");
                let mean = cell_mean(&src, width, rect);
                assert_eq!(out[(y * width + x) as usize], mean, "{x}, {y}");
            }
        }
    }

    #[test]
    fn a_cell_at_an_edge_is_the_mean_of_what_of_it_is_inside_the_frame() {
        // Twelve pixels across in cells of four from the centre: 0 and 1, 2 to 5, 6 to 9, and
        // 10 and 11. Red is the column.
        let src: Vec<[f32; 4]> = (0..4)
            .flat_map(|_| (0..12_u8).map(|x| [f32::from(x), 0.0, 0.0, 1.0]))
            .collect();
        let out = mosaic(&src, 12, 4, &of(3.0), None);
        assert_eq!(out[0][0], 0.5);
        assert_eq!(out[3][0], 3.5);
        assert_eq!(out[12 + 6][0], 7.5);
        assert_eq!(out[11][0], 10.5);
    }

    #[test]
    fn cells_tile_the_frame_with_no_gap_or_overlap_whatever_their_size() {
        let offsets = [[0.0, 0.0], [1.5, -2.25], [100.0, -100.0]];
        for (width, height, frequency, aspect) in [
            (13, 7, 3.0, 1.0),
            (10, 10, 4.0, 1.0),
            (17, 5, 6.5, 2.0),
            (16, 9, 5.0, 0.5),
            (1, 1, 1.0, 1.0),
            (5, 3, 32.0, 1.0),
        ] {
            for offset in offsets {
                let settings = Settings {
                    frequency,
                    aspect,
                    offset,
                    ..Settings::default()
                };
                let grid = Grid::new(width, height, &settings);
                let [across, down] = grid.cells();
                let mut held = vec![0_u32; (width * height) as usize];
                for j in 0..down {
                    for i in 0..across {
                        let rect = grid.rect([i, j]);
                        assert!(!rect.is_empty(), "{settings:?}: {rect:?}");
                        for y in rect.y0..rect.y1 {
                            for x in rect.x0..rect.x1 {
                                assert_eq!(grid.cell(x, y), [i, j], "{settings:?}");
                                held[(y * width + x) as usize] += 1;
                            }
                        }
                    }
                }
                assert!(held.iter().all(|&cells| cells == 1), "{settings:?}");
            }
        }
    }

    #[test]
    fn the_first_pass_makes_a_texel_of_each_cell_and_the_second_reads_it() {
        let src = picture(13, 7);
        let grid = Grid::new(13, 7, &of(3.0));
        let [across, down] = grid.cells();
        let means = cell_means(&src, &grid);
        assert_eq!(means.len(), (across * down) as usize);
        let out = mosaic(&src, 13, 7, &of(3.0), None);
        for y in 0..7 {
            for x in 0..13 {
                let [i, j] = grid.cell(x, y);
                let texel = means[(j * across + i) as usize];
                assert_eq!(out[(y * 13 + x) as usize], texel, "{x}, {y}");
            }
        }
    }

    #[test]
    fn an_offset_moves_the_grid_by_that_many_pixels_and_wraps_by_a_cell() {
        // Cells of six pixels in a frame of 24 by 16.
        let moved = |x: f32, y: f32| {
            let settings = Settings {
                offset: [x, y],
                ..of(4.0)
            };
            Grid::new(24, 16, &settings)
        };
        let still = moved(0.0, 0.0);
        assert_eq!(sides(&still, 0), [6, 12, 18]);
        assert_eq!(sides(&still, 1), [2, 8, 14]);
        // To the right and down.
        let grid = moved(2.0, 1.0);
        assert_eq!(sides(&grid, 0), [2, 8, 14, 20]);
        assert_eq!(sides(&grid, 1), [3, 9, 15]);
        // To the left and up.
        let grid = moved(-1.0, -1.0);
        assert_eq!(sides(&grid, 0), [5, 11, 17, 23]);
        assert_eq!(sides(&grid, 1), [1, 7, 13]);
        // By a part of a pixel, a side passes a pixel once it is past the centre of the pixel.
        assert_eq!(sides(&moved(0.5, 0.0), 0), [6, 12, 18]);
        assert_eq!(sides(&moved(0.75, 0.0), 0), [1, 7, 13, 19]);
        // By a cell, the grid is where it was.
        assert_eq!(moved(6.0, 6.0), still);
        assert_eq!(moved(-6.0, 12.0), still);
        assert_eq!(moved(8.0, -5.0), moved(2.0, 1.0));
        let src = picture(24, 16);
        let wrapped = Settings {
            offset: [6.0, -6.0],
            ..of(4.0)
        };
        assert_eq!(
            mosaic(&src, 24, 16, &wrapped, None),
            mosaic(&src, 24, 16, &of(4.0), None)
        );
    }

    #[test]
    fn aspect_makes_cells_that_many_times_as_wide_as_high() {
        let grid = |aspect: f32| {
            let settings = Settings { aspect, ..of(4.0) };
            Grid::new(32, 32, &settings)
        };
        let square = grid(1.0);
        assert_eq!(square.cell_size(), [8.0, 8.0]);
        assert_eq!(square.cells(), [4, 4]);
        let wide = grid(2.0);
        assert_eq!(wide.cell_size(), [8.0, 4.0]);
        assert_eq!(wide.cells(), [4, 8]);
        let rect = wide.rect([1, 1]);
        assert_eq!((rect.width(), rect.height()), (8, 4));
        assert_eq!((rect.x0, rect.y0), (8, 4));
        let tall = grid(0.5);
        assert_eq!(tall.cell_size(), [8.0, 16.0]);
        assert_eq!(tall.cells(), [4, 2]);
        let rect = tall.rect([3, 1]);
        assert_eq!((rect.width(), rect.height()), (8, 16));
        assert_eq!((rect.x1, rect.y1), (32, 32));
    }

    #[test]
    fn a_frequency_so_high_that_a_cell_is_a_pixel_changes_nothing() {
        let src = picture(9, 5);
        for frequency in [9.0, 100.0, FREQUENCY.high] {
            for offset in [[0.0, 0.0], [0.3, -0.6]] {
                for aspect in [1.0, 2.0] {
                    let settings = Settings {
                        frequency,
                        aspect,
                        offset,
                        ..Settings::default()
                    };
                    let grid = Grid::new(9, 5, &settings);
                    assert_eq!(grid.cell_size(), [1.0, 1.0], "{settings:?}");
                    assert_eq!(grid.cells(), [9, 5], "{settings:?}");
                    assert_eq!(mosaic(&src, 9, 5, &settings, None), src, "{settings:?}");
                }
            }
        }
    }

    #[test]
    fn a_key_of_0_leaves_the_source_and_a_key_of_1_gives_the_mosaic() {
        let src = picture(13, 7);
        let settings = of(3.0);
        let whole = mosaic(&src, 13, 7, &settings, None);
        assert_ne!(whole, src);
        let none = vec![0.0_f32; src.len()];
        assert_eq!(mosaic(&src, 13, 7, &settings, Some(none.as_slice())), src);
        let all = vec![1.0_f32; src.len()];
        assert_eq!(mosaic(&src, 13, 7, &settings, Some(all.as_slice())), whole);
        // Between, each pixel is as far from its source to the colour of its cell.
        let half = vec![0.5_f32; src.len()];
        let out = mosaic(&src, 13, 7, &settings, Some(half.as_slice()));
        for ((got, source), cell) in out.iter().zip(&src).zip(&whole) {
            let want: [f32; 4] = std::array::from_fn(|i| source[i] + (cell[i] - source[i]) * 0.5);
            assert_eq!(*got, want);
        }
        // A key beyond 0 and 1 is taken within them, and one that is no number as 0.
        let mut key = none;
        key[0] = -3.0;
        key[1] = 2.0;
        key[2] = f32::NAN;
        let out = mosaic(&src, 13, 7, &settings, Some(key.as_slice()));
        assert_eq!(out[0], src[0]);
        assert_eq!(out[1], whole[1]);
        assert_eq!(out[2..], src[2..]);
        assert_eq!(level(-3.0), 0.0);
        assert_eq!(level(0.25), 0.25);
        assert_eq!(level(2.0), 1.0);
        assert_eq!(level(f32::NAN), 0.0);
    }

    #[test]
    fn whole_cells_only_leaves_a_partly_covered_cell_alone() {
        // Four cells of four pixels by two.
        let (width, height) = (8, 4);
        let src = picture(width, height);
        let at = |x: u32, y: u32| (y * width + x) as usize;
        let mut key = vec![0.0_f32; src.len()];
        // The cell at the left of the top is covered; that at its right, but for a pixel; that
        // below it, not quite anywhere; and the last, nowhere.
        for y in 0..2 {
            for x in 0..8 {
                key[at(x, y)] = 1.0;
            }
        }
        key[at(7, 1)] = 0.5;
        for y in 2..4 {
            for x in 0..4 {
                key[at(x, y)] = 0.999;
            }
        }
        let grid = Grid::new(width, height, &of(2.0));
        assert_eq!(cell_leasts(&key, &grid), [1.0, 0.5, 0.999, 0.0]);
        let whole = mosaic(&src, width, height, &of(2.0), None);
        let settings = Settings {
            whole_cells_only: true,
            ..of(2.0)
        };
        let out = mosaic(&src, width, height, &settings, Some(key.as_slice()));
        for y in 0..height {
            for x in 0..width {
                let want = if x < 4 && y < 2 {
                    whole[at(x, y)]
                } else {
                    src[at(x, y)]
                };
                assert_eq!(out[at(x, y)], want, "{x}, {y}");
            }
        }
        assert_ne!(out[at(0, 0)], src[at(0, 0)]);
        // Without it, each pixel is mixed by its own key.
        let soft = mosaic(&src, width, height, &of(2.0), Some(key.as_slice()));
        assert_eq!(soft[at(4, 0)], whole[at(4, 0)]);
        let mixed_half = mixed(src[at(7, 1)], whole[at(7, 1)], 0.5);
        assert_eq!(soft[at(7, 1)], mixed_half);
        assert_ne!(soft[at(0, 2)], src[at(0, 2)]);
        assert_eq!(soft[at(7, 3)], src[at(7, 3)]);
        // A cell that the edge of the frame cuts is whole where the key covers what there is of
        // it, and with no key every cell is whole.
        let clipped = Settings {
            offset: [2.0, 0.0],
            ..settings
        };
        let all = vec![1.0_f32; src.len()];
        assert_eq!(Grid::new(width, height, &clipped).cells(), [3, 2]);
        assert_eq!(
            mosaic(&src, width, height, &clipped, Some(all.as_slice())),
            mosaic(&src, width, height, &clipped, None)
        );
        assert_eq!(mosaic(&src, width, height, &settings, None), whole);
    }

    #[test]
    fn a_pixel_that_is_no_number_stays_within_its_cell() {
        // Four cells of four pixels by two; the pixel 5 of the top row is in the second.
        let (width, height) = (8, 4);
        let clean = picture(width, height);
        let mut src = clean.clone();
        src[5][0] = f32::NAN;
        let settings = of(2.0);
        let want = mosaic(&clean, width, height, &settings, None);
        let out = mosaic(&src, width, height, &settings, None);
        for (at, (got, want)) in out.iter().zip(&want).enumerate() {
            let (x, y) = (at % 8, at / 8);
            if x >= 4 && y < 2 {
                assert!(got[0].is_nan(), "{x}, {y}");
                assert_eq!(got[1..], want[1..], "{x}, {y}");
            } else {
                assert_eq!(got, want, "{x}, {y}");
            }
        }
        // Under a key of 0 it is where it was, and nowhere else.
        let none = vec![0.0_f32; src.len()];
        let out = mosaic(&src, width, height, &settings, Some(none.as_slice()));
        for (at, (got, source)) in out.iter().zip(&clean).enumerate() {
            if at == 5 {
                assert!(got[0].is_nan());
                assert_eq!(got[1..], source[1..]);
            } else {
                assert_eq!(got, source, "{at}");
            }
        }
    }

    #[test]
    fn check_refuses_each_setting_that_is_out_of_its_range() {
        let refused = |settings: Settings, what: &'static str| {
            assert_eq!(settings.check(), Err(OutOfRange { what }), "{settings:?}");
        };
        for frequency in [0.0, 0.99, 16_385.0, -32.0, f32::NAN, f32::INFINITY] {
            refused(of(frequency), "mosaic.frequency");
        }
        for aspect in [0.0, 0.06, 16.5, -1.0, f32::NAN, f32::INFINITY] {
            let settings = Settings {
                aspect,
                ..Settings::default()
            };
            refused(settings, "mosaic.aspect");
        }
        for offset in [16_385.0, -16_385.0, f32::NAN, f32::NEG_INFINITY] {
            let across = Settings {
                offset: [offset, 0.0],
                ..Settings::default()
            };
            refused(across, "mosaic.offset.x");
            let down = Settings {
                offset: [0.0, offset],
                ..Settings::default()
            };
            refused(down, "mosaic.offset.y");
        }
        // The first that is out of its range is the one named.
        let both = Settings {
            aspect: 0.0,
            offset: [f32::NAN, f32::NAN],
            ..Settings::default()
        };
        refused(both, "mosaic.aspect");
        // The ends of each range are within it.
        let least = Settings {
            frequency: FREQUENCY.low,
            aspect: ASPECT.low,
            offset: [OFFSET.low, OFFSET.high],
            ..Settings::default()
        };
        assert_eq!(least.check(), Ok(()));
        let most = Settings {
            frequency: FREQUENCY.high,
            aspect: ASPECT.high,
            offset: [OFFSET.high, OFFSET.low],
            ..Settings::default()
        };
        assert_eq!(most.check(), Ok(()));
        let error = OutOfRange {
            what: "mosaic.aspect",
        };
        assert_eq!(
            error.to_string(),
            "the setting mosaic.aspect is not a number within its range"
        );
    }

    #[test]
    fn settings_out_of_their_ranges_are_taken_as_the_nearest_within_them() {
        let grid = |settings: Settings| Grid::new(64, 64, &settings);
        assert_eq!(grid(of(f32::NAN)), grid(Settings::default()));
        assert_eq!(grid(of(0.0)), grid(of(FREQUENCY.low)));
        assert_eq!(grid(of(f32::INFINITY)), grid(of(FREQUENCY.high)));
        let flat = Settings {
            aspect: 0.0,
            ..of(8.0)
        };
        let flattest = Settings {
            aspect: ASPECT.low,
            ..of(8.0)
        };
        assert_eq!(grid(flat), grid(flattest));
        let far = Settings {
            offset: [f32::NAN, f32::INFINITY],
            ..of(8.0)
        };
        let furthest = Settings {
            offset: [0.0, OFFSET.high],
            ..of(8.0)
        };
        assert_eq!(grid(far), grid(furthest));
        // A frame of no pixel has no cell across, and no picture.
        assert_eq!(Grid::new(0, 5, &of(4.0)).cells(), [0, 5]);
        assert_eq!(
            mosaic(&[], 0, 5, &Settings::default(), None),
            Vec::<[f32; 4]>::new()
        );
    }
}
