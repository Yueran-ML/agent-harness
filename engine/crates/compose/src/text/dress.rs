//! What a title has about its letters, drawn on the processor ([`TitleStyle`]): a stroke about
//! them, a shadow of them, a box behind them, and runs of its text in a colour, a size, a
//! weight, a slant or a family of their own.
//!
//! A title that has none of them is laid out and drawn as it always was ([`super::draw`]). One
//! that has any is drawn here, a part at a time as any text is, in this order: its box, the
//! shadow of its letters and of their stroke, the stroke, and the letters over it. The pad about
//! the block of its lines grows by as far as the stroke, the shadow (moved and blurred) and the
//! box reach beyond the letters, so that nothing of them is cut away ([`Dress::pad`]).
//!
//! The stroke is made for each letter from how much of each pixel it covers, by a distance
//! transform ([`planes::stroke`]), and kept as the outlines of subtitles are. The shadow is the
//! letters and their stroke as they are drawn, as opaque as they are, moved by its offset (by a
//! fraction of a pixel too, between the pixels about it) and blurred by a Gaussian
//! ([`planes::gaussian`]). A run is given to cosmic-text as attributes of its own: its colour,
//! its size (in the metrics of its letters, so that a line is as high as its largest letters
//! need), its weight, its slant (slanted by cosmic-text where the font has no italic face) and
//! its family. Letters are as opaque as their colours: all of them as the title's, as titles
//! always were, or, where a run's colour is of another opacity, each pixel as the letters drawn
//! in it are.
//!
//! [`TitleStyle`]: columbina_model::project::TitleStyle

use std::collections::HashMap;
use std::ops::Range;
use std::sync::PoisonError;

use columbina_model::project::Styled;
use cosmic_text::{
    Attrs, CacheKey, Color, Family, FontSystem, Metrics, PhysicalGlyph, Style, SwashCache, Weight,
};

use super::{
    covered, fonts, forget_glyphs_beyond, line_height, planes, DressKey, Fonts, Laid, Outline,
    Raster, Strokes, GLYPH_BYTES, LARGE, OUTLINES_KEPT,
};

/// What a title has about its letters, each length in pixels drawn; nothing for subtitles and
/// for a title that has none of it ([`Dress::is_bare`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Dress {
    /// The stroke about its letters.
    pub stroke: Option<Stroke>,
    /// The shadow of its letters and of their stroke.
    pub shadow: Option<Shadow>,
    /// The box behind its lines.
    pub background: Option<Backing>,
    /// The runs of its text drawn otherwise than the rest, in order and apart.
    pub runs: Vec<Run>,
}

/// A stroke: its colour, R'G'B'A, and how far beyond the edges of the letters it reaches.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Stroke {
    pub color: [u8; 4],
    pub width: f32,
}

/// A shadow: its colour, how far right and down it is moved, and the standard deviation of its
/// blur.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Shadow {
    pub color: [u8; 4],
    pub x: f32,
    pub y: f32,
    pub blur: f32,
}

/// A box behind the lines: its colour, how far it reaches beyond them, and the radius of its
/// corners.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Backing {
    pub color: [u8; 4],
    pub padding: f32,
    pub radius: f32,
}

/// A run of the text: its bytes, and the colour, the size (a share of the title's), the weight,
/// the slant and the family of its letters (none for the one that the engine chooses).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Run {
    pub bytes: Range<usize>,
    pub color: Option<[u8; 4]>,
    pub scale: f32,
    pub bold: bool,
    pub italic: bool,
    pub family: Option<String>,
}

