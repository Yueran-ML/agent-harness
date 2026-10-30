//! Text laid out and drawn into pixels on the processor ([cosmic-text]): titles, in the font,
//! size, colour and alignment of their style, and subtitles, white with a black outline.
//!
//! A text is laid out first ([`lay_out`]), which says how large it is without drawing it, and
//! then drawn into pixels a part at a time ([`draw`]): the part that is seen on the canvas, at
//! most [`MOST_PIXELS`] of them, so that a title of letters 4096 pixels high, many lines long,
//! takes no more memory than a picture of 4K.
//!
//! The fonts are the system's: the family that a style names, or, where it names none, the
//! sans-serif family of the system (Segoe UI on Windows); letters that a font does not have are
//! taken from the fonts that cosmic-text falls back to for their script (`Microsoft YaHei UI` for
//! Chinese, Yu Gothic for Japanese, on Windows). No font is bundled. The fonts of the system are
//! read once for the process, on a thread of its own as the first compositor is made
//! ([`warm_up`]), since that takes a moment.
//!
//! [cosmic-text]: https://github.com/pop-os/cosmic-text

use std::sync::{Mutex, OnceLock, PoisonError};

use columbina_model::project::{TextAlign, TitleStyle};
use std::collections::HashMap;

use cosmic_text::{
    Align, Attrs, Buffer, CacheKey, Color, Family, FontSystem, Metrics, Shaping, SwashCache,
    SwashContent,
};

/// The most pixels of a text that are drawn at once: 4096 × 2048.
pub(crate) const MOST_PIXELS: u64 = 4096 * 2048;

/// Letters higher than this many pixels are drawn, and not kept to be drawn again.
const LARGE: f32 = 256.0;

/// A title: what it says, and how it is drawn.
#[derive(Clone, Debug, PartialEq)]
pub struct Text {
    pub text: String,
    pub style: TitleStyle,
}

/// Part of a text drawn into pixels: R'G'B'A, premultiplied, `width` × `height` of them from
/// the pixel `start` of `pixels`, rows `row` pixels apart.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Raster {
    pub width: u32,
    pub height: u32,
    pub row: u32,
    pub start: usize,
    pub pixels: Vec<[u8; 4]>,
}

/// The fonts of the system, read once, and the glyphs drawn from them.
struct Fonts {
    system: FontSystem,
    glyphs: SwashCache,
    /// The outlines of letters, as they were made, by letter and radius.
    outlines: HashMap<(CacheKey, usize), Option<Outline>>,
}

/// How many outlines of letters are kept, at most.
const OUTLINES_KEPT: usize = 4096;

/// The outline of a letter: how much of each pixel it covers, `width` × `height` of them, from
/// `left` across and `top` up of where the letter is put (as the image of a letter is placed).
struct Outline {
    left: i32,
    top: i32,
    width: usize,
    height: usize,
    mask: Vec<u8>,
}

/// The outline, `radius` pixels about it, of the letter of `key`; none for a letter of no
/// pixels.
fn outline_of(
    system: &mut FontSystem,
    glyphs: &mut SwashCache,
    key: CacheKey,
    radius: usize,
) -> Option<Outline> {
    let image = glyphs.get_image(system, key).as_ref()?;
    let (width, height) = (
        image.placement.width as usize,
        image.placement.height as usize,
    );
    let channels = match image.content {
        SwashContent::Mask => 1,
        SwashContent::Color => 4,
        SwashContent::SubpixelMask => return None,
    };
    let (columns, rows) = (width + 2 * radius, height + 2 * radius);
    let mut cover = vec![0_u8; columns * rows];
    for y in 0..height {
        for x in 0..width {
            let at = (y * width + x) * channels + channels - 1;
            cover[(y + radius) * columns + x + radius] = image.data[at];
        }
    }
    let radius_signed = i32::try_from(radius).ok()?;
    Some(Outline {
        left: image.placement.left - radius_signed,
        top: image.placement.top + radius_signed,
        width: columns,
        height: rows,
        mask: dilate(&cover, columns, rows, radius),
    })
}

