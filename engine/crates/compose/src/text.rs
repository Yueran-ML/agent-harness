//! Text laid out and drawn into pixels on the processor ([cosmic-text]): titles, in the font,
//! size, colour and alignment of their style, and subtitles, white with a black outline.
//!
//! The fonts are the system's: the family that a style names, or, where it names none, the
//! sans-serif family of the system (Segoe UI on Windows); letters that a font does not have are
//! taken from the fonts that cosmic-text falls back to for their script (`Microsoft YaHei UI` for
//! Chinese, Yu Gothic for Japanese, on Windows). No font is bundled. The fonts of the system are
//! read once for the process, which takes a moment (up to a second), the first time a text is
//! drawn.
//!
//! [cosmic-text]: https://github.com/pop-os/cosmic-text

use std::sync::{Mutex, OnceLock, PoisonError};

use columbina_model::project::{TextAlign, TitleStyle};
use cosmic_text::{Align, Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, SwashCache};

/// A title: what it says, and how it is drawn.
#[derive(Clone, Debug, PartialEq)]
pub struct Text {
    pub text: String,
    pub style: TitleStyle,
}

/// Text drawn into pixels: R'G'B'A, premultiplied, a row after another, with `pad` pixels about
/// the block of its lines (`block`), for the parts of letters that reach beyond it and for an
/// outline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Raster {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    /// The size of the block of its lines: as wide as the widest, as high as all of them.
    pub block: (u32, u32),
    pub pad: u32,
}

/// The fonts of the system, read once, and the glyphs drawn from them.
struct Fonts {
    system: FontSystem,
    glyphs: SwashCache,
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
        })
    })
}

/// The height of a line, for letters of `size` pixels.
fn line_height(size: f32) -> f32 {
    (size * 1.25).ceil()
}

/// What is drawn: the lines of a text, in a family (or the system's), of letters `size` pixels
/// high, aligned, in a colour, with an outline of `outline` pixels about the letters, in black.
struct Spec<'a> {
    text: &'a str,
    family: Option<&'a str>,
    size: f32,
    align: TextAlign,
    color: [u8; 4],
    outline: u32,
}

/// Draw text as `spec` says.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    reason = "pixels of a block of text, far within the range of each"
)]
fn draw(spec: &Spec) -> Raster {
    let mut fonts = fonts().lock().unwrap_or_else(PoisonError::into_inner);
    let Fonts { system, glyphs } = &mut *fonts;
    let size = spec.size.max(1.0);
    let mut buffer = Buffer::new(system, Metrics::new(size, line_height(size)));
    let attrs = match spec.family {
        Some(family) => Attrs::new().family(Family::Name(family)),
        None => Attrs::new().family(Family::SansSerif),
    };
    let align = match spec.align {
        TextAlign::Left => Align::Left,
        TextAlign::Center => Align::Center,
        TextAlign::Right => Align::Right,
    };
    buffer.set_size(None, None);
    buffer.set_text(spec.text, &attrs, Shaping::Advanced, Some(align));
    buffer.shape_until_scroll(system, false);
    let measure = |buffer: &Buffer| {
        buffer
            .layout_runs()
            .fold((0.0_f32, 0.0_f32), |(width, height), run| {
                (
                    width.max(run.line_w),
                    height.max(run.line_top + run.line_height),
                )
            })
    };
    let (width, height) = measure(&buffer);
    // Aligned within the widest line.
    buffer.set_size(Some(width.ceil()), None);
    buffer.shape_until_scroll(system, false);
    let block = (width.ceil() as u32, height.ceil() as u32);
    let pad = (size / 4.0).ceil() as u32 + spec.outline;
    let (full_width, full_height) = (block.0 + 2 * pad, block.1 + 2 * pad);
    let (columns, rows) = (full_width as usize, full_height as usize);
    // How much of each pixel the letters cover.
    let mut cover = vec![0_f32; columns * rows];
    // Their colour there, premultiplied, for letters of colour (emoji).
    let mut paint = vec![[0_f32; 4]; columns * rows];
    let [r, g, b, _] = spec.color;
    buffer.draw(
        system,
        glyphs,
        Color::rgba(r, g, b, 255),
        |x, y, width, height, color| {
            let alpha = f32::from(color.a()) / 255.0;
            let rgb = [color.r(), color.g(), color.b()].map(|c| f32::from(c) / 255.0 * alpha);
            for dy in 0..height as i32 {
                for dx in 0..width as i32 {
                    let (px, py) = (x + dx + pad as i32, y + dy + pad as i32);
                    if px < 0 || py < 0 || px >= full_width as i32 || py >= full_height as i32 {
                        continue;
                    }
                    let at = py as usize * columns + px as usize;
                    let under = paint[at];
                    // Over what is there, premultiplied.
                    paint[at] = [
                        rgb[0] + under[0] * (1.0 - alpha),
                        rgb[1] + under[1] * (1.0 - alpha),
                        rgb[2] + under[2] * (1.0 - alpha),
                        alpha + under[3] * (1.0 - alpha),
                    ];
                    cover[at] = cover[at].max(alpha);
                }
            }
        },
    );
    drop(fonts);
    let outline = if spec.outline > 0 {
        dilate(&cover, columns, rows, spec.outline)
    } else {
        vec![0.0; columns * rows]
    };
    let opacity = f32::from(spec.color[3]) / 255.0;
    let mut pixels = Vec::with_capacity(columns * rows * 4);
    for (at, letter) in paint.iter().enumerate() {
        // The letters over their outline, which is black.
        let under = outline[at] * (1.0 - letter[3]);
        let rgba = [letter[0], letter[1], letter[2], letter[3] + under];
        pixels.extend(rgba.map(|value| (value * opacity * 255.0).round().clamp(0.0, 255.0) as u8));
    }
    Raster {
        width: full_width,
        height: full_height,
        pixels,
        block,
        pad,
    }
}