impl Dress {
    /// What a title that says `text` has about its letters, drawn `scale` pixels to a pixel of
    /// the timeline.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "lengths in pixels drawn, and a share from 0.1 to 10"
    )]
    pub(crate) fn of(text: &str, style: &Styled, scale: f64) -> Dress {
        let drawn = |length: f64| ((length * scale) as f32).max(0.0);
        let moved = |length: f64| (length * scale) as f32;
        let family = (!style.font.is_empty()).then(|| style.font.clone());
        // Where each character begins, and where the text ends.
        let starts: Vec<usize> = text
            .char_indices()
            .map(|(at, _)| at)
            .chain([text.len()])
            .collect();
        let byte = |index: usize| starts.get(index).copied().unwrap_or(text.len());
        let runs = style
            .spans
            .iter()
            .map(|span| Run {
                bytes: byte(span.from)..byte(span.to),
                color: span.color.map(|color| color.0),
                scale: span.scale.unwrap_or(1.0) as f32,
                bold: span.bold,
                italic: span.italic,
                family: match &span.font {
                    Some(font) => (!font.is_empty()).then(|| font.clone()),
                    None => family.clone(),
                },
            })
            .collect();
        Dress {
            stroke: style.stroke.as_ref().map(|stroke| Stroke {
                color: stroke.color.0,
                width: drawn(stroke.width),
            }),
            shadow: style.shadow.as_ref().map(|shadow| Shadow {
                color: shadow.color.0,
                x: moved(shadow.x),
                y: moved(shadow.y),
                blur: drawn(shadow.blur),
            }),
            background: style.background.as_ref().map(|back| Backing {
                color: back.color.0,
                padding: drawn(back.padding),
                radius: drawn(back.radius),
            }),
            runs,
        }
    }

    /// Whether it has nothing: a title of none of it is drawn as titles always were.
    pub(crate) fn is_bare(&self) -> bool {
        self.stroke.is_none()
            && self.shadow.is_none()
            && self.background.is_none()
            && self.runs.is_empty()
    }

    /// How high the largest letters of a title whose letters are `size` high are.
    pub(super) fn largest(&self, size: f32) -> f32 {
        size * self.runs.iter().map(|run| run.scale).fold(1.0, f32::max)
    }

    /// How opaque all the letters of a title in a colour of opacity `alpha` are, where all are
    /// alike; none where a run's colour is of another opacity.
    fn uniform(&self, alpha: u8) -> Option<u8> {
        self.runs
            .iter()
            .all(|run| run.color.is_none_or(|color| color[3] == alpha))
            .then_some(alpha)
    }

    /// How many pixels beyond the edge of the letters their stroke may cover.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a width of some thousands of pixels at most"
    )]
    fn stroke_reach(&self) -> u32 {
        self.stroke
            .filter(|stroke| stroke.width > 0.0)
            .map_or(0, |stroke| (stroke.width.ceil() as u32).saturating_add(1))
    }

    /// The pixels about the block of the lines of a title whose letters are `size` high that its
    /// letters (a quarter of the largest beyond it, as any text's), their stroke and their
    /// shadow, moved and blurred, take beyond the block; or its box, if that reaches farther.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "lengths of some thousands of pixels at most"
    )]
    pub(super) fn pad(&self, size: f32) -> u32 {
        let overhang = (self.largest(size) / 4.0).ceil() as u32;
        let stroke = self.stroke_reach();
        let shadow = self.shadow.map_or(0, |shadow| {
            let moved = shadow.x.abs().max(shadow.y.abs()).ceil() as u32;
            let blurred = u32::try_from(planes::blur_reach(shadow.blur)).unwrap_or(u32::MAX);
            moved
                .saturating_add(blurred)
                .saturating_add(stroke)
                .saturating_add(1)
        });
        let boxed = self
            .background
            .map_or(0, |back| (back.padding.ceil() as u32).saturating_add(1));
        overhang.saturating_add(stroke.max(shadow)).max(boxed)
    }

    /// What it is known by.
    pub(crate) fn key(&self) -> DressKey {
        let mut lengths = Vec::new();
        if let Some(stroke) = self.stroke {
            let color = u32::from_be_bytes(stroke.color);
            lengths.extend([1, color, stroke.width.to_bits()]);
        }
        if let Some(shadow) = self.shadow {
            let color = u32::from_be_bytes(shadow.color);
            let moved = [shadow.x, shadow.y, shadow.blur].map(f32::to_bits);
            lengths.extend([2, color]);
            lengths.extend(moved);
        }
        if let Some(back) = self.background {
            let color = u32::from_be_bytes(back.color);
            lengths.extend([3, color, back.padding.to_bits(), back.radius.to_bits()]);
        }
        let runs = self
            .runs
            .iter()
            .map(|run| {
                (
                    run.bytes.clone(),
                    run.color,
                    run.scale.to_bits(),
                    run.bold,
                    run.italic,
                    run.family.clone(),
                )
            })
            .collect();
        DressKey { lengths, runs }
    }
}

impl Run {
    /// Its attributes: those of the rest of the text, `whole`, of letters `size` high, with its
    /// own in their place.
    fn attrs<'a>(&'a self, whole: &Attrs<'a>, size: f32) -> Attrs<'a> {
        let family = self
            .family
            .as_deref()
            .map_or(Family::SansSerif, Family::Name);
        let mut attrs = whole.clone().family(family);
        if self.bold {
            attrs = attrs.weight(Weight::BOLD);
        }
        if self.italic {
            attrs = attrs.style(Style::Italic);
        }
        if let Some([red, green, blue, alpha]) = self.color {
            attrs = attrs.color(Color::rgba(red, green, blue, alpha));
        }
        if (self.scale - 1.0).abs() > f32::EPSILON {
            let size = size * self.scale;
            attrs = attrs.metrics(Metrics::new(size, line_height(size)));
        }
        attrs
    }
}

/// The text cut into pieces for cosmic-text, each with its attributes: the runs with their own,
/// and the rest with those of the title, `whole`, of letters `size` high.
pub(super) fn pieces<'t>(
    text: &'t str,
    runs: &'t [Run],
    whole: &Attrs<'t>,
    size: f32,
) -> Vec<(&'t str, Attrs<'t>)> {
    let mut pieces = Vec::with_capacity(2 * runs.len() + 1);
    let mut done = 0;
    for run in runs {
        let from = run.bytes.start.clamp(done, text.len());
        let to = run.bytes.end.clamp(from, text.len());
        if let Some(before) = text.get(done..from).filter(|piece| !piece.is_empty()) {
            pieces.push((before, whole.clone()));
        }
        if let Some(own) = text.get(from..to).filter(|piece| !piece.is_empty()) {
            pieces.push((own, run.attrs(whole, size)));
        }
        done = to;
    }
    if let Some(after) = text.get(done..).filter(|piece| !piece.is_empty()) {
        pieces.push((after, whole.clone()));
    }
    pieces
}