fn fonts() -> &'static Mutex<Fonts> {
    static FONTS: OnceLock<Mutex<Fonts>> = OnceLock::new();
    FONTS.get_or_init(|| {
        let mut system = FontSystem::new();
        if cfg!(windows) {
            system.db_mut().set_sans_serif_family("Segoe UI");
        }
        Mutex::new(Fonts {
            system,
            glyphs: SwashCache::new(),
            outlines: HashMap::new(),
        })
    })
}

/// Read the fonts of the system on a thread of its own, once for the process, so that the first
/// text drawn does not wait for them (unless it comes before they are read).
pub fn warm_up() {
    static WARMED: OnceLock<()> = OnceLock::new();
    WARMED.get_or_init(|| {
        let _ = std::thread::Builder::new()
            .name("columbina-fonts".into())
            .spawn(|| {
                fonts();
            });
    });
}

/// The height of a line, for letters of `size` pixels.
fn line_height(size: f32) -> f32 {
    (size * 1.25).ceil()
}

/// What is drawn: the lines of a text, in a family (or the system's), of letters `size` pixels
/// high, aligned, in a colour, with an outline of `outline` pixels about the letters, in black.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Spec {
    pub text: String,
    pub family: Option<String>,
    pub size: f32,
    pub align: TextAlign,
    pub color: [u8; 4],
    pub outline: u32,
}

impl Spec {
    /// A title, of letters `size` pixels high.
    pub(crate) fn title(text: &Text, size: f32) -> Spec {
        let style = &text.style;
        Spec {
            text: text.text.clone(),
            family: (!style.font.is_empty()).then(|| style.font.clone()),
            size,
            align: style.align,
            color: style.color.0,
            outline: 0,
        }
    }

    /// Subtitles, a line after another, centred, white with a black outline, of letters `size`
    /// pixels high.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a size of at most 16384 / 20"
    )]
    pub(crate) fn subtitles(lines: &[String], size: f32) -> Spec {
        Spec {
            text: lines.join("\n"),
            family: None,
            size,
            align: TextAlign::Center,
            color: [255, 255, 255, 255],
            outline: (size / 14.0).ceil().max(1.0) as u32,
        }
    }
}

/// A text laid out: how large the block of its lines is (as wide as the widest, as high as all
/// of them), and the pixels about it (`pad`) that the parts of letters that reach beyond it and
/// an outline take.
pub(crate) struct Laid {
    buffer: Buffer,
    spec: Spec,
    pub block: (u32, u32),
    pub pad: u32,
}

impl Laid {
    /// The size of all its pixels, the block and the pad about it.
    pub(crate) fn full(&self) -> (u32, u32) {
        (self.block.0 + 2 * self.pad, self.block.1 + 2 * self.pad)
    }
}

/// Lay out text as `spec` says, drawing nothing.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "pixels of a block of text"
)]
pub(crate) fn lay_out(spec: &Spec) -> Laid {
    let mut fonts = fonts().lock().unwrap_or_else(PoisonError::into_inner);
    let system = &mut fonts.system;
    let size = spec.size.max(1.0);
    let mut buffer = Buffer::new(system, Metrics::new(size, line_height(size)));
    let attrs = match &spec.family {
        Some(family) => Attrs::new().family(Family::Name(family)),
        None => Attrs::new().family(Family::SansSerif),
    };
    let align = match spec.align {
        TextAlign::Left => Align::Left,
        TextAlign::Center => Align::Center,
        TextAlign::Right => Align::Right,
    };
    buffer.set_size(None, None);
    buffer.set_text(&spec.text, &attrs, Shaping::Advanced, Some(align));
    buffer.shape_until_scroll(system, false);
    let (width, height) = buffer
        .layout_runs()
        .fold((0.0_f32, 0.0_f32), |(width, height), run| {
            (
                width.max(run.line_w),
                height.max(run.line_top + run.line_height),
            )
        });
    // Aligned within the widest line.
    buffer.set_size(Some(width.ceil()), None);
    buffer.shape_until_scroll(system, false);
    Laid {
        buffer,
        spec: spec.clone(),
        block: (width.ceil() as u32, height.ceil() as u32),
        pad: (size / 4.0).ceil() as u32 + spec.outline,
    }
}

