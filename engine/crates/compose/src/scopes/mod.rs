//! The video scopes of a frame, counted on the card: the waveform (luma, or R, G and B over one
//! another), the RGB parade, the vectorscope, the histogram and the CIE 1931 xy chromaticity
//! diagram, as the Color page of `DaVinci Resolve` has them (RM20 p3007–3022); and the figures
//! of a frame that the agent reads ([`Statistics`]).
//!
//! # What is measured
//!
//! A scope measures the frame of a [`crate::Target`] ([`crate::Target::texture`], of half floats):
//! after the output transform, before the view maps it to its display, so it measures the signal
//! that is exported. Its channels are R', G' and B' as signals from 0 to 1 (`DaVinci Resolve`'s
//! "data levels", p3008): a level of 1 is the code 1023 of 10 bits, whatever the range that the
//! file is written in. What the signal means is told by its [`Signal`]: the luma coefficients
//! that luma and chroma are made with (Rec.709 unless said), and the transfer, which the
//! chromaticity diagram makes the signal linear by and which the scale of nits is of.
//!
//! # How
//!
//! Each scope is two passes of compute shaders (`scopes.wgsl`):
//!
//! 1. **Counting** into a buffer of `u32`, at a resolution of the scope's own (512 columns by
//!    256 levels for a waveform, 256² for a vectorscope, say), and laid out as
//!    [`Scopes::read_counts`] says:
//!    - a waveform or a parade: a workgroup for each column, counting the pixels of the columns
//!      of the frame that fall in it into its own memory, by level, then writing them, so that no
//!      two workgroups add to one count;
//!    - a histogram: each workgroup counts into its own memory, then adds what it has to the
//!      whole by atomics, and keeps the highest count of each channel (for drawing);
//!    - a vectorscope, a chromaticity diagram: each thread counts a run of pixels down a column,
//!      adding a run of one bin with one atomic, so that a flat field does not make every pixel
//!      wait on one count.
//! 2. **Drawing** the counts into a picture of RGBA of 8 bits ([`Scope::picture`]): each bin as
//!    bright as its count is against a uniform spread, by a log or a gamma, made brighter by the
//!    look's `brightness` (`DaVinci Resolve`'s slider of each scope); then the graticule over it
//!    as brightly as the look's `graticule` says. The graticule is also given as lines and labels
//!    ([`Scope::graticule`]), for an interface to draw over the picture as vectors.
//!
//! # Exact counts
//!
//! Each channel is first made an integer code, a signal of 1 being 16384, rounded to even; from
//! there on, luma, chroma, levels, bins and the chromaticity (by a table of linear light and a
//! matrix of integers) are integer arithmetic, which the card does as the processor does. So the
//! counts of the card are those of the processor exactly (`reference.rs`), the tests hold them to
//! that, and a scope says the same of a frame wherever it is counted. Channels are held to −1…2
//! before luma and chroma are made (for sums within 32 bits), and levels to 0…1: a waveform or
//! histogram puts what is beyond in its top or bottom level, and the [`Statistics`] tell the true
//! extremes.
//!
//! # Fewer pixels
//!
//! A scope may count one pixel in two or four across and down ([`Sampling`]): a quarter or a
//! sixteenth of the pixels, taken as they are (not averaged), with no copy of the frame. Every
//! pixel counted is one of the frame, so no colour is made up and the spread of colours is
//! not narrowed (as averaging would, like the Low Pass Filter of p3012); the shape of each scope
//! is an unbiased sample of the whole frame's, and the pictures are drawn against the count of
//! pixels counted, so they are as bright either way. What is lost: a detail narrower than the
//! step (a line of a pixel, a lone pixel beyond the others) may not be counted at all, so
//! extremes may be missed, and the counts are a quarter or a sixteenth as large, so faint parts
//! of a scope are noisier. Every pixel is cheap enough at 4K (each scope within 0.7 ms on an RTX
//! 4080 Laptop, against 2 ms asked for), so [`Sampling::auto`] counts every pixel up to DCI 4K
//! and steps only beyond it. The [`Statistics`] always read every pixel.
//!
//! # Statistics
//!
//! [`Scopes::statistics`] sums every pixel of a frame on the card, in an order that does not
//! change (so the same frame gives the same figures), and reads back a few figures: the least,
//! most and mean of R', G', B' and luma; the centroid, spread and mean saturation of chroma (of
//! the vectorscope, unzoomed); and a histogram of R, G, B and luma.