/// The fonts and the caches that the letters of a draw are drawn from.
struct Ink<'f> {
    system: &'f mut FontSystem,
    glyphs: &'f mut SwashCache,
    strokes: &'f mut Strokes,
}

/// Draw the part `window` of a title laid out that has a stroke, a shadow, a box or runs of its
/// own: x0, y0, x1, y1 of its full pixels, which the caller keeps within
/// [`super::MOST_PIXELS`].
pub(super) fn draw(laid: &Laid, window: [u32; 4]) -> Raster {
    let [x0, y0, x1, y1] = window;
    let (width, height) = (x1.saturating_sub(x0), y1.saturating_sub(y0));
    let region = [
        i64::from(x0),
        i64::from(y0),
        i64::from(x0) + i64::from(width),
        i64::from(y0) + i64::from(height),
    ];
    let dress = &laid.spec.dress;
    let mut out = match &dress.background {
        Some(back) => boxed(laid, back, region),
        None => vec![[0; 4]; width as usize * height as usize],
    };
    if out.is_empty() {
        return Raster {
            width,
            height,
            row: width,
            start: 0,
            pixels: out,
        };
    }
    let mut fonts = fonts().lock().unwrap_or_else(PoisonError::into_inner);
    let Fonts {
        system,
        glyphs,
        strokes,
        ..
    } = &mut *fonts;
    // Letters larger than LARGE, and their strokes, are not kept once drawn, as the others are.
    let (mut once, mut once_strokes) = (SwashCache::new(), HashMap::new());
    let (glyphs, strokes) = if dress.largest(laid.spec.size) > LARGE {
        (&mut once, &mut once_strokes)
    } else {
        forget_glyphs_beyond(glyphs);
        forget_strokes_beyond(strokes);
        (glyphs, strokes)
    };
    let mut ink = Ink {
        system,
        glyphs,
        strokes,
    };
    if let Some(shadow) = &dress.shadow {
        shade(laid, &mut ink, shadow, region, &mut out);
    }
    let letters = body(laid, &mut ink, region);
    drop(fonts);
    for (under, over) in out.iter_mut().zip(&letters) {
        *under = over_of(*over, *under);
    }
    Raster {
        width,
        height,
        row: width,
        start: 0,
        pixels: out,
    }
}

/// Let go of the strokes of letters made and kept, once they take more than [`GLYPH_BYTES`] or
/// are more than [`OUTLINES_KEPT`].
fn forget_strokes_beyond(strokes: &mut Strokes) {
    let bytes: usize = strokes.values().flatten().map(|made| made.mask.len()).sum();
    if bytes > GLYPH_BYTES || strokes.len() > OUTLINES_KEPT {
        strokes.clear();
    }
}

/// How many pixels there are from `low` up to `high`.
fn extent(low: i64, high: i64) -> usize {
    usize::try_from(high.saturating_sub(low)).unwrap_or(0)
}

/// `rect`, x0, y0, x1, y1, made `by` larger on every side.
fn grow(rect: [i64; 4], by: i64) -> [i64; 4] {
    [rect[0] - by, rect[1] - by, rect[2] + by, rect[3] + by]
}

/// What both of two rects cover, x0, y0, x1, y1: none past its start.
fn meet(one: [i64; 4], other: [i64; 4]) -> [i64; 4] {
    let (x0, y0) = (one[0].max(other[0]), one[1].max(other[1]));
    [
        x0,
        y0,
        one[2].min(other[2]).max(x0),
        one[3].min(other[3]).max(y0),
    ]
}

/// `over` laid over `under`, both premultiplied.
#[allow(clippy::cast_possible_truncation, reason = "a byte")]
fn over_of(over: [u8; 4], under: [u8; 4]) -> [u8; 4] {
    let keep = 255 - u32::from(over[3]);
    let mut mixed = [0; 4];
    for ((mixed, top), bottom) in mixed.iter_mut().zip(over).zip(under) {
        *mixed = (u32::from(top) + u32::from(bottom) * keep / 255).min(255) as u8;
    }
    mixed
}

/// The box behind the lines of a title laid out, over the pixels `region` of its full pixels,
/// premultiplied: the block of the lines and its padding about it, with rounded corners.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    reason = "pixels of a block of text, far within the range of each; a byte"
)]
fn boxed(laid: &Laid, back: &Backing, region: [i64; 4]) -> Vec<[u8; 4]> {
    let (columns, rows) = (extent(region[0], region[2]), extent(region[1], region[3]));
    let pad = f64::from(laid.pad);
    let padding = f64::from(back.padding);
    let rect = [
        pad - padding,
        pad - padding,
        pad + f64::from(laid.block.0) + padding,
        pad + f64::from(laid.block.1) + padding,
    ];
    let [red, green, blue, alpha] = back.color.map(f64::from);
    let mut out = vec![[0_u8; 4]; columns * rows];
    for (index, pixel) in out.iter_mut().enumerate() {
        let x = (region[0] + (index % columns) as i64) as f64 + 0.5;
        let y = (region[1] + (index / columns) as i64) as f64 + 0.5;
        let seen = planes::rounded_box(rect, f64::from(back.radius), (x, y)) * alpha / 255.0;
        if seen > 0.0 {
            *pixel = [red * seen, green * seen, blue * seen, 255.0 * seen]
                .map(|value| value.round().clamp(0.0, 255.0) as u8);
        }
    }
    out
}

