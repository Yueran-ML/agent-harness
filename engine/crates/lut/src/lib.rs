//! Look-up tables of colour (LUTs): read from `.cube` files, evaluated on the processor as the
//! reference, and laid out for the card with the WGSL that reads them.
//!
//! # What a LUT is here
//!
//! A [`Lut`] has a one-dimensional table ([`Lut1d`], three curves, one for each of R, G and B),
//! a three-dimensional one ([`Lut3d`], a cube of RGB), or both: the 1D table is then the
//! *shaper* that is done first, as `DaVinci Resolve` reads a `.cube` with both (Reference Manual
//! of Resolve 20, p. 3407), and as its project LUTs put 1D before 3D (pp. 151–153). Each table
//! has its own [`Domain`], the input that its first and last points stand for.
//!
//! Tables are kept as `f32`, the cube with the red index changing fastest, as a `.cube` lists
//! it (index `r + n·g + n²·b`); that is also the order of the texels of a 3D texture whose
//! width is red, so the cube goes to the card as it is.
//!
//! # Interpolation and the domain
//!
//! - 1D: linear between the two points about the input.
//! - 3D: [`Interpolation::Tetrahedral`] (the cell cut into six tetrahedra along its grey
//!   diagonal, four points weighed), which the Adobe specification asks for (§8) and Resolve
//!   offers as the better (p. 152); or [`Interpolation::Trilinear`] (all eight points), which is
//!   Resolve's default for the sake of older grades and other programs (p. 152).
//! - **Input beyond the domain is clamped to it**, so the LUT gives what it gives at its edge:
//!   Resolve says that "LUTs clip image detail that goes outside of the numeric range they're
//!   designed to handle" (p. 225), and that applying a LUT is an operation where "clipping will
//!   occur" (p. 227); the example of mixed domains in the Adobe specification (A.3, "y = x for
//!   x <= 1" with a domain to 3) reads the same way. A NaN is taken as the bottom of the domain.
//!   The output is not clamped: a table may hold any finite values, above 1 and below 0.
//!
//! The functions on the processor ([`Lut1d::eval`], [`Lut3d::tetrahedral`],
//! [`Lut3d::trilinear`], [`Lut::eval`]) compute in `f64` and are the reference that the WGSL of
//! [`gpu`] is held to.
//!
//! # Where this crate stands
//!
//! It is pure: it reads files and makes numbers and text, and depends on no card and on no
//! other crate of the engine. The compositor (`columbina-compose`) depends on it, not the
//! other way: [`gpu::GpuLut`] is the plain data of an operation of colour (the WGSL of its body,
//! its parameters, and the bytes of its textures), which compose uploads and wraps in its own
//! `ColorOp` with [`gpu::WGSL`] as the library. So the crate has no cycle with compose, and it
//! can be tested and used (the scopes, an agent, an export of LUTs) without a card.
//!
//! # Files
//!
//! [`parse_cube`] reads the `.cube` of the Adobe specification (Cube LUT Specification 1.0,
//! 2013) and the dialect of Resolve (`LUT_1D_INPUT_RANGE`, `LUT_3D_INPUT_RANGE`, a 1D shaper and
//! a cube in one file); see [`cube`] for the rules. [`Lut::to_cube`] writes one back.
//! `.3dl` (Autodesk/Lustre) is not read: its files do not say the depth of their integers, and
//! readers guess it from the largest value; it is left for when a file of it is met.
//!
//! [`cache::LutCache`] keeps what was read, by the digest of the file, so that every node that
//! names the same LUT reads it once.

pub mod cache;
pub mod cube;
pub mod gpu;
pub mod half;

pub use cache::{LoadError, Loaded, LutCache, Roots};
pub use cube::{parse_cube, ErrorKind, ParseError};

/// The most points of a 1D table (Adobe §6.1).
pub const MAX_1D_SIZE: usize = 65536;
/// The most points along each edge of a cube (Adobe §7.1).
pub const MAX_3D_SIZE: usize = 256;
/// The fewest points of either.
pub const MIN_SIZE: usize = 2;