mod card;
pub mod graticule;
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "the reference of the card's counts, which the tests hold them to"
    )
)]
pub(crate) mod reference;
#[cfg(test)]
mod tests;

use columbina_model::{Gamma, Gamut};

pub use card::{Scope, ScopeImage, Scopes};
pub use graticule::{Graticule, Label, Line, Mark};

/// The luma coefficients of Rec.709 (red, green, blue).
pub const REC709_LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];
/// The luma coefficients of Rec.601.
pub const REC601_LUMA: [f32; 3] = [0.299, 0.587, 0.114];
/// The luma coefficients of Rec.2020.
pub const REC2020_LUMA: [f32; 3] = [0.2627, 0.6780, 0.0593];

/// What the signal of a frame is: the luma coefficients of its space (red, green, blue; green is
/// taken as what red and blue leave of 1), and its transfer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Signal {
    pub luma: [f32; 3],
    pub transfer: Gamma,
}

impl Default for Signal {
    /// Rec.709 of standard range: its luma, and a gamma of 2.4.
    fn default() -> Self {
        Signal {
            luma: REC709_LUMA,
            transfer: Gamma::Rec709,
        }
    }
}

/// How a scope is drawn from its counts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub mapping: Mapping,
    /// How bright the trace is, from 0 to 1 (0.5 as it is made): each step of 1/16 doubles it.
    pub brightness: f32,
    /// How bright the graticule is drawn into the picture, from 0 (not at all) to 1.
    pub graticule: f32,
}

impl Default for Look {
    fn default() -> Self {
        Look {
            mapping: Mapping::Log,
            brightness: 0.5,
            graticule: 0.5,
        }
    }
}

/// How a count is made a brightness: by its log, which shows the faint parts and the strong
/// apart, or by a gamma of 2.2, which keeps more of the difference among the strong.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mapping {
    #[default]
    Log,
    Gamma,
}

/// The units of the scale of a waveform, a parade or a histogram (p3009): codes of 10 bits, a
/// percent, or nits (on the curve of PQ, or of HLG where the transfer is HLG).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScaleStyle {
    #[default]
    TenBit,
    Percent,
    Nits,
}

/// What a waveform shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WaveformChannels {
    /// Luma alone.
    #[default]
    Luma,
    /// R, G and B over one another, white where they meet.
    Rgb,
}

/// A waveform: the signal up, where it is in the frame across.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Waveform {
    /// Columns, from 16 to 4096.
    pub width: u32,
    /// Levels, from 16 to 1024 (1024 being the codes of 10 bits).
    pub levels: u32,
    pub channels: WaveformChannels,
    pub scale: ScaleStyle,
}

impl Default for Waveform {
    fn default() -> Self {
        Waveform {
            width: 512,
            levels: 256,
            channels: WaveformChannels::default(),
            scale: ScaleStyle::default(),
        }
    }
}

/// A parade: the waveforms of R, G and B side by side.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Parade {
    /// Columns of each of the three, from 16 to 1365.
    pub width: u32,
    /// Levels, from 16 to 1024.
    pub levels: u32,
    pub scale: ScaleStyle,
}

impl Default for Parade {
    fn default() -> Self {
        Parade {
            width: 256,
            levels: 256,
            scale: ScaleStyle::default(),
        }
    }
}

/// Which pixels a vectorscope shows, by their luma (p3020).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tones {
    #[default]
    All,
    /// The shadows: below the low range.
    Low,
    /// The mid-tones: from the low range to the high.
    Mid,
    /// The highlights: above the high range.
    High,
}

/// Which targets of colour bars a vectorscope shows (p3010).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Targets {
    Off,
    #[default]
    Percent75,
    Percent100,
    Both,
}

/// A vectorscope: Cb across, Cr up; unzoomed, from −0.5 to 0.5 each, so that no colour of the
/// gamut falls outside.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vectorscope {
    /// Its side, even, from 16 to 1024.
    pub size: u32,
    /// From 1 to 8 (2 is `DaVinci Resolve`'s 2X Zoom), by sixteenths.
    pub zoom: f32,
    pub tones: Tones,
    /// The luma that parts the shadows from the mid-tones, and the mid-tones from the highlights
    /// (0.30 and 0.70 as they are made, p3020).
    pub low: f32,
    pub high: f32,
    pub targets: Targets,
    /// Whether the skin tone line is drawn.
    pub skin_tone: bool,
}