/// The letters that may reach into `region` of the full pixels of a title laid out, each with
/// its colour: a letter reaches at most the size of the largest, and `extra` more, beyond its
/// box.
#[allow(clippy::cast_precision_loss, reason = "pixels of a block of text")]
fn letters_near(laid: &Laid, region: [i64; 4], extra: u32) -> Vec<(PhysicalGlyph, Color)> {
    let spec = &laid.spec;
    let reach = f64::from(spec.dress.largest(spec.size)) + f64::from(extra);
    let pad = f64::from(laid.pad);
    let [x0, y0, x1, y1] = region.map(|value| value as f64);
    let (seen_x, seen_y) = (
        (x0 - pad - reach)..(x1 - pad + reach),
        (y0 - pad - reach)..(y1 - pad + reach),
    );
    let [red, green, blue, alpha] = spec.color;
    let base = Color::rgba(red, green, blue, alpha);
    let mut near = Vec::new();
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
            near.push((
                glyph.physical((0.0, run.line_y), 1.0),
                glyph.color_opt.unwrap_or(base),
            ));
        }
    }
    near
}

/// The letters of a title laid out and the stroke about them, R'G'B'A premultiplied, over the
/// pixels `region` of its full pixels (which may reach beyond them), as opaque as their colours.
/// Each letter is laid over those before it as titles always were; where their colours are of
/// more than one opacity, how opaque each pixel is is kept as they are laid.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    reason = "pixels of a block of text, far within the range of each; bytes"
)]
fn body(laid: &Laid, ink: &mut Ink<'_>, region: [i64; 4]) -> Vec<[u8; 4]> {
    let (columns, rows) = (extent(region[0], region[2]), extent(region[1], region[3]));
    let spec = &laid.spec;
    let stroke = spec.dress.stroke.filter(|stroke| stroke.width > 0.0);
    let near = letters_near(laid, region, spec.dress.stroke_reach());
    let origin = (
        i64::from(laid.pad) - region[0],
        i64::from(laid.pad) - region[1],
    );
    let uniform = spec.dress.uniform(spec.color[3]);
    let mut paint = vec![[0_u8; 4]; columns * rows];
    // How opaque the letters laid in each pixel are, by how much of it each covers, in
    // 255ths of 255ths: kept only where they are not all alike.
    let mut weights = vec![0_u16; if uniform.is_some() { 0 } else { paint.len() }];
    let (wide, high) = (columns as i64, rows as i64);
    for (physical, color) in &near {
        let opacity = u32::from(color.a());
        ink.glyphs
            .with_pixels(ink.system, physical.cache_key, *color, |x, y, pixel| {
                let across = i64::from(physical.x + x) + origin.0;
                let down = i64::from(physical.y + y) + origin.1;
                if across < 0 || down < 0 || across >= wide || down >= high {
                    return;
                }
                let at = down as usize * columns + across as usize;
                let alpha = u32::from(pixel.a());
                let keep = 255 - alpha;
                let lit = [pixel.r(), pixel.g(), pixel.b()]
                    .map(|channel| u32::from(channel) * alpha / 255);
                let under = paint[at].map(u32::from);
                paint[at] = [
                    (lit[0] + under[0] * keep / 255) as u8,
                    (lit[1] + under[1] * keep / 255) as u8,
                    (lit[2] + under[2] * keep / 255) as u8,
                    (alpha + under[3] * keep / 255) as u8,
                ];
                if let Some(weight) = weights.get_mut(at) {
                    *weight = (opacity * alpha + u32::from(*weight) * keep / 255) as u16;
                }
            });
    }
    let cover = match stroke {
        Some(stroke) => stroked(ink, &near, stroke.width, origin, (columns, rows)),
        None => Vec::new(),
    };
    finish(
        &mut paint,
        &weights,
        &cover,
        uniform,
        stroke.map(|stroke| stroke.color),
    );
    paint
}

/// Make the letters laid in `paint` as opaque as their colours, `uniform` where all are alike
/// and as `weights` say where not, and put the stroke under them, in its colour, where it
/// covers (`cover`) what they do not.
#[allow(clippy::cast_possible_truncation, reason = "bytes")]
fn finish(
    paint: &mut [[u8; 4]],
    weights: &[u16],
    cover: &[u8],
    uniform: Option<u8>,
    stroke: Option<[u8; 4]>,
) {
    let stroke = stroke.unwrap_or_default().map(u32::from);
    for (index, pixel) in paint.iter_mut().enumerate() {
        let coverage = u32::from(pixel[3]);
        let letters = match (uniform, weights.get(index)) {
            (Some(opacity), _) => {
                pixel.map(|value| (u32::from(value) * u32::from(opacity) / 255) as u8)
            }
            (None, Some(weight)) if coverage > 0 => pixel.map(|value| {
                (u32::from(value) * u32::from(*weight) / (255 * coverage)).min(255) as u8
            }),
            (None, _) => [0; 4],
        };
        let under = cover.get(index).map_or(0, |covered| {
            u32::from(*covered) * stroke[3] / 255 * (255 - coverage) / 255
        });
        *pixel = if under == 0 {
            letters
        } else {
            let [red, green, blue, alpha] = letters.map(u32::from);
            [
                red + stroke[0] * under / 255,
                green + stroke[1] * under / 255,
                blue + stroke[2] * under / 255,
                alpha + under,
            ]
            .map(|value| value.min(255) as u8)
        };
    }
}