/// Draw the part `window` of a text laid out: x0, y0, x1, y1 of its full pixels (see
/// [`Laid::full`]), which the caller keeps within [`MOST_PIXELS`].
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::many_single_char_names,
    clippy::too_many_lines,
    reason = "pixels of a block of text, far within the range of each; R, G, B; a part drawn               step by step"
)]
pub(crate) fn draw(laid: &mut Laid, window: [u32; 4]) -> Raster {
    let [x0, y0, x1, y1] = window;
    let (width, height) = (x1.saturating_sub(x0), y1.saturating_sub(y0));
    let (columns, rows) = (width as usize, height as usize);
    let spec = &laid.spec;
    let outline = spec.outline as usize;
    // The letters are drawn over a margin as wide as their outline, which reaches into the part
    // from beyond it.
    let (margin_columns, margin_rows) = (columns + 2 * outline, rows + 2 * outline);
    let origin = (
        i64::from(laid.pad) - i64::from(x0) + outline as i64,
        i64::from(laid.pad) - i64::from(y0) + outline as i64,
    );
    // Premultiplied, over what is there, and how much of each pixel the letters cover.
    let mut paint = vec![[0_u8; 4]; margin_columns * margin_rows];
    let mut cover = vec![0_u8; if outline > 0 { paint.len() } else { 0 }];
    let [r, g, b, _] = spec.color;
    let mut fonts = fonts().lock().unwrap_or_else(PoisonError::into_inner);
    let Fonts {
        system,
        glyphs,
        outlines,
    } = &mut *fonts;
    let mut put = |x: i32, y: i32, color: Color| {
        let py = i64::from(y) + origin.1;
        let px = i64::from(x) + origin.0;
        if py < 0 || py >= margin_rows as i64 || px < 0 || px >= margin_columns as i64 {
            return;
        }
        let alpha = u32::from(color.a());
        let rgb = [color.r(), color.g(), color.b()].map(|c| u32::from(c) * alpha / 255);
        let at = py as usize * margin_columns + px as usize;
        let under = paint[at].map(u32::from);
        let keep = 255 - alpha;
        paint[at] = [
            (rgb[0] + under[0] * keep / 255) as u8,
            (rgb[1] + under[1] * keep / 255) as u8,
            (rgb[2] + under[2] * keep / 255) as u8,
            (alpha + under[3] * keep / 255) as u8,
        ];
    };
    // Only the letters that may reach into the part are drawn: a letter reaches at most its size
    // beyond its box. Letters larger than LARGE are not kept once drawn, as the others are.
    let reach = f64::from(spec.size) + outline as f64;
    let (seen_x, seen_y) = (
        (f64::from(x0) - f64::from(laid.pad) - reach)
            ..(f64::from(x1) - f64::from(laid.pad) + reach),
        (f64::from(y0) - f64::from(laid.pad) - reach)
            ..(f64::from(y1) - f64::from(laid.pad) + reach),
    );
    let large = spec.size > LARGE;
    let (mut once, mut once_outlines) = (SwashCache::new(), HashMap::new());
    let (glyphs, outlines) = if large {
        (&mut once, &mut once_outlines)
    } else {
        if outlines.len() > OUTLINES_KEPT {
            outlines.clear();
        }
        (glyphs, outlines)
    };
    let base = Color::rgba(r, g, b, 255);
    let mut seen = Vec::new();
    for run in laid.buffer.layout_runs() {
        let (top, bottom) = (
            f64::from(run.line_top),
            f64::from(run.line_top + run.line_height),
        );
        if bottom < seen_y.start || top > seen_y.end {
            continue;
        }
        for glyph in run.glyphs {
            let (left, right) = (f64::from(glyph.x), f64::from(glyph.x + glyph.w));
            if right < seen_x.start || left > seen_x.end {
                continue;
            }
            seen.push((
                glyph.physical((0.0, run.line_y), 1.0),
                glyph.color_opt.unwrap_or(base),
            ));
        }
    }
    // The outlines first, each made once for its letter and kept, rather than of all the
    // pixels at once.
    if outline > 0 {
        for (physical, _) in &seen {
            let made = outlines
                .entry((physical.cache_key, outline))
                .or_insert_with(|| outline_of(system, glyphs, physical.cache_key, outline));
            let Some(made) = made else { continue };
            let x = i64::from(physical.x + made.left) + origin.0;
            let y = i64::from(physical.y - made.top) + origin.1;
            for row in 0..made.height {
                let py = y + row as i64;
                if py < 0 || py >= margin_rows as i64 {
                    continue;
                }
                let line = &made.mask[row * made.width..(row + 1) * made.width];
                let (from, to) = (
                    (-x).clamp(0, made.width as i64) as usize,
                    (margin_columns as i64 - x).clamp(0, made.width as i64) as usize,
                );
                if from >= to {
                    continue;
                }
                let at = py as usize * margin_columns + (x + from as i64) as usize;
                for (covered, value) in cover[at..at + (to - from)].iter_mut().zip(&line[from..to])
                {
                    *covered = (*covered).max(*value);
                }
            }
        }
    }
    for (physical, color) in seen {
        glyphs.with_pixels(system, physical.cache_key, color, |x, y, color| {
            put(physical.x + x, physical.y + y, color);
        });
    }
    drop(fonts);
    let around = cover;
    // The letters over their outline, which is black, in place; all as opaque as the colour.
    let opacity = u32::from(spec.color[3]);
    if outline > 0 || opacity < 255 {
        for (index, pixel) in paint.iter_mut().enumerate() {
            let under = around
                .get(index)
                .map_or(0, |a| u32::from(*a) * (255 - u32::from(pixel[3])) / 255);
            if under == 0 && opacity == 255 {
                continue;
            }
            let alpha = (u32::from(pixel[3]) + under).min(255);
            let [r, g, b] = [pixel[0], pixel[1], pixel[2]].map(u32::from);
            *pixel = [r, g, b, alpha].map(|value| (value * opacity / 255) as u8);
        }
    }
    Raster {
        width,
        height,
        row: u32::try_from(margin_columns).unwrap_or(u32::MAX),
        start: outline * margin_columns + outline,
        pixels: paint,
    }
}