impl Default for Vectorscope {
    fn default() -> Self {
        Vectorscope {
            size: 256,
            zoom: 1.0,
            tones: Tones::All,
            low: 0.30,
            high: 0.70,
            targets: Targets::default(),
            skin_tone: true,
        }
    }
}

/// Which channels a histogram draws, each in a strip of its own, from the top.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HistogramChannels {
    #[default]
    Rgb,
    /// Luma, then R, G and B.
    Yrgb,
}

impl HistogramChannels {
    #[must_use]
    pub fn count(self) -> u32 {
        match self {
            HistogramChannels::Rgb => 3,
            HistogramChannels::Yrgb => 4,
        }
    }
}

/// A histogram: how many pixels have each level, the levels across. R, G, B and luma are all
/// counted; [`HistogramChannels`] says which are drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Histogram {
    /// Bins (and columns of the picture), from 16 to 1024.
    pub bins: u32,
    /// The height of the picture, from 16 to 4096, parted into strips.
    pub height: u32,
    pub channels: HistogramChannels,
    pub scale: ScaleStyle,
}

impl Default for Histogram {
    fn default() -> Self {
        Histogram {
            bins: 256,
            height: 256,
            channels: HistogramChannels::default(),
            scale: ScaleStyle::default(),
        }
    }
}

/// The chromaticities of the primaries and the white of a colour space, CIE 1931 xy.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Primaries {
    pub red: [f64; 2],
    pub green: [f64; 2],
    pub blue: [f64; 2],
    pub white: [f64; 2],
}

/// The white of D65.
const D65: [f64; 2] = [0.3127, 0.3290];

impl Primaries {
    pub const REC709: Primaries = Primaries {
        red: [0.64, 0.33],
        green: [0.30, 0.60],
        blue: [0.15, 0.06],
        white: D65,
    };
    pub const REC601_PAL: Primaries = Primaries {
        red: [0.64, 0.33],
        green: [0.29, 0.60],
        blue: [0.15, 0.06],
        white: D65,
    };
    pub const REC601_NTSC: Primaries = Primaries {
        red: [0.630, 0.340],
        green: [0.310, 0.595],
        blue: [0.155, 0.070],
        white: D65,
    };
    pub const REC2020: Primaries = Primaries {
        red: [0.708, 0.292],
        green: [0.170, 0.797],
        blue: [0.131, 0.046],
        white: D65,
    };
    pub const DCI_P3: Primaries = Primaries {
        red: [0.680, 0.320],
        green: [0.265, 0.690],
        blue: [0.150, 0.060],
        white: [0.314, 0.351],
    };
    pub const P3_D65: Primaries = Primaries {
        red: [0.680, 0.320],
        green: [0.265, 0.690],
        blue: [0.150, 0.060],
        white: D65,
    };

    /// The primaries of a gamut of media; none for a camera's own, which has none known.
    #[must_use]
    pub fn of(gamut: Gamut) -> Option<Primaries> {
        match gamut {
            Gamut::Rec709 => Some(Primaries::REC709),
            Gamut::Rec601Pal => Some(Primaries::REC601_PAL),
            Gamut::Rec601Ntsc => Some(Primaries::REC601_NTSC),
            Gamut::Rec2020 => Some(Primaries::REC2020),
            Gamut::DciP3 => Some(Primaries::DCI_P3),
            Gamut::P3D65 => Some(Primaries::P3_D65),
            Gamut::CameraNative => None,
        }
    }

    /// The matrix from linear R, G and B to XYZ (by rows), white making a Y of 1.
    #[must_use]
    pub fn to_xyz(&self) -> [[f64; 3]; 3] {
        let column = |[x, y]: [f64; 2]| [x / y, 1.0, (1.0 - x - y) / y];
        let columns = [self.red, self.green, self.blue].map(column);
        let primaries = [0, 1, 2].map(|row| columns.map(|each| each[row]));
        let scale = solve(&primaries, column(self.white));
        [0, 1, 2].map(|row| [0, 1, 2].map(|k| primaries[row][k] * scale[k]))
    }