/// How a cube is read between its points.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Interpolation {
    /// Eight points of the cell, weighed in each axis in turn.
    Trilinear,
    /// Four points of the cell, those of the tetrahedron about the input.
    #[default]
    Tetrahedral,
}

/// The input that the first and the last points of a table stand for, in each of R, G and B.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Domain {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

impl Default for Domain {
    /// 0 to 1, as a file that says nothing has it (Adobe §5.6).
    fn default() -> Domain {
        Domain {
            min: [0.0; 3],
            max: [1.0; 3],
        }
    }
}

impl Domain {
    /// The same bounds in each channel, as `LUT_nD_INPUT_RANGE` gives them.
    #[must_use]
    pub fn uniform(min: f32, max: f32) -> Domain {
        Domain {
            min: [min; 3],
            max: [max; 3],
        }
    }

    /// Whether every bound is finite and every lower one is below its upper one.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        (0..3).all(|c| {
            self.min[c].is_finite() && self.max[c].is_finite() && self.min[c] < self.max[c]
        })
    }

    /// Whether it is the same in each channel.
    #[must_use]
    // Exactly the same, as a file says it.
    #[allow(clippy::float_cmp)]
    pub fn is_uniform(&self) -> bool {
        self.min.iter().all(|&m| m == self.min[0]) && self.max.iter().all(|&m| m == self.max[0])
    }

    /// Where `value` of channel `c` falls between the points of a table whose last index is
    /// `last`: from 0 to `last`, clamped, a NaN taken as 0.
    fn place(&self, c: usize, value: f32, last: usize) -> f64 {
        let min = f64::from(self.min[c]);
        let max = f64::from(self.max[c]);
        let t = (f64::from(value) - min) / (max - min);
        // Not `clamp`, which keeps a NaN: `max` first makes it 0.
        #[allow(clippy::manual_clamp)]
        let t = t.max(0.0).min(1.0);
        t * to_f64(last)
    }
}

/// What is wrong with a table that is made in code.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TableError {
    #[error("a table of {size} points is outside {min}..={max}")]
    Size { size: usize, min: usize, max: usize },
    #[error("a cube of {size} points on an edge has {size}³ rows, not {rows}")]
    Rows { size: usize, rows: usize },
    #[error("a value of the table is not finite")]
    NotFinite,
    #[error("the lower bound of the domain is not below the upper one, or is not finite")]
    Domain,
    #[error("a LUT has a 1D table, a cube, or both")]
    Empty,
}

fn check_rows(table: &[[f32; 3]]) -> Result<(), TableError> {
    if table.iter().flatten().all(|v| v.is_finite()) {
        Ok(())
    } else {
        Err(TableError::NotFinite)
    }
}

/// Three curves, one for each of R, G and B, of the same number of points, spaced evenly over
/// the domain.
#[derive(Clone, Debug, PartialEq)]
pub struct Lut1d {
    domain: Domain,
    table: Vec<[f32; 3]>,
}

impl Lut1d {
    /// A table of these points, first to last.
    ///
    /// # Errors
    /// The number of points is outside 2..=65536, a value is not finite, or the domain is not
    /// one.
    pub fn new(domain: Domain, table: Vec<[f32; 3]>) -> Result<Lut1d, TableError> {
        if !(MIN_SIZE..=MAX_1D_SIZE).contains(&table.len()) {
            return Err(TableError::Size {
                size: table.len(),
                min: MIN_SIZE,
                max: MAX_1D_SIZE,
            });
        }
        if !domain.is_valid() {
            return Err(TableError::Domain);
        }
        check_rows(&table)?;
        Ok(Lut1d { domain, table })
    }

    /// The table of `size` points of `f` over the domain, `f` taking the input of each channel
    /// at a point (the same in all three where the domain is uniform).
    ///
    /// # Errors
    /// As [`Lut1d::new`].
    pub fn from_fn(
        size: usize,
        domain: Domain,
        mut f: impl FnMut([f64; 3]) -> [f64; 3],
    ) -> Result<Lut1d, TableError> {
        let last = to_f64(size.max(2) - 1);
        let table = (0..size)
            .map(|i| {
                let t = to_f64(i) / last;
                let input = [0, 1, 2].map(|c| {
                    let (min, max) = (f64::from(domain.min[c]), f64::from(domain.max[c]));
                    min + (max - min) * t
                });
                f(input).map(to_f32)
            })
            .collect();
        Lut1d::new(domain, table)
    }