/// How far the outline of letters that cover `cover` reaches: the most of what they cover within
/// `radius` pixels across and down of each pixel (a square), in two passes, each in time linear
/// in the pixels whatever the radius, and each over rows, as they lie in memory.
fn dilate(cover: &[u8], columns: usize, rows: usize, radius: usize) -> Vec<u8> {
    // Across: each row alone.
    let mut across = vec![0_u8; cover.len()];
    let mut scratch = Scratch::default();
    for (row, out) in cover.chunks(columns).zip(across.chunks_mut(columns)) {
        if row.iter().any(|value| *value > 0) {
            most_within(row, out, radius, &mut scratch);
        }
    }
    // Down: rows as wholes, the most of each place of them.
    most_within_rows(&across, columns, rows, radius)
}

/// What [`most_within`] works in, kept from a line to the next.
#[derive(Default)]
struct Scratch {
    padded: Vec<u8>,
    from_start: Vec<u8>,
    to_end: Vec<u8>,
}

/// The most of `line` within `radius` of each place, beyond its ends taken as 0: by blocks of
/// the width of the window, the most from the start of its block to each place and from each
/// place to the end of its block (van Herk, Gil and Werman).
fn most_within(line: &[u8], out: &mut [u8], radius: usize, scratch: &mut Scratch) {
    let window = 2 * radius + 1;
    // The line with `radius` zeros at each end, so that a window never leaves it.
    let padded = &mut scratch.padded;
    padded.clear();
    padded.resize(radius, 0);
    padded.extend_from_slice(line);
    padded.resize(line.len() + 2 * radius, 0);
    let length = padded.len();
    scratch.from_start.resize(length, 0);
    scratch.to_end.resize(length, 0);
    let (from_start, to_end) = (&mut scratch.from_start, &mut scratch.to_end);
    for block in (0..length).step_by(window) {
        let end = (block + window).min(length);
        let mut most = 0;
        for index in block..end {
            most = most.max(padded[index]);
            from_start[index] = most;
        }
        let mut most = 0;
        for index in (block..end).rev() {
            most = most.max(padded[index]);
            to_end[index] = most;
        }
    }
    for (place, value) in out.iter_mut().enumerate().take(line.len()) {
        // The window of `place` is padded[place ..= place + 2 radius].
        *value = to_end[place].max(from_start[place + 2 * radius]);
    }
}