    /// Whether these are chromaticities that make a matrix the scope can count with.
    fn check(&self) -> Result<(), String> {
        let points = [self.red, self.green, self.blue, self.white];
        if points.iter().flatten().any(|value| !value.is_finite())
            || points.iter().any(|[_, y]| y.abs() < 1e-6)
        {
            return Err("primaries must be finite, with y not 0".into());
        }
        let matrix = self.to_xyz();
        if matrix
            .iter()
            .flatten()
            .any(|value| !value.is_finite() || value.abs() >= reference::MATRIX_LARGEST)
        {
            return Err("primaries that make no matrix to XYZ of coefficients below 2".into());
        }
        Ok(())
    }
}

/// `x` such that `m x = v`, by Cramer's rule; NaN where `m` is singular.
fn solve(m: &[[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    let determinant = |m: &[[f64; 3]; 3]| {
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    };
    let whole = determinant(m);
    [0, 1, 2].map(|k| {
        let mut replaced = *m;
        for (row, value) in replaced.iter_mut().zip(v) {
            row[k] = value;
        }
        if whole.abs() < 1e-12 {
            f64::NAN
        } else {
            determinant(&replaced) / whole
        }
    })
}

/// A triangle of a gamut on the chromaticity diagram, with its name.
#[derive(Clone, Debug, PartialEq)]
pub struct Triangle {
    pub name: String,
    pub primaries: Primaries,
}

/// A chromaticity diagram, CIE 1931 xy: x from 0 to 0.8 across, y from 0 to 0.9 up.
#[derive(Clone, Debug, PartialEq)]
pub struct Chromaticity {
    /// From 16 to 1024 each; 8 to 9 keeps the units of x and y alike.
    pub width: u32,
    pub height: u32,
    /// The space of the signal: what its R, G and B are, made XYZ by.
    pub space: Primaries,
    /// The triangles drawn: the space's own first, as a rule, and another to compare with
    /// (p3017). They come from the catalogue of colour spaces.
    pub gamuts: Vec<Triangle>,
}

impl Default for Chromaticity {
    fn default() -> Self {
        Chromaticity {
            width: 512,
            height: 576,
            space: Primaries::REC709,
            gamuts: vec![Triangle {
                name: "Rec.709".into(),
                primaries: Primaries::REC709,
            }],
        }
    }
}

/// Which scope, with its settings.
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Waveform(Waveform),
    Parade(Parade),
    Vectorscope(Vectorscope),
    Histogram(Histogram),
    Chromaticity(Chromaticity),
}

impl Kind {
    /// The size of the scope's picture.
    #[must_use]
    pub fn picture_size(&self) -> (u32, u32) {
        match self {
            Kind::Waveform(waveform) => (waveform.width, waveform.levels),
            Kind::Parade(parade) => (3 * parade.width, parade.levels),
            Kind::Vectorscope(scope) => (scope.size, scope.size),
            Kind::Histogram(histogram) => {
                let strips = histogram.channels.count();
                (histogram.bins, histogram.height / strips * strips)
            }
            Kind::Chromaticity(diagram) => (diagram.width, diagram.height),
        }
    }

    /// How many counts the scope keeps.
    #[must_use]
    pub fn count_size(&self) -> usize {
        let size = |a: u32, b: u32| a as usize * b as usize;
        match self {
            Kind::Waveform(waveform) => {
                let planes = match waveform.channels {
                    WaveformChannels::Luma => 1,
                    WaveformChannels::Rgb => 3,
                };
                planes * size(waveform.width, waveform.levels)
            }
            Kind::Parade(parade) => 3 * size(parade.width, parade.levels),
            Kind::Vectorscope(scope) => size(scope.size, scope.size),
            Kind::Histogram(histogram) => 4 * histogram.bins as usize + 4,
            Kind::Chromaticity(diagram) => size(diagram.width, diagram.height),
        }
    }
}

/// A scope: which, what it measures, and how it is drawn.
#[derive(Clone, Debug, PartialEq)]
pub struct ScopeSettings {
    pub kind: Kind,
    pub signal: Signal,
    pub look: Look,
}

impl ScopeSettings {
    /// A scope of a signal of Rec.709, drawn as it is made.
    #[must_use]
    pub fn new(kind: Kind) -> ScopeSettings {
        ScopeSettings {
            kind,
            signal: Signal::default(),
            look: Look::default(),
        }
    }

    /// Whether the settings are of sizes and values the scope can count with.
    ///
    /// # Errors
    /// What is out of range, said.
    pub fn check(&self) -> Result<(), String> {
        check_signal(&self.signal)?;
        let look = &self.look;
        if !(0.0..=1.0).contains(&look.brightness) || !(0.0..=1.0).contains(&look.graticule) {
            return Err("brightness and graticule are from 0 to 1".into());
        }
        let within = |name: &str, value: u32, low: u32, high: u32| {
            if (low..=high).contains(&value) {
                Ok(())
            } else {
                Err(format!("{name} is {value}, not from {low} to {high}"))
            }
        };
        match &self.kind {
            Kind::Waveform(waveform) => {
                within("the width of a waveform", waveform.width, 16, 4096)?;
                within("the levels of a waveform", waveform.levels, 16, 1024)
            }
            Kind::Parade(parade) => {
                within("the width of a parade", parade.width, 16, 1365)?;
                within("the levels of a parade", parade.levels, 16, 1024)
            }
            Kind::Vectorscope(scope) => {
                within("the size of a vectorscope", scope.size, 16, 1024)?;
                if scope.size % 2 != 0 {
                    return Err("the size of a vectorscope is even".into());
                }
                if !(1.0..=8.0).contains(&scope.zoom) {
                    return Err("the zoom of a vectorscope is from 1 to 8".into());
                }
                if !(0.0..=1.0).contains(&scope.low)
                    || !(0.0..=1.0).contains(&scope.high)
                    || scope.low > scope.high
                {
                    return Err("the low and high ranges are from 0 to 1, low first".into());
                }
                Ok(())
            }
            Kind::Histogram(histogram) => {
                within("the bins of a histogram", histogram.bins, 16, 1024)?;
                within("the height of a histogram", histogram.height, 16, 4096)
            }
            Kind::Chromaticity(diagram) => {
                within(
                    "the width of a chromaticity diagram",
                    diagram.width,
                    16,
                    1024,
                )?;
                within(
                    "the height of a chromaticity diagram",
                    diagram.height,
                    16,
                    1024,
                )?;
                diagram.space.check()?;
                for triangle in &diagram.gamuts {
                    let points = [
                        triangle.primaries.red,
                        triangle.primaries.green,
                        triangle.primaries.blue,
                        triangle.primaries.white,
                    ];
                    if points.iter().flatten().any(|value| !value.is_finite()) {
                        return Err(format!("the gamut {} is not finite", triangle.name));
                    }
                }
                Ok(())
            }
        }
    }
}

fn check_signal(signal: &Signal) -> Result<(), String> {
    let [kr, kg, kb] = signal.luma;
    if !(kr > 0.0 && kr <= 0.5 && kb > 0.0 && kb <= 0.5 && kg.is_finite() && 1.0 - kr - kb >= 0.0) {
        return Err(format!(
            "luma coefficients {:?}: red and blue are from 0 to 0.5, and leave green some",
            signal.luma
        ));
    }
    Ok(())
}

/// How many pixels a scope counts: every one, or one in two or four across and down, as they
/// are (see the module's documentation for what it costs in accuracy).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sampling {
    #[default]
    Full,
    Half,
    Quarter,
}