    #[must_use]
    pub fn domain(&self) -> Domain {
        self.domain
    }

    /// The points, first to last.
    #[must_use]
    pub fn table(&self) -> &[[f32; 3]] {
        &self.table
    }

    /// The number of points.
    #[must_use]
    pub fn size(&self) -> usize {
        self.table.len()
    }

    /// Each channel through its curve, linearly between points; input outside the domain
    /// clamped to it.
    #[must_use]
    pub fn eval(&self, rgb: [f32; 3]) -> [f32; 3] {
        let last = self.table.len() - 1;
        [0, 1, 2].map(|c| {
            let place = self.domain.place(c, rgb[c], last);
            let low = index(place, last);
            let fraction = place - to_f64(low);
            let below = f64::from(self.table[low][c]);
            let above = f64::from(self.table[low + 1][c]);
            to_f32(below + (above - below) * fraction)
        })
    }
}

/// A cube of `size` points on each edge, spaced evenly over the domain, of RGB at each.
#[derive(Clone, Debug, PartialEq)]
pub struct Lut3d {
    domain: Domain,
    size: usize,
    table: Vec<[f32; 3]>,
}

impl Lut3d {
    /// A cube of these rows, red changing fastest (index `r + n·g + n²·b`).
    ///
    /// # Errors
    /// The size is outside 2..=256, there are not size³ rows, a value is not finite, or the
    /// domain is not one.
    pub fn new(size: usize, domain: Domain, table: Vec<[f32; 3]>) -> Result<Lut3d, TableError> {
        if !(MIN_SIZE..=MAX_3D_SIZE).contains(&size) {
            return Err(TableError::Size {
                size,
                min: MIN_SIZE,
                max: MAX_3D_SIZE,
            });
        }
        if table.len() != size * size * size {
            return Err(TableError::Rows {
                size,
                rows: table.len(),
            });
        }
        if !domain.is_valid() {
            return Err(TableError::Domain);
        }
        check_rows(&table)?;
        Ok(Lut3d {
            domain,
            size,
            table,
        })
    }

    /// The cube of `size` points on an edge of `f` over the domain.
    ///
    /// # Errors
    /// As [`Lut3d::new`].
    pub fn from_fn(
        size: usize,
        domain: Domain,
        mut f: impl FnMut([f64; 3]) -> [f64; 3],
    ) -> Result<Lut3d, TableError> {
        if !(MIN_SIZE..=MAX_3D_SIZE).contains(&size) {
            return Err(TableError::Size {
                size,
                min: MIN_SIZE,
                max: MAX_3D_SIZE,
            });
        }
        let last = to_f64(size - 1);
        let at = |c: usize, i: usize| {
            let (min, max) = (f64::from(domain.min[c]), f64::from(domain.max[c]));
            min + (max - min) * to_f64(i) / last
        };
        let mut table = Vec::with_capacity(size * size * size);
        for b in 0..size {
            for g in 0..size {
                for r in 0..size {
                    table.push(f([at(0, r), at(1, g), at(2, b)]).map(to_f32));
                }
            }
        }
        Lut3d::new(size, domain, table)
    }

    #[must_use]
    pub fn domain(&self) -> Domain {
        self.domain
    }

    /// The points on an edge.
    #[must_use]
    pub fn size(&self) -> usize {
        self.size
    }

    /// The rows, red changing fastest.
    #[must_use]
    pub fn table(&self) -> &[[f32; 3]] {
        &self.table
    }

    /// The point at these indices.
    #[must_use]
    pub fn at(&self, r: usize, g: usize, b: usize) -> [f32; 3] {
        self.table[r + self.size * (g + self.size * b)]
    }

    /// The cube at `rgb`, read as `interpolation` says.
    #[must_use]
    pub fn eval(&self, rgb: [f32; 3], interpolation: Interpolation) -> [f32; 3] {
        match interpolation {
            Interpolation::Trilinear => self.trilinear(rgb),
            Interpolation::Tetrahedral => self.tetrahedral(rgb),
        }
    }