/// How much of each pixel of a part `columns` × `rows`, whose first is `origin` before the
/// place of the letters, the stroke `width` wide about the letters `near` covers, the letters
/// with it: the stroke of each letter made once and kept.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    reason = "pixels of a block of text, far within the range of each"
)]
fn stroked(
    ink: &mut Ink<'_>,
    near: &[(PhysicalGlyph, Color)],
    width: f32,
    origin: (i64, i64),
    (columns, rows): (usize, usize),
) -> Vec<u8> {
    let mut cover = vec![0_u8; columns * rows];
    let (system, glyphs, strokes) = (&mut *ink.system, &mut *ink.glyphs, &mut *ink.strokes);
    for (physical, _) in near {
        let made = strokes
            .entry((physical.cache_key, width.to_bits()))
            .or_insert_with(|| stroke_of(system, glyphs, physical.cache_key, width));
        let Some(made) = made else { continue };
        let x = i64::from(physical.x + made.left) + origin.0;
        let y = i64::from(physical.y - made.top) + origin.1;
        let wide = made.width as i64;
        for (row, line) in made.mask.chunks(made.width.max(1)).enumerate() {
            let down = y + row as i64;
            if down < 0 || down >= rows as i64 {
                continue;
            }
            let (from, to) = (
                (-x).clamp(0, wide) as usize,
                (columns as i64 - x).clamp(0, wide) as usize,
            );
            if from >= to {
                continue;
            }
            let at = down as usize * columns + (x + from as i64) as usize;
            for (covered, value) in cover[at..at + (to - from)].iter_mut().zip(&line[from..to]) {
                *covered = (*covered).max(*value);
            }
        }
    }
    cover
}

/// The stroke `width` pixels wide about the letter of `key`, the letter with it; none for a
/// letter of no pixels.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a width of some thousands of pixels at most"
)]
fn stroke_of(
    system: &mut FontSystem,
    glyphs: &mut SwashCache,
    key: CacheKey,
    width: f32,
) -> Option<Outline> {
    let radius = (width.ceil() as usize).saturating_add(1);
    let mut made = covered(system, glyphs, key, radius)?;
    made.mask = planes::stroke(&made.mask, made.width, made.height, width);
    Some(made)
}

/// Lay the shadow of the letters of a title laid out and of their stroke over `out`, the
/// pixels `region` of its full pixels: what they cover as they are drawn, moved by the shadow,
/// blurred, in its colour.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    reason = "pixels of a block of text, far within the range of each; bytes"
)]
fn shade(laid: &Laid, ink: &mut Ink<'_>, shadow: &Shadow, region: [i64; 4], out: &mut [[u8; 4]]) {
    if shadow.color[3] == 0 {
        return;
    }
    // Moved by whole pixels, and by a share of one more, between the pixels.
    let (whole_x, whole_y) = (shadow.x.floor(), shadow.y.floor());
    let (share_x, share_y) = (shadow.x - whole_x, shadow.y - whole_y);
    let (whole_x, whole_y) = (whole_x as i64, whole_y as i64);
    let reach = i64::try_from(planes::blur_reach(shadow.blur)).unwrap_or(i64::MAX >> 8);
    let full = laid.full();
    let drawn = [0, 0, i64::from(full.0), i64::from(full.1)];
    // The letters blurred are read at the part moved back, and a pixel before it for the share;
    // they are blurred from those within the reach of the blur of those, where any are drawn.
    let looked = [
        region[0] - whole_x - 1,
        region[1] - whole_y - 1,
        region[2] - whole_x,
        region[3] - whole_y,
    ];
    let plane = meet(grow(looked, reach), grow(drawn, reach));
    let (columns, rows) = (extent(plane[0], plane[2]), extent(plane[1], plane[3]));
    if columns == 0 || rows == 0 {
        return;
    }
    let mut alpha = vec![0_u8; columns * rows];
    let source = meet(plane, drawn);
    let width = extent(source[0], source[2]);
    if width > 0 && source[3] > source[1] {
        let letters = body(laid, ink, source);
        let (left, top) = (
            (source[0] - plane[0]) as usize,
            (source[1] - plane[1]) as usize,
        );
        for (row, line) in letters.chunks(width).enumerate() {
            let at = (top + row) * columns + left;
            for (value, pixel) in alpha[at..at + width].iter_mut().zip(line) {
                *value = pixel[3];
            }
        }
    }
    planes::gaussian(&mut alpha, columns, rows, shadow.blur);
    let blurred = |x: i64, y: i64| {
        if x < plane[0] || y < plane[1] || x >= plane[2] || y >= plane[3] {
            0.0
        } else {
            f32::from(alpha[(y - plane[1]) as usize * columns + (x - plane[0]) as usize])
        }
    };
    let across = extent(region[0], region[2]);
    let [red, green, blue, opacity] = shadow.color.map(f32::from);
    for (index, pixel) in out.iter_mut().enumerate() {
        let x = region[0] + (index % across) as i64 - whole_x;
        let y = region[1] + (index / across) as i64 - whole_y;
        let value = (1.0 - share_x) * (1.0 - share_y) * blurred(x, y)
            + share_x * (1.0 - share_y) * blurred(x - 1, y)
            + (1.0 - share_x) * share_y * blurred(x, y - 1)
            + share_x * share_y * blurred(x - 1, y - 1);
        let seen = value * opacity / 255.0;
        if seen <= 0.0 {
            continue;
        }
        let keep = 1.0 - seen / 255.0;
        let under = pixel.map(f32::from);
        *pixel = [
            red * seen / 255.0 + under[0] * keep,
            green * seen / 255.0 + under[1] * keep,
            blue * seen / 255.0 + under[2] * keep,
            seen + under[3] * keep,
        ]
        .map(|value| value.round().clamp(0.0, 255.0) as u8);
    }
}