/// The most of each place of the rows of `plane` (of `columns` each) within `radius` rows, as
/// [`most_within`] does along a line, a row taken as one value.
fn most_within_rows(plane: &[u8], columns: usize, rows: usize, radius: usize) -> Vec<u8> {
    let window = 2 * radius + 1;
    let length = rows + 2 * radius;
    // Row `index` of the plane padded with `radius` rows of zeros above and below.
    let row = |index: usize| -> Option<&[u8]> {
        (radius..radius + rows)
            .contains(&index)
            .then(|| &plane[(index - radius) * columns..(index - radius + 1) * columns])
    };
    let mut from_start = vec![0_u8; length * columns];
    let mut to_end = vec![0_u8; length * columns];
    for index in 0..length {
        let (done, rest) = from_start.split_at_mut(index * columns);
        let out = &mut rest[..columns];
        let previous = (index % window != 0).then(|| &done[(index - 1) * columns..]);
        match (row(index), previous) {
            (Some(row), Some(previous)) => {
                for ((out, a), b) in out.iter_mut().zip(row).zip(previous) {
                    *out = (*a).max(*b);
                }
            }
            (Some(row), None) => out.copy_from_slice(row),
            (None, Some(previous)) => out.copy_from_slice(&previous[..columns]),
            (None, None) => {}
        }
    }
    for index in (0..length).rev() {
        let (head, rest) = to_end.split_at_mut((index + 1) * columns);
        let out = &mut head[index * columns..];
        let next = (index % window != window - 1 && index != length - 1).then_some(&rest[..]);
        match (row(index), next) {
            (Some(row), Some(next)) => {
                for ((out, a), b) in out.iter_mut().zip(row).zip(next) {
                    *out = (*a).max(*b);
                }
            }
            (Some(row), None) => out.copy_from_slice(row),
            (None, Some(next)) => out.copy_from_slice(&next[..columns]),
            (None, None) => {}
        }
    }
    let mut out = vec![0_u8; rows * columns];
    for (place, line) in out.chunks_mut(columns).enumerate() {
        let low = &to_end[place * columns..(place + 1) * columns];
        let high = &from_start[(place + 2 * radius) * columns..(place + 2 * radius + 1) * columns];
        for ((out, a), b) in line.iter_mut().zip(low).zip(high) {
            *out = (*a).max(*b);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_outline_reaches_a_square_about_each_letter() {
        let (columns, rows) = (7, 7);
        let mut cover = vec![0; columns * rows];
        cover[3 * columns + 3] = 200;
        cover[0] = 10;
        let out = dilate(&cover, columns, rows, 2);
        let at = |x: usize, y: usize| out[y * columns + x];
        for (x, y) in [(3, 3), (5, 3), (3, 1), (4, 4), (5, 5), (1, 1)] {
            assert_eq!(at(x, y), 200, "({x}, {y})");
        }
        assert_eq!(at(6, 3), 0);
        assert_eq!(at(3, 6), 0);
        // The corner reaches as far, within the plane.
        assert_eq!(at(2, 2), 200);
        assert_eq!(at(0, 2), 10);
        assert_eq!(at(0, 3), 0);
    }

    #[test]
    fn the_most_within_a_window_is_that_of_each_place_however_long_the_line() {
        let line = [1, 5, 2, 0, 0, 0, 9, 3, 0, 4];
        for radius in 0..5 {
            let mut out = [0; 10];
            most_within(&line, &mut out, radius, &mut Scratch::default());
            // And as rows of one place each.
            assert_eq!(most_within_rows(&line, 1, line.len(), radius), out);
            for (place, value) in out.iter().enumerate() {
                let low = place.saturating_sub(radius);
                let high = (place + radius).min(line.len() - 1);
                let wanted = line[low..=high].iter().copied().max().unwrap();
                assert_eq!(*value, wanted, "radius {radius} at {place}");
            }
        }
    }

    #[test]
    fn lines_are_a_quarter_higher_than_their_letters() {
        assert!((line_height(48.0) - 60.0).abs() < f32::EPSILON);
        assert!((line_height(54.0) - 68.0).abs() < f32::EPSILON);
    }

    #[test]
    fn subtitles_are_outlined_a_fourteenth_of_their_size() {
        let lines = ["one".to_owned(), "two".to_owned()];
        let spec = Spec::subtitles(&lines, 54.0);
        assert_eq!(spec.text, "one\ntwo");
        assert_eq!(spec.outline, 4);
        assert_eq!(Spec::subtitles(&lines, 5.0).outline, 1);
        assert_eq!(spec.align, TextAlign::Center);
    }

    #[test]
    fn a_row_covered_whole_is_outlined_as_it_is() {
        let (columns, rows) = (4, 3);
        let mut cover = vec![0; columns * rows];
        cover[columns..2 * columns].fill(255);
        let out = dilate(&cover, columns, rows, 1);
        assert!(out.iter().all(|value| *value == 255), "{out:?}");
    }

    #[test]
    fn a_title_is_drawn_in_its_font_or_in_the_one_of_the_engine() {
        let title = |font: &str| Text {
            text: "Hello".into(),
            style: TitleStyle {
                font: font.into(),
                size: 96,
                color: columbina_model::project::Rgba([255, 255, 255, 255]),
                x: 0.5,
                y: 0.5,
                align: TextAlign::Center,
            },
        };
        assert_eq!(Spec::title(&title(""), 96.0).family, None);
        let spec = Spec::title(&title("Arial"), 48.0);
        assert_eq!(spec.family.as_deref(), Some("Arial"));
        assert!((spec.size - 48.0).abs() < f32::EPSILON);
        assert_eq!(spec.outline, 0);
    }

    #[test]
    fn a_text_is_laid_out_a_line_high_for_each_of_its_lines_with_a_pad_about_it() {
        let lines = ["one".to_owned(), "two".to_owned()];
        let spec = Spec::subtitles(&lines, 48.0);
        let laid = lay_out(&spec);
        // Two lines of 60 pixels; a pad of a quarter of the size and the outline of 4.
        assert_eq!(laid.block.1, 120);
        assert!(laid.block.0 > 0);
        assert_eq!(laid.pad, 12 + 4);
        assert_eq!(laid.full(), (laid.block.0 + 32, 152));
        let one = lay_out(&Spec::subtitles(&lines[..1], 48.0));
        assert_eq!(one.block.1, 60);
    }

    #[test]
    fn subtitles_are_outlined_as_far_on_every_side_of_their_letters() {
        // Letters of 56 pixels, outlined by 4.
        let spec = Spec::subtitles(&["I".to_owned()], 56.0);
        let mut laid = lay_out(&spec);
        let (width, height) = laid.full();
        let raster = draw(&mut laid, [0, 0, width, height]);
        let bounds = |keep: &dyn Fn([u8; 4]) -> bool| {
            let mut found = [u32::MAX, u32::MAX, 0, 0];
            for y in 0..raster.height {
                for x in 0..raster.width {
                    let at = raster.start + (y * raster.row + x) as usize;
                    if keep(raster.pixels[at]) {
                        found = [
                            found[0].min(x),
                            found[1].min(y),
                            found[2].max(x + 1),
                            found[3].max(y + 1),
                        ];
                    }
                }
            }
            found
        };
        let letter = bounds(&|pixel| pixel[0] > 128);
        let outlined = bounds(&|pixel| pixel[3] > 0);
        let reach = [
            letter[0] - outlined[0],
            letter[1] - outlined[1],
            outlined[2] - letter[2],
            outlined[3] - letter[3],
        ];
        for side in reach {
            assert!(
                (4..=6).contains(&side),
                "{reach:?}: {letter:?} in {outlined:?}"
            );
        }
    }

    #[test]
    fn the_outline_of_subtitles_covers_what_is_within_its_width_of_a_letter_and_nothing_else() {
        // Letters of 56 pixels, outlined by 4; and of 7, outlined by 1, some a pixel thin.
        for (text, size) in [("TL", 56.0), ("l-|.i", 7.0), ("-_.~", 5.0), ("-_=", 9.0)] {
            let spec = Spec::subtitles(&[text.to_owned()], size);
            let radius = spec.outline as usize;
            let mut laid = lay_out(&spec);
            let (width, height) = laid.full();
            let raster = draw(&mut laid, [0, 0, width, height]);
            let (columns, rows) = (width as usize, height as usize);
            let at = |x: usize, y: usize| raster.pixels[raster.start + y * raster.row as usize + x];
            let mut wrong = vec![];
            for y in 0..rows {
                for x in 0..columns {
                    let near = (y.saturating_sub(radius)..=(y + radius).min(rows - 1)).any(|v| {
                        (x.saturating_sub(radius)..=(x + radius).min(columns - 1))
                            .any(|u| at(u, v)[0] > 0)
                    });
                    if near != (at(x, y)[3] > 0) {
                        wrong.push((x, y));
                    }
                }
            }
            assert!(
                wrong.is_empty(),
                "{text}: {} pixels, first {:?}",
                wrong.len(),
                wrong.first()
            );
        }
    }

    #[test]
    fn a_letter_in_colour_is_outlined_as_far_on_every_side_as_one_in_white() {
        let spec = Spec::subtitles(&["\u{1F600}".to_owned()], 56.0);
        let bare = Spec {
            outline: 0,
            ..spec.clone()
        };
        let drawn = |spec: &Spec| {
            let mut laid = lay_out(spec);
            let (width, height) = laid.full();
            let raster = draw(&mut laid, [0, 0, width, height]);
            let mut found = [u32::MAX, u32::MAX, 0, 0];
            for y in 0..raster.height {
                for x in 0..raster.width {
                    if raster.pixels[raster.start + (y * raster.row + x) as usize][3] > 0 {
                        found = [
                            found[0].min(x),
                            found[1].min(y),
                            found[2].max(x + 1),
                            found[3].max(y + 1),
                        ];
                    }
                }
            }
            found
        };
        let (letter, outlined) = (drawn(&bare), drawn(&spec));
        if letter[2] == 0 {
            // No font of this system draws it, nor a box in its place.
            return;
        }
        // The pad about the outlined one is as much larger as its outline.
        let outline = spec.outline;
        assert_eq!(
            outlined,
            [
                letter[0],
                letter[1],
                letter[2] + 2 * outline,
                letter[3] + 2 * outline
            ]
        );
    }
}