    /// The cell about `rgb`: the indices of its lowest corner, and where in it `rgb` is (0 to 1
    /// in each axis). The top of the domain is the top of the last cell, not a cell of its own.
    fn cell(&self, rgb: [f32; 3]) -> ([usize; 3], [f64; 3]) {
        let last = self.size - 1;
        let x = [0, 1, 2].map(|c| self.domain.place(c, rgb[c], last));
        let base = x.map(|x| index(x, last));
        let f = [0, 1, 2].map(|c| x[c] - to_f64(base[c]));
        (base, f)
    }

    fn corner(&self, base: [usize; 3], r: usize, g: usize, b: usize) -> [f64; 3] {
        self.at(base[0] + r, base[1] + g, base[2] + b)
            .map(f64::from)
    }

    /// Eight points of the cell, weighed along red, then green, then blue.
    #[must_use]
    pub fn trilinear(&self, rgb: [f32; 3]) -> [f32; 3] {
        let (base, [fr, fg, fb]) = self.cell(rgb);
        let lerp = |a: [f64; 3], b: [f64; 3], t: f64| [0, 1, 2].map(|c| a[c] + (b[c] - a[c]) * t);
        let c = |r, g, b| self.corner(base, r, g, b);
        let x00 = lerp(c(0, 0, 0), c(1, 0, 0), fr);
        let x10 = lerp(c(0, 1, 0), c(1, 1, 0), fr);
        let x01 = lerp(c(0, 0, 1), c(1, 0, 1), fr);
        let x11 = lerp(c(0, 1, 1), c(1, 1, 1), fr);
        let y0 = lerp(x00, x10, fg);
        let y1 = lerp(x01, x11, fg);
        lerp(y0, y1, fb).map(to_f32)
    }

    /// Four points of the cell: those of the tetrahedron about `rgb`, of the six that the cell
    /// is cut into by the planes r = g, g = b and r = b, all six sharing the grey diagonal
    /// from the lowest corner to the highest.
    #[must_use]
    pub fn tetrahedral(&self, rgb: [f32; 3]) -> [f32; 3] {
        let (base, [fr, fg, fb]) = self.cell(rgb);
        let c = |r, g, b| self.corner(base, r, g, b);
        let c000 = c(0, 0, 0);
        let c111 = c(1, 1, 1);
        // The weights of the lowest corner, of the two between, and of the highest, and the
        // two corners between, by the order of the fractions. The same branches, and the same
        // ties, as the WGSL.
        let (w, p, q) = if fr > fg {
            if fg > fb {
                ([1.0 - fr, fr - fg, fg - fb, fb], c(1, 0, 0), c(1, 1, 0))
            } else if fr > fb {
                ([1.0 - fr, fr - fb, fb - fg, fg], c(1, 0, 0), c(1, 0, 1))
            } else {
                ([1.0 - fb, fb - fr, fr - fg, fg], c(0, 0, 1), c(1, 0, 1))
            }
        } else if fb > fg {
            ([1.0 - fb, fb - fg, fg - fr, fr], c(0, 0, 1), c(0, 1, 1))
        } else if fb > fr {
            ([1.0 - fg, fg - fb, fb - fr, fr], c(0, 1, 0), c(0, 1, 1))
        } else {
            ([1.0 - fg, fg - fr, fr - fb, fb], c(0, 1, 0), c(1, 1, 0))
        };
        [0, 1, 2].map(|k| to_f32(w[0] * c000[k] + w[1] * p[k] + w[2] * q[k] + w[3] * c111[k]))
    }
}

/// A LUT: a 1D table, a cube, or a 1D shaper and then a cube.
#[derive(Clone, Debug, PartialEq)]
pub struct Lut {
    title: Option<String>,
    shaper: Option<Lut1d>,
    cube: Option<Lut3d>,
    ignored: Vec<Ignored>,
}

/// A keyword of a file that was read past, as Resolve reads past it: one this crate does not
/// know, or the input range of a table that the file does not have.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ignored {
    /// Its line, from 1.
    pub line: usize,
    pub keyword: String,
}