#[cfg(test)]
mod tests {
    use super::super::{draw as draw_text, lay_out, line_height, Spec, Text};
    use super::*;
    use columbina_model::project::{
        Rgba, TextAlign, TitleBackground, TitleShadow, TitleSpan, TitleStroke,
    };

    /// A title of white letters `size` pixels high, with nothing about them.
    fn plain(text: &str, size: f64) -> Text {
        Text {
            text: text.into(),
            style: Styled {
                font: String::new(),
                size,
                color: Rgba([255, 255, 255, 255]),
                x: 0.5,
                y: 0.5,
                align: TextAlign::Center,
                stroke: None,
                shadow: None,
                background: None,
                spans: Vec::new(),
            },
        }
    }

    /// A run of the characters `from` up to `to` of a title, of nothing of its own.
    fn run(from: usize, to: usize) -> TitleSpan {
        TitleSpan {
            from,
            to,
            color: None,
            scale: None,
            bold: false,
            italic: false,
            font: None,
        }
    }

    /// A title laid out at its size, a pixel to a pixel of the timeline, and the whole of it
    /// drawn: its width, and its pixels, row after row.
    #[allow(clippy::cast_possible_truncation, reason = "a size of letters")]
    fn drawn(text: &Text) -> (Laid, u32, Vec<[u8; 4]>) {
        let spec = Spec::title(text, text.style.size as f32, 1.0);
        let mut laid = lay_out(&spec);
        let (width, height) = laid.full();
        let raster = draw_text(&mut laid, [0, 0, width, height]);
        let mut pixels = Vec::new();
        for y in 0..raster.height as usize {
            let at = raster.start + y * raster.row as usize;
            pixels.extend_from_slice(&raster.pixels[at..at + raster.width as usize]);
        }
        (laid, width, pixels)
    }

    /// The smallest rectangle of the pixels that `keep` keeps: x0, y0, x1, y1, the last beyond.
    #[allow(clippy::cast_possible_truncation, reason = "pixels of a text")]
    fn bounds(pixels: &[[u8; 4]], width: u32, keep: impl Fn([u8; 4]) -> bool) -> [u32; 4] {
        let mut found = [u32::MAX, u32::MAX, 0, 0];
        for (index, pixel) in pixels.iter().enumerate() {
            if keep(*pixel) {
                let (x, y) = (index as u32 % width, index as u32 / width);
                found = [
                    found[0].min(x),
                    found[1].min(y),
                    found[2].max(x + 1),
                    found[3].max(y + 1),
                ];
            }
        }
        found
    }

    #[test]
    fn a_title_with_a_stroke_of_no_width_is_drawn_as_one_with_nothing_about_its_letters() {
        for color in [Rgba([255, 255, 255, 255]), Rgba([200, 40, 90, 128])] {
            let mut bare = plain("Columbina 标题 Ag", 48.0);
            bare.style.color = color;
            let mut dressed = bare.clone();
            dressed.style.stroke = Some(TitleStroke {
                color: Rgba([255, 0, 0, 255]),
                width: 0.0,
            });
            let (laid, _, pixels) = drawn(&bare);
            let (dressed_laid, _, dressed_pixels) = drawn(&dressed);
            assert_eq!(
                (laid.block, laid.pad),
                (dressed_laid.block, dressed_laid.pad)
            );
            let differ = pixels
                .iter()
                .zip(&dressed_pixels)
                .filter(|(one, other)| one != other)
                .count();
            assert_eq!(differ, 0, "{color:?}");
            assert_eq!(pixels.len(), dressed_pixels.len());
        }
    }