/// How far the outline of letters that cover `cover` reaches: the most of what they cover within
/// `radius` pixels of each pixel, a disc.
#[allow(
    clippy::cast_possible_wrap,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "radii and sizes of a block of text"
)]
fn dilate(cover: &[f32], columns: usize, rows: usize, radius: u32) -> Vec<f32> {
    let radius = radius as i32;
    let mut out = vec![0.0_f32; cover.len()];
    let offsets: Vec<(i32, i32)> = (-radius..=radius)
        .flat_map(|dy| (-radius..=radius).map(move |dx| (dx, dy)))
        .filter(|(dx, dy)| dx * dx + dy * dy <= radius * radius)
        .collect();
    for y in 0..rows as i32 {
        for x in 0..columns as i32 {
            let mut most = 0.0_f32;
            for (dx, dy) in &offsets {
                let (sx, sy) = (x + dx, y + dy);
                if sx >= 0 && sy >= 0 && (sx as usize) < columns && (sy as usize) < rows {
                    most = most.max(cover[sy as usize * columns + sx as usize]);
                }
            }
            out[y as usize * columns + x as usize] = most;
        }
    }
    out
}

/// A title drawn as its style says. A size beyond what can be drawn is taken as 4096 pixels.
pub(crate) fn title(text: &Text) -> Raster {
    let style = &text.style;
    #[allow(clippy::cast_precision_loss, reason = "sizes of at most 4096")]
    let size = style.size.min(4096) as f32;
    draw(&Spec {
        text: &text.text,
        family: (!style.font.is_empty()).then_some(style.font.as_str()),
        size,
        align: style.align,
        color: style.color.0,
        outline: 0,
    })
}

/// Subtitles, a line after another, centred, white with a black outline, of letters `size`
/// pixels high.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a size of at most 16384 / 20"
)]
pub(crate) fn subtitles(lines: &[String], size: f32) -> Raster {
    let text = lines.join("\n");
    draw(&Spec {
        text: &text,
        family: None,
        size,
        align: TextAlign::Center,
        color: [255, 255, 255, 255],
        outline: (size / 14.0).ceil().max(1.0) as u32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_outline_reaches_a_disc_about_each_letter() {
        let (columns, rows) = (7, 7);
        let mut cover = vec![0.0; columns * rows];
        cover[3 * columns + 3] = 1.0;
        let out = dilate(&cover, columns, rows, 2);
        let at = |x: usize, y: usize| out[y * columns + x];
        assert!((at(3, 3) - 1.0).abs() < f32::EPSILON);
        assert!((at(5, 3) - 1.0).abs() < f32::EPSILON);
        assert!((at(3, 1) - 1.0).abs() < f32::EPSILON);
        assert!((at(4, 4) - 1.0).abs() < f32::EPSILON);
        // The corners of the square about it are beyond the disc.
        assert!(at(5, 5).abs() < f32::EPSILON);
        assert!(at(0, 3).abs() < f32::EPSILON);
    }

    #[test]
    fn lines_are_a_quarter_higher_than_their_letters() {
        assert!((line_height(48.0) - 60.0).abs() < f32::EPSILON);
        assert!((line_height(54.0) - 68.0).abs() < f32::EPSILON);
    }
}