impl Lut {
    /// A LUT of these tables.
    ///
    /// # Errors
    /// It has neither table.
    pub fn new(
        title: Option<String>,
        shaper: Option<Lut1d>,
        cube: Option<Lut3d>,
    ) -> Result<Lut, TableError> {
        if shaper.is_none() && cube.is_none() {
            return Err(TableError::Empty);
        }
        Ok(Lut {
            title,
            shaper,
            cube,
            ignored: Vec::new(),
        })
    }

    /// The title of the file, if it has one.
    #[must_use]
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    /// The 1D table: alone, the whole LUT; with a cube, the shaper done before it.
    #[must_use]
    pub fn shaper(&self) -> Option<&Lut1d> {
        self.shaper.as_ref()
    }

    /// The cube.
    #[must_use]
    pub fn cube(&self) -> Option<&Lut3d> {
        self.cube.as_ref()
    }

    /// The keywords of its file that were read past.
    #[must_use]
    pub fn ignored(&self) -> &[Ignored] {
        &self.ignored
    }

    /// The bytes that its tables take.
    #[must_use]
    pub fn bytes(&self) -> usize {
        let rows = self.shaper.as_ref().map_or(0, |s| s.table.len())
            + self.cube.as_ref().map_or(0, |c| c.table.len());
        rows * std::mem::size_of::<[f32; 3]>()
    }

    /// `rgb` through the shaper, then the cube.
    #[must_use]
    pub fn eval(&self, rgb: [f32; 3], interpolation: Interpolation) -> [f32; 3] {
        let mut rgb = rgb;
        if let Some(shaper) = &self.shaper {
            rgb = shaper.eval(rgb);
        }
        if let Some(cube) = &self.cube {
            rgb = cube.eval(rgb, interpolation);
        }
        rgb
    }

    /// The same LUT with each value of its tables as the card keeps it in a texture of half
    /// floats ([`half::round`]): what the WGSL reads from an `Rgba16Float` texture, for a
    /// reference that the card is held to exactly.
    #[must_use]
    pub fn rounded_to_half(&self) -> Lut {
        let round = |table: &[[f32; 3]]| -> Vec<[f32; 3]> {
            table.iter().map(|row| row.map(half::round)).collect()
        };
        Lut {
            title: self.title.clone(),
            shaper: self.shaper.as_ref().map(|s| Lut1d {
                domain: s.domain,
                table: round(&s.table),
            }),
            cube: self.cube.as_ref().map(|c| Lut3d {
                domain: c.domain,
                size: c.size,
                table: round(&c.table),
            }),
            ignored: self.ignored.clone(),
        }
    }

    /// The `.cube` of it: the Adobe form for one table, with `DOMAIN_MIN`/`DOMAIN_MAX` where the
    /// domain is not 0 to 1; Resolve's for a shaper and a cube, with the domain of the shaper as
    /// `DOMAIN_*` (or `LUT_1D_INPUT_RANGE` where it is the same in each channel) and that of the
    /// cube as `LUT_3D_INPUT_RANGE`. Values are written as the shortest decimal that reads back
    /// as the same `f32`, so [`parse_cube`] gives back this LUT.
    ///
    /// # Errors
    /// It has a shaper and a cube whose domain is not the same in each channel, which a `.cube`
    /// cannot say.
    pub fn to_cube(&self) -> Result<String, TableError> {
        use std::fmt::Write as _;
        let mut out = String::new();
        if let Some(title) = &self.title {
            let title: String = title
                .chars()
                .map(|c| if c == '"' || c.is_control() { '\'' } else { c })
                .collect();
            let _ = writeln!(out, "TITLE \"{title}\"");
        }
        let domain_lines = |out: &mut String, domain: &Domain| {
            if *domain != Domain::default() {
                let [r, g, b] = domain.min;
                let _ = writeln!(out, "DOMAIN_MIN {r} {g} {b}");
                let [r, g, b] = domain.max;
                let _ = writeln!(out, "DOMAIN_MAX {r} {g} {b}");
            }
        };
        match (&self.shaper, &self.cube) {
            (Some(shaper), Some(cube)) => {
                let _ = writeln!(out, "LUT_1D_SIZE {}", shaper.size());
                if shaper.domain.is_uniform() {
                    if shaper.domain != Domain::default() {
                        let _ = writeln!(
                            out,
                            "LUT_1D_INPUT_RANGE {} {}",
                            shaper.domain.min[0], shaper.domain.max[0]
                        );
                    }
                } else {
                    domain_lines(&mut out, &shaper.domain);
                }
                let _ = writeln!(out, "LUT_3D_SIZE {}", cube.size);
                if !cube.domain.is_uniform() {
                    return Err(TableError::Domain);
                }
                if cube.domain != Domain::default() {
                    let _ = writeln!(
                        out,
                        "LUT_3D_INPUT_RANGE {} {}",
                        cube.domain.min[0], cube.domain.max[0]
                    );
                }
            }
            (Some(shaper), None) => {
                let _ = writeln!(out, "LUT_1D_SIZE {}", shaper.size());
                domain_lines(&mut out, &shaper.domain);
            }
            (None, Some(cube)) => {
                let _ = writeln!(out, "LUT_3D_SIZE {}", cube.size);
                domain_lines(&mut out, &cube.domain);
            }
            (None, None) => return Err(TableError::Empty),
        }
        let tables = self.shaper.iter().map(|s| &s.table[..]);
        let tables = tables.chain(self.cube.iter().map(|c| &c.table[..]));
        for table in tables {
            for [r, g, b] in table {
                let _ = writeln!(out, "{r} {g} {b}");
            }
        }
        Ok(out)
    }
}