impl Sampling {
    /// One pixel in how many, across and down.
    #[must_use]
    pub fn step(self) -> u32 {
        match self {
            Sampling::Full => 1,
            Sampling::Half => 2,
            Sampling::Quarter => 4,
        }
    }

    /// Every pixel of a frame of up to 4096×2160 (DCI 4K); one in two of a larger one, up to
    /// 8K, and one in four beyond.
    #[must_use]
    pub fn auto(width: u32, height: u32) -> Sampling {
        let pixels = u64::from(width) * u64::from(height);
        if pixels <= 4096 * 2160 {
            Sampling::Full
        } else if pixels <= 7680 * 4320 {
            Sampling::Half
        } else {
            Sampling::Quarter
        }
    }
}

/// The least, most and mean of a channel over a frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Channel {
    pub min: f32,
    pub max: f32,
    pub mean: f64,
}

/// The chroma of a frame, in Cb and Cr (from −0.5 to 0.5 within the gamut), as the
/// vectorscope shows it unzoomed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Chroma {
    /// The mean of Cb and of Cr: where the trace is centred; off the centre, a cast.
    pub centroid: [f64; 2],
    /// The standard deviation of Cb and of Cr about the centroid.
    pub spread: [f64; 2],
    /// The root mean square distance from the centroid.
    pub radius: f64,
    /// The mean distance from the centre: how saturated the frame is, on the whole.
    pub saturation: f64,
    /// The angle of the centroid, in degrees counterclockwise from Cb, from 0 to 360 (the skin
    /// tone line is at [`graticule::SKIN_TONE_DEGREES`]).
    pub hue: f64,
}