    #[test]
    fn a_stroke_lies_as_wide_as_it_is_about_a_letter_in_its_colour() {
        let mut title = plain("I", 96.0);
        title.style.stroke = Some(TitleStroke {
            color: Rgba([255, 0, 0, 255]),
            width: 6.0,
        });
        let (_, width, pixels) = drawn(&title);
        // The letters are white, the stroke red: green only where the letters are.
        let letter = bounds(&pixels, width, |pixel| pixel[1] > 128);
        let stroked = bounds(&pixels, width, |pixel| pixel[3] > 0);
        let reach = [
            letter[0] - stroked[0],
            letter[1] - stroked[1],
            stroked[2] - letter[2],
            stroked[3] - letter[3],
        ];
        for side in reach {
            assert!(
                (5..=7).contains(&side),
                "{reach:?}: {letter:?} in {stroked:?}"
            );
        }
        // Halfway down the letter, 3 pixels beyond its right edge: the stroke alone, whole.
        let middle = u32::midpoint(letter[1], letter[3]);
        let at = |x: u32| pixels[(middle * width + x) as usize];
        assert_eq!(at(letter[2] + 2), [255, 0, 0, 255]);
        assert_eq!(at(letter[0] - 3), [255, 0, 0, 255]);
        // Within the letter, the letter alone.
        assert_eq!(
            at(u32::midpoint(letter[0], letter[2])),
            [255, 255, 255, 255]
        );
        // Beyond its width, nothing.
        assert_eq!(at(letter[2] + 8), [0; 4]);
    }

    #[test]
    fn a_shadow_lies_where_it_is_moved_to_and_its_blur_spreads_it() {
        let shadowed = |blur: f64| {
            let mut title = plain("I", 64.0);
            title.style.shadow = Some(TitleShadow {
                color: Rgba([0, 0, 255, 255]),
                x: 10.0,
                y: 6.0,
                blur,
            });
            drawn(&title)
        };
        // The letters are white, the shadow blue: red only where the letters are.
        let (_, width, pixels) = shadowed(0.0);
        let letter = bounds(&pixels, width, |pixel| pixel[0] > 128);
        let shadow = bounds(&pixels, width, |pixel| pixel[2] > 128 && pixel[0] < 128);
        assert_eq!(
            [shadow[2], shadow[3]],
            [letter[2] + 10, letter[3] + 6],
            "{letter:?} {shadow:?}"
        );
        let sharp = bounds(&pixels, width, |pixel| pixel[2] > 0 && pixel[0] == 0);
        let darkest = |pixels: &[[u8; 4]]| {
            pixels
                .iter()
                .filter(|pixel| pixel[0] == 0)
                .map(|pixel| pixel[2])
                .max()
        };
        assert_eq!(darkest(&pixels), Some(255));
        // Blurred by 4, it reaches farther, and is nowhere whole beside the letters; and the
        // pad about the text keeps all of it.
        let (laid, width, pixels) = shadowed(4.0);
        let blurred = bounds(&pixels, width, |pixel| pixel[2] > 0 && pixel[0] == 0);
        assert!(blurred[2] >= sharp[2] + 4, "{sharp:?} {blurred:?}");
        assert!(blurred[3] >= sharp[3] + 4, "{sharp:?} {blurred:?}");
        assert!(darkest(&pixels).is_some_and(|darkest| darkest < 230));
        let full = laid.full();
        assert!(
            blurred[2] < full.0 && blurred[3] < full.1,
            "{blurred:?} in {full:?}"
        );
    }

    #[test]
    fn a_shadow_moved_by_half_a_pixel_lies_half_on_each() {
        let mut title = plain("I", 64.0);
        title.style.shadow = Some(TitleShadow {
            color: Rgba([0, 0, 255, 255]),
            x: 20.5,
            y: 0.0,
            blur: 0.0,
        });
        let (_, width, pixels) = drawn(&title);
        let letter = bounds(&pixels, width, |pixel| pixel[0] > 128);
        let middle = u32::midpoint(letter[1], letter[3]);
        let row = &pixels[(middle * width) as usize..((middle + 1) * width) as usize];
        // Beyond the letter, each pixel of the shadow is half the letter 20 pixels before it and
        // half the one 21 before: the letter is white, so how much of a pixel it covers is its
        // red.
        let (mut checked, mut halves) = (0, 0);
        for x in letter[2] as usize + 2..width as usize {
            let wanted = f64::midpoint(f64::from(row[x - 20][0]), f64::from(row[x - 21][0]));
            let shade = f64::from(row[x][2]);
            assert!((shade - wanted).abs() <= 1.0, "{x}: {shade} for {wanted}");
            checked += 1;
            if (64.0..=192.0).contains(&shade) {
                halves += 1;
            }
        }
        assert!(checked > 20 && halves > 0, "{checked} {halves}");
    }