/// The index of the lower point about `x` (0 to `last`), so that the top is in the last
/// interval rather than past it.
fn index(x: f64, last: usize) -> usize {
    // `x` is 0..=last, so the cast is exact and in range.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let i = x.floor() as usize;
    i.min(last - 1)
}

#[allow(clippy::cast_precision_loss)]
fn to_f64(n: usize) -> f64 {
    n as f64
}

#[allow(clippy::cast_possible_truncation)]
fn to_f32(x: f64) -> f32 {
    x as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_table_made_in_code_is_checked() {
        assert_eq!(
            Lut1d::new(Domain::default(), vec![[0.0; 3]]),
            Err(TableError::Size {
                size: 1,
                min: 2,
                max: 65536
            })
        );
        assert_eq!(
            Lut1d::new(Domain::uniform(1.0, 1.0), vec![[0.0; 3]; 2]),
            Err(TableError::Domain)
        );
        assert_eq!(
            Lut1d::new(Domain::default(), vec![[0.0, f32::NAN, 0.0]; 2]),
            Err(TableError::NotFinite)
        );
        assert_eq!(
            Lut3d::new(2, Domain::default(), vec![[0.0; 3]; 7]),
            Err(TableError::Rows { size: 2, rows: 7 })
        );
        assert!(matches!(
            Lut3d::new(257, Domain::default(), Vec::new()),
            Err(TableError::Size { size: 257, .. })
        ));
        assert_eq!(Lut::new(None, None, None), Err(TableError::Empty));
    }

    #[test]
    fn the_domain_is_clamped_and_a_nan_is_its_bottom() {
        let domain = Domain::uniform(-1.0, 3.0);
        assert!((domain.place(0, 1.0, 4) - 2.0).abs() < 1e-12);
        assert!((domain.place(0, -5.0, 4)).abs() < 1e-12);
        assert!((domain.place(0, 9.0, 4) - 4.0).abs() < 1e-12);
        assert!((domain.place(0, f32::NAN, 4)).abs() < 1e-12);
        assert_eq!(index(4.0, 4), 3);
        assert_eq!(index(0.0, 1), 0);
    }

    #[test]
    fn a_lut_of_both_tables_whose_cube_is_not_uniform_cannot_be_written() {
        let shaper = Lut1d::from_fn(4, Domain::default(), |x| x).unwrap();
        let cube = Lut3d::from_fn(
            2,
            Domain {
                min: [0.0; 3],
                max: [1.0, 2.0, 1.0],
            },
            |x| x,
        )
        .unwrap();
        let lut = Lut::new(None, Some(shaper), Some(cube)).unwrap();
        assert_eq!(lut.to_cube(), Err(TableError::Domain));
    }
}