impl Chroma {
    /// Of the means of Cb and Cr, the means of their squares, and the mean length.
    fn of(means: [f64; 2], squares: [f64; 2], saturation: f64) -> Chroma {
        let variance = [0, 1].map(|axis| (squares[axis] - means[axis] * means[axis]).max(0.0));
        Chroma {
            centroid: means,
            spread: variance.map(f64::sqrt),
            radius: (variance[0] + variance[1]).sqrt(),
            saturation,
            hue: means[1].atan2(means[0]).to_degrees().rem_euclid(360.0),
        }
    }
}

/// A histogram as counted: `bins` levels each of R, G, B and luma (in that order, see
/// [`Bins::RED`] and the rest), from a signal of 0 to 1.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bins {
    pub bins: u32,
    pub counts: [Vec<u32>; 4],
}

impl Bins {
    pub const RED: usize = 0;
    pub const GREEN: usize = 1;
    pub const BLUE: usize = 2;
    pub const LUMA: usize = 3;

    /// Of the counts of a histogram, as [`Scopes::read_counts`] lays them out.
    fn of(bins: u32, counts: &[u32]) -> Bins {
        let size = bins as usize;
        Bins {
            bins,
            counts: [0, 1, 2, 3]
                .map(|channel| counts[channel * size..(channel + 1) * size].to_vec()),
        }
    }

    /// The signal of a bin (at its middle).
    #[must_use]
    pub fn signal(&self, bin: u32) -> f64 {
        f64::from(bin) / f64::from(self.bins - 1)
    }

    /// The signal of the bin below which `share` (from 0 to 1) of the pixels of `channel` are:
    /// 0.5 for the median, 0.01 and 0.99 for the near extremes.
    ///
    /// # Panics
    /// `channel` is not one of the four.
    #[must_use]
    pub fn percentile(&self, channel: usize, share: f64) -> f64 {
        let counts = &self.counts[channel];
        let total: u64 = counts.iter().map(|&count| u64::from(count)).sum();
        #[allow(clippy::cast_precision_loss, reason = "a count of pixels")]
        let wanted = share.clamp(0.0, 1.0) * total as f64;
        let mut below = 0u64;
        for (bin, &count) in (0..).zip(counts) {
            below += u64::from(count);
            #[allow(clippy::cast_precision_loss, reason = "a count of pixels")]
            if below as f64 >= wanted && below > 0 {
                return self.signal(bin);
            }
        }
        1.0
    }
}

/// The figures of a frame for the agent: of every pixel, its R', G', B' and luma as signals,
/// its chroma, and a histogram.
#[derive(Clone, Debug, PartialEq)]
pub struct Statistics {
    /// How many pixels.
    pub samples: u64,
    pub red: Channel,
    pub green: Channel,
    pub blue: Channel,
    pub luma: Channel,
    pub chroma: Chroma,
    pub histogram: Bins,
}

/// Why a scope cannot be made, counted or read.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ScopeError {
    /// Its settings are out of range.
    #[error("the settings of the scope are out of range: {0}")]
    Settings(String),
    /// The frame is not one that a scope reads.
    #[error("the frame cannot be measured: {0}")]
    Frame(String),
    /// The device was lost.
    #[error("the graphics device was lost: {0}")]
    Lost(String),
    /// The card did not give back what was read.
    #[error("the scope cannot be read back: {0}")]
    Read(String),
}