    #[test]
    fn a_box_lies_behind_the_lines_and_its_padding_with_rounded_corners() {
        let boxed = |radius: f64| {
            let mut title = plain("Ij\nIjI", 48.0);
            title.style.background = Some(TitleBackground {
                color: Rgba([0, 255, 0, 255]),
                padding: 10.0,
                radius,
            });
            drawn(&title)
        };
        let (laid, width, pixels) = boxed(0.0);
        let (pad, block) = (laid.pad, laid.block);
        assert_eq!(
            bounds(&pixels, width, |pixel| pixel[3] > 0),
            [pad - 10, pad - 10, pad + block.0 + 10, pad + block.1 + 10]
        );
        // Opaque all over: green, or the letters on it.
        let inside = |x: u32, y: u32| pixels[(y * width + x) as usize];
        assert_eq!(inside(pad - 10, pad - 10), [0, 255, 0, 255]);
        assert_eq!(
            inside(pad + block.0 + 9, pad + block.1 + 9),
            [0, 255, 0, 255]
        );
        // Two lines high.
        assert_eq!(block.1, 2 * 60);
        // Its corners rounded by 12: the corner pixel clear, the middle of its side not.
        let (laid, width, pixels) = boxed(12.0);
        let (pad, block) = (laid.pad, laid.block);
        let at = |x: u32, y: u32| pixels[(y * width + x) as usize];
        assert_eq!(at(pad - 10, pad - 10), [0; 4]);
        assert_eq!(at(pad + block.0 + 9, pad + block.1 + 9), [0; 4]);
        assert_eq!(at(pad - 10, pad + block.1 / 2), [0, 255, 0, 255]);
        assert_eq!(at(pad + block.0 / 2, pad - 10), [0, 255, 0, 255]);
    }

    #[test]
    fn a_run_in_a_colour_of_its_own_colours_its_letters_only() {
        for alpha in [255, 128] {
            let mut title = plain("A  B", 48.0);
            title.style.spans = vec![TitleSpan {
                color: Some(Rgba([255, 0, 0, alpha])),
                ..run(3, 4)
            }];
            let (_, width, pixels) = drawn(&title);
            let middle = width / 2;
            let (mut white, mut red) = (0, 0);
            for (index, pixel) in pixels.iter().enumerate() {
                if pixel[3] == 0 {
                    continue;
                }
                let x = u32::try_from(index).unwrap() % width;
                if x < middle {
                    // White, as opaque as the title.
                    assert_eq!([pixel[0], pixel[1]], [pixel[3], pixel[3]], "{x}: {pixel:?}");
                    white += 1;
                } else {
                    assert_eq!([pixel[1], pixel[2]], [0, 0], "{x}: {pixel:?}");
                    assert!(pixel[3] <= alpha, "{x}: {pixel:?}");
                    red += 1;
                }
            }
            assert!(white > 50 && red > 50, "{white} white, {red} red");
            // As opaque as its own colour where it covers whole.
            let most = pixels.iter().map(|pixel| pixel[3]).max();
            assert_eq!(most, Some(255));
            let red_most = pixels
                .iter()
                .enumerate()
                .filter(|(index, _)| u32::try_from(*index).unwrap() % width >= middle)
                .map(|(_, pixel)| pixel[3])
                .max();
            assert_eq!(red_most, Some(alpha));
        }
    }

    #[test]
    fn a_run_of_twice_the_size_has_letters_twice_as_high_in_a_line_as_high_as_they_need() {
        let plain_title = plain("I", 48.0);
        let mut large = plain_title.clone();
        large.style.spans = vec![TitleSpan {
            scale: Some(2.0),
            ..run(0, 1)
        }];
        let (plain_laid, plain_width, plain_pixels) = drawn(&plain_title);
        let (large_laid, large_width, large_pixels) = drawn(&large);
        let high = |pixels: &[[u8; 4]], width: u32| {
            let found = bounds(pixels, width, |pixel| pixel[3] > 128);
            f64::from(found[3] - found[1])
        };
        let ratio = high(&large_pixels, large_width) / high(&plain_pixels, plain_width);
        assert!((1.85..=2.15).contains(&ratio), "{ratio}");
        assert_eq!(plain_laid.block.1, 60);
        assert_eq!(large_laid.block.1, 120);
        // Its letters may reach a quarter of their own size beyond the block.
        assert_eq!(large_laid.pad, 24);
    }

    #[test]
    fn runs_are_cut_from_the_text_by_characters() {
        let mut title = plain("标题ab", 48.0);
        title.style.spans = vec![run(1, 3)];
        title.style.stroke = Some(TitleStroke {
            color: Rgba([0, 0, 0, 255]),
            width: 4.0,
        });
        let dress = Dress::of(&title.text, &title.style, 2.0);
        assert_eq!(dress.runs[0].bytes, 3..7);
        // Its lengths are pixels drawn: twice those of the timeline here.
        assert_eq!(
            dress.stroke,
            Some(Stroke {
                color: [0, 0, 0, 255],
                width: 8.0,
            })
        );
        let whole = Attrs::new();
        let pieces: Vec<&str> = pieces(&title.text, &dress.runs, &whole, 48.0)
            .into_iter()
            .map(|(piece, _)| piece)
            .collect();
        assert_eq!(pieces, ["标", "题a", "b"]);
        // A run's family is the title's where it names none, and the engine's where it names
        // an empty one.
        title.style.font = "Georgia".into();
        title.style.spans = vec![
            run(0, 1),
            TitleSpan {
                font: Some(String::new()),
                ..run(1, 2)
            },
        ];
        let dress = Dress::of(&title.text, &title.style, 1.0);
        assert_eq!(dress.runs[0].family.as_deref(), Some("Georgia"));
        assert_eq!(dress.runs[1].family, None);
        assert_ne!(dress.key(), Dress::of(&title.text, &title.style, 2.0).key());
        assert!((line_height(96.0) - 120.0).abs() < f32::EPSILON);
    }
}
