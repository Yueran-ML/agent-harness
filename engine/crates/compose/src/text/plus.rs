//! The Text+ of a composition drawn on the processor ([`TextPlused`]): its text laid out by
//! cosmic-text in the fonts that titles are drawn in, and each glyph that is written drawn
//! through the map of its character into the picture of the size that the pictures of the
//! composition are drawn at, which the compositor puts on the card. Only the block of the
//! picture that something covers is drawn ([`Lettering`]); the rest of it is transparent.
//!
//! # The layout
//!
//! The text is cut into its lines at its line breaks, and each line is shaped alone, in the
//! family of its font (the engine's own where it names none), its em `size` heights of the
//! picture: so a line is a quarter higher than its em, as a line of a title is. The lines are
//! `line_spacing` of those apart; each character is moved `tracking` ems more than the one
//! before it in its line; each line lies across about the centre as `justify` says, its width
//! that of its glyphs from its start, and the block of the lines up and down as `vertical`
//! says, its height from the top of its first line to the bottom of its last. The cell of a
//! glyph is its advance across and its line down: the box of a unit is that of the cells of
//! its glyphs, and the box of the whole that of all of them.
//!
//! # The map of a glyph
//!
//! A glyph is put in the picture by the map of its character: made `scale` (that of the
//! transform of the units times the follower's) times as large and turned by `angle` (the
//! transform's and the follower's) about the middle of the box of its unit, moved by the
//! offsets of both, and then turned with the whole by `angle` about the centre. It is drawn at
//! the size that it is made, up to an em of [`LARGEST_EM`] pixels, so that it is as sharp at
//! any size as a title is; and each pixel of the picture reads it where the middle of the
//! pixel lies in it, between the pixels of its image (bilinear). A glyph that is only moved is
//! drawn between pixels across by cosmic-text and put on the nearest pixel down, as the glyphs
//! of a title are, so that each pixel of its image is a pixel of the picture.
//!
//! # The shading
//!
//! Over the box of the border, turned with the whole, the outlines of the glyphs that are
//! written are laid, one over another in the order of the text, and then their fills: so the
//! outline of a glyph never lies over the fill of another. An outline is the stroke of a title
//! about its glyph ([`super::planes::stroke`]), `thickness` ems of the glyph wide as it is made,
//! which covers the glyph too: an opaque fill over it hides what of it lies within the glyph.
//! The follower's opacity multiplies both. The colours are held within 0 and 1 and
//! premultiplied, as that of a Background is, and laid each over what is under it.

use std::collections::HashMap;
use std::sync::PoisonError;

use columbina_model::fusion::{
    Bordered, Followed, TextJustify, TextPlused, TextSelect, TextVertical,
};
use cosmic_text::{
    Attrs, Buffer, CacheKey, Family, FontSystem, LayoutGlyph, Metrics, Shaping, SwashCache,
};

use super::{covered, fonts, forget_glyphs_beyond, planes, Fonts, Outline, LARGE};
use crate::geometry::Affine;

/// The largest em that a glyph is drawn at, in pixels: one made larger is drawn at this and
/// read between its pixels as large as it is made.
const LARGEST_EM: f64 = 1024.0;

/// The smallest em that a glyph is drawn at, in pixels: one made smaller is not seen.
const SMALLEST_EM: f64 = 0.25;

/// How many times as high as its em a line of a Text+ is, as a line of a title is.
const LINE: f64 = 1.25;

/// The pixels of the picture of a Text+ that something covers: the block of `width` ×
/// `height` of them whose left and top pixel is `at`, R, G, B and A premultiplied, row after
/// row. Every other pixel of the picture is transparent.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Lettering {
    pub at: (u32, u32),
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<[f32; 4]>,
}

impl Lettering {
    /// Its pixels as half floats, R, G, B and A, row after row, as the pictures of a
    /// composition hold them.
    pub(crate) fn halves(&self) -> Vec<u16> {
        self.pixels
            .iter()
            .flatten()
            .map(|value| half_bits(*value))
            .collect()
    }
}

/// The bits of the half float nearest `value`, half way to the even one; 0 for what is no
/// number, and the infinity of its sign beyond the largest half float.
fn half_bits(value: f32) -> u16 {
    if value.is_nan() {
        return 0;
    }
    let bits = value.to_bits();
    let sign = u16::try_from((bits >> 16) & 0x8000).unwrap_or(0);
    let exponent = i64::from((bits >> 23) & 0xFF) - 127 + 15;
    let fraction = bits & 0x007F_FFFF;
    if exponent >= 0x1F {
        return sign | 0x7C00;
    }
    let (kept, shift) = if exponent <= 0 {
        if exponent < -10 {
            return sign;
        }
        (
            fraction | 0x0080_0000,
            u32::try_from(14 - exponent).unwrap_or(24),
        )
    } else {
        (fraction, 13)
    };
    let mut half = kept >> shift;
    let rest = kept & ((1 << shift) - 1);
    let middle = 1 << (shift - 1);
    if rest > middle || (rest == middle && half & 1 == 1) {
        half += 1;
    }
    let exponent = u32::try_from(exponent.max(0)).unwrap_or(0);
    u16::try_from((exponent << 10) + half).map_or(sign | 0x7C00, |magnitude| sign | magnitude)
}

/// What the picture of a Text+ drawn some pixels large is known by: all that decides it
/// ([`key`]).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct LetteringKey {
    text: String,
    font: String,
    numbers: Vec<u64>,
}

/// Put the bits of each of `values` after `numbers`.
fn bits(numbers: &mut Vec<u64>, values: &[f64]) {
    numbers.extend(values.iter().map(|value| value.to_bits()));
}

/// What the picture of a Text+ of `settings` drawn `size` pixels large is known by: the size,
/// its text, its font and each of its numbers, its follower's for each character too.
pub(crate) fn key(settings: &TextPlused, size: (u32, u32)) -> LetteringKey {
    let mut numbers = vec![
        u64::from(size.0),
        u64::from(size.1),
        settings.justify as u64,
        settings.vertical as u64,
        settings.transform.select as u64,
    ];
    numbers.extend(
        settings
            .written
            .map(|at| u64::try_from(at).unwrap_or(u64::MAX)),
    );
    let transform = settings.transform;
    bits(
        &mut numbers,
        &[
            settings.size,
            settings.center[0],
            settings.center[1],
            settings.line_spacing,
            settings.tracking,
            settings.angle,
            transform.offset[0],
            transform.offset[1],
            transform.angle,
            transform.scale,
        ],
    );
    bits(&mut numbers, &settings.fill);
    if let Some(outline) = settings.outline {
        numbers.push(1);
        bits(&mut numbers, &outline.color);
        bits(&mut numbers, &[outline.thickness]);
    }
    if let Some(border) = settings.border {
        numbers.push(2);
        bits(&mut numbers, &border.color);
        bits(&mut numbers, &[border.extend, border.round]);
    }
    for followed in &settings.followed {
        let Followed {
            opacity,
            offset,
            angle,
            scale,
        } = *followed;
        bits(&mut numbers, &[opacity, offset[0], offset[1], angle, scale]);
    }
    LetteringKey {
        text: settings.text.clone(),
        font: settings.font.clone(),
        numbers,
    }
}

/// A turn of the plane, as a matrix that puts the point (x, y) at (`[0][0]` x + `[0][1]` y,
/// `[1][0]` x + `[1][1]` y).
type Turn = [[f64; 2]; 2];

/// A turn by `degrees` counter-clockwise as the picture is seen, in pixels whose `y` is down:
/// a point (x, y) is put at (x cos + y sin, −x sin + y cos).
fn turn(degrees: f64) -> Turn {
    let (sin, cos) = degrees.to_radians().sin_cos();
    [[cos, sin], [-sin, cos]]
}

/// The matrix that does `inner` and then `outer`.
fn times(outer: Turn, inner: Turn) -> Turn {
    let cell = |row: usize, column: usize| {
        outer[row][0] * inner[0][column] + outer[row][1] * inner[1][column]
    };
    [[cell(0, 0), cell(0, 1)], [cell(1, 0), cell(1, 1)]]
}

/// Where `matrix` puts `point`.
fn put(matrix: Turn, point: [f64; 2]) -> [f64; 2] {
    [
        matrix[0][0] * point[0] + matrix[0][1] * point[1],
        matrix[1][0] * point[0] + matrix[1][1] * point[1],
    ]
}

/// What the glyphs of a Text+ are put in its picture by.
struct Frame {
    /// The centre, in pixels of the picture from its top left corner.
    anchor: [f64; 2],
    /// The turn of the whole about the centre.
    whole: Turn,
    /// The height of the picture, in pixels: what the size and the offsets are shares of.
    height: f64,
    /// The em of its letters, in pixels.
    em: f64,
}

/// A glyph as it is laid out: where its pen is, from the centre of the text, across and down to
/// its baseline, in pixels of the picture; the character that it is of, by its place among the
/// characters of the text; and its cell, x0, y0, x1, y1 from the centre.
struct Placed {
    glyph: LayoutGlyph,
    pen: [f64; 2],
    character: usize,
    cell: [f64; 4],
}

/// A Text+ laid out: its glyphs, line after line, and the unit of each of its characters, by
/// their places.
struct Layout {
    glyphs: Vec<Placed>,
    units: Vec<usize>,
}

/// The box of the border of a Text+ laid out, x0, y0, x1, y1 from its centre, the radius of its
/// corners, and its colour, premultiplied.
struct Backing {
    rect: [f64; 4],
    radius: f64,
    colour: [f64; 4],
}

/// How much of each pixel of an image a glyph, or its outline, covers: `columns` × `rows` of
/// them, row after row; and the map from a place of the image, in its pixels from its top left
/// corner, to a place of the picture.
struct Plane {
    columns: usize,
    rows: usize,
    cover: Vec<u8>,
    map: Affine,
}

/// A glyph that is written, to be drawn: its outline and its fill, where they cover anything,
/// and what its alpha is multiplied by.
struct Inked {
    outline: Option<Plane>,
    fill: Option<Plane>,
    opacity: f64,
}

/// Draw the picture of a Text+ of `settings`, of `size` pixels: the block of it that something
/// covers, or none where nothing does (its text is empty, Write On draws none of it, or its
/// letters are less than a quarter of a pixel high).
pub(crate) fn draw(settings: &TextPlused, size: (u32, u32)) -> Option<Lettering> {
    let (width, height) = (f64::from(size.0), f64::from(size.1));
    let em = settings.size * height;
    if !settings.writes_any() || !(SMALLEST_EM..f64::INFINITY).contains(&em) {
        return None;
    }
    let frame = Frame {
        anchor: [
            settings.center[0] * width,
            (1.0 - settings.center[1]) * height,
        ],
        whole: turn(settings.angle),
        height,
        em,
    };
    let (layout, inked) = {
        let mut fonts = fonts().lock().unwrap_or_else(PoisonError::into_inner);
        let Fonts { system, glyphs, .. } = &mut *fonts;
        let layout = lay_out(system, settings, em);
        let inked = ink(system, glyphs, settings, &layout, &frame);
        (layout, inked)
    };
    let backing = settings.border.and_then(|border| {
        let (rect, radius) = border_box(&layout, &border, height)?;
        Some(Backing {
            rect,
            radius,
            colour: premultiplied(border.color),
        })
    });
    let region = reach_of(&inked, backing.as_ref(), &frame, size)?;
    let (columns, rows) = (region[2] - region[0], region[3] - region[1]);
    let mut pixels = vec![[0.0_f32; 4]; columns as usize * rows as usize];
    if let Some(back) = &backing {
        boxed(&mut pixels, region, back, &frame);
    }
    if let Some(outline) = settings.outline {
        let colour = premultiplied(outline.color);
        for glyph in &inked {
            if let Some(plane) = &glyph.outline {
                lay(&mut pixels, region, plane, colour, glyph.opacity);
            }
        }
    }
    let fill = premultiplied(settings.fill);
    for glyph in &inked {
        if let Some(plane) = &glyph.fill {
            lay(&mut pixels, region, plane, fill, glyph.opacity);
        }
    }
    Some(Lettering {
        at: (region[0], region[1]),
        width: columns,
        height: rows,
        pixels,
    })
}

/// Lay out the text of `settings`, its em `em` pixels (see the [module](self)).
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    reason = "an em of some thousands of pixels at most, as cosmic-text takes it; counts of \
              lines and characters"
)]
fn lay_out(system: &mut FontSystem, settings: &TextPlused, em: f64) -> Layout {
    let natural = em * LINE;
    let pitch = natural * settings.line_spacing.max(0.0);
    let tracking = settings.tracking * em;
    let metrics = Metrics::new(em as f32, natural as f32);
    let attrs = if settings.font.is_empty() {
        Attrs::new().family(Family::SansSerif)
    } else {
        Attrs::new().family(Family::Name(&settings.font))
    };
    let lines: Vec<&str> = settings.text.split('\n').collect();
    let block = lines.len().saturating_sub(1) as f64 * pitch + natural;
    let top = match settings.vertical {
        TextVertical::Top => 0.0,
        TextVertical::Center => -block / 2.0,
        TextVertical::Bottom => -block,
    };
    let mut layout = Layout {
        glyphs: Vec::new(),
        units: Vec::new(),
    };
    let mut first = 0;
    for (number, line) in lines.iter().enumerate() {
        let select = settings.transform.select;
        let of_byte = characters_of_line(line, number, first, select, &mut layout.units);
        let line_top = top + number as f64 * pitch;
        // Each glyph moved by the tracking of the characters before its own in the line.
        let tracked: Vec<(LayoutGlyph, f64, usize, f64)> = shaped(system, line, &attrs, metrics)
            .into_iter()
            .map(|(glyph, baseline)| {
                let character = of_byte.get(glyph.start).copied().unwrap_or(first);
                let shift = tracking * (character - first) as f64;
                (glyph, f64::from(baseline), character, shift)
            })
            .collect();
        let wide = tracked
            .iter()
            .map(|(glyph, _, _, shift)| shift + f64::from(glyph.x + glyph.w))
            .fold(0.0, f64::max);
        let left = match settings.justify {
            TextJustify::Left => 0.0,
            TextJustify::Center => -wide / 2.0,
            TextJustify::Right => -wide,
        };
        for (glyph, baseline, character, shift) in tracked {
            let pen = [left + shift, line_top + baseline];
            let start = pen[0] + f64::from(glyph.x);
            let cell = [
                start,
                line_top,
                start + f64::from(glyph.w),
                line_top + natural,
            ];
            layout.glyphs.push(Placed {
                glyph,
                pen,
                character,
                cell,
            });
        }
        first += line.chars().count();
    }
    layout
}

/// The character of each byte of `line`, the line `number` whose first character is at the
/// place `first` among those of the text; and, put after `units`, the unit of each of its
/// characters by `select`: its own place, the place of the first character of its word
/// (whitespace being each a unit of its own), or the number of its line.
fn characters_of_line(
    line: &str,
    number: usize,
    first: usize,
    select: TextSelect,
    units: &mut Vec<usize>,
) -> Vec<usize> {
    let mut of_byte = vec![first; line.len() + 1];
    let mut word = None;
    for (index, (byte, character)) in line.char_indices().enumerate() {
        let place = first + index;
        of_byte[byte..byte + character.len_utf8()].fill(place);
        let unit = match select {
            TextSelect::Characters => place,
            TextSelect::Words if character.is_whitespace() => {
                word = None;
                place
            }
            TextSelect::Words => *word.get_or_insert(place),
            TextSelect::Lines => number,
        };
        units.push(unit);
    }
    of_byte
}

/// The glyphs of a line shaped alone in `attrs` and `metrics`, each with how far its baseline
/// is below the top of its line. The separators that cosmic-text would break a line at are
/// shaped as spaces of as many bytes, so that the line is one and its bytes are where they
/// were.
fn shaped(
    system: &mut FontSystem,
    line: &str,
    attrs: &Attrs<'_>,
    metrics: Metrics,
) -> Vec<(LayoutGlyph, f32)> {
    let plain: String = line
        .chars()
        .map(|character| match character {
            '\r' | '\u{0B}' | '\u{0C}' => ' ',
            '\u{85}' => '\u{A0}',
            '\u{2028}' | '\u{2029}' => '\u{2002}',
            other => other,
        })
        .collect();
    let mut buffer = Buffer::new(system, metrics);
    buffer.set_size(None, None);
    buffer.set_text(&plain, attrs, Shaping::Advanced, None);
    buffer.shape_until_scroll(system, false);
    buffer
        .layout_runs()
        .flat_map(|run| {
            let baseline = run.line_y - run.line_top;
            run.glyphs
                .iter()
                .map(move |glyph| (glyph.clone(), baseline))
        })
        .collect()
}

/// `one` and `other` made one box, x0, y0, x1, y1: the least that holds both.
fn union(one: [f64; 4], other: [f64; 4]) -> [f64; 4] {
    [
        one[0].min(other[0]),
        one[1].min(other[1]),
        one[2].max(other[2]),
        one[3].max(other[3]),
    ]
}

/// The box of each unit of a Text+ laid out, by its unit: that of the cells of its glyphs.
fn unit_boxes(layout: &Layout) -> HashMap<usize, [f64; 4]> {
    let mut boxes: HashMap<usize, [f64; 4]> = HashMap::new();
    for placed in &layout.glyphs {
        let Some(&unit) = layout.units.get(placed.character) else {
            continue;
        };
        boxes
            .entry(unit)
            .and_modify(|cell| *cell = union(*cell, placed.cell))
            .or_insert(placed.cell);
    }
    boxes
}

/// The glyphs of a Text+ laid out that are written, each with the planes of its outline and of
/// its fill drawn at the size that it is made, and what its alpha is multiplied by. A glyph
/// that its follower makes transparent, or too small to be seen, is left out.
#[allow(
    clippy::cast_possible_truncation,
    reason = "a scale of an em of at most LARGEST_EM pixels, as cosmic-text takes it"
)]
fn ink(
    system: &mut FontSystem,
    glyphs: &mut SwashCache,
    settings: &TextPlused,
    layout: &Layout,
    frame: &Frame,
) -> Vec<Inked> {
    let boxes = unit_boxes(layout);
    let [first, last] = settings.written;
    // Glyphs drawn larger than LARGE are not kept once drawn, as the others are.
    let mut once = SwashCache::new();
    forget_glyphs_beyond(glyphs);
    let mut all = Vec::new();
    for placed in &layout.glyphs {
        if !(first..last).contains(&placed.character) {
            continue;
        }
        let followed = settings.followed_at(placed.character);
        let scale = settings.transform.scale * followed.scale;
        let opacity = followed.opacity.min(1.0);
        let seen = (SMALLEST_EM..f64::INFINITY).contains(&(scale * frame.em));
        if !seen || opacity.is_nan() || opacity <= 0.0 {
            continue;
        }
        let raster = scale.min(LARGEST_EM / frame.em);
        let around = layout
            .units
            .get(placed.character)
            .and_then(|unit| boxes.get(unit))
            .copied()
            .unwrap_or(placed.cell);
        let centre = [
            f64::midpoint(around[0], around[2]),
            f64::midpoint(around[1], around[3]),
        ];
        let Some(map) = glyph_map(frame, settings, placed, centre, followed, raster) else {
            continue;
        };
        let (map, nudge) = snapped(map);
        let physical = placed.glyph.physical((nudge, 0.0), raster as f32);
        let cache = if frame.em * raster > f64::from(LARGE) {
            &mut once
        } else {
            &mut *glyphs
        };
        let at = (physical.x, physical.y);
        let fill = covered(system, cache, physical.cache_key, 0).map(|made| plane(made, at, &map));
        let width = settings
            .outline
            .map_or(0.0, |outline| outline.thickness * frame.em * raster);
        let outline = if width > 0.0 {
            stroked(system, cache, physical.cache_key, width).map(|made| plane(made, at, &map))
        } else {
            None
        };
        all.push(Inked {
            outline,
            fill,
            opacity,
        });
    }
    all
}

/// The map from a place of a glyph drawn `raster` times as large as it is laid out, in the
/// pixels that it is drawn in from its pen, to a place of the picture: made as large and
/// turned as its character is about `centre`, the middle of the box of its unit, moved by the
/// offsets, and turned with the whole about the centre of the text. None where it flattens the
/// glyph.
fn glyph_map(
    frame: &Frame,
    settings: &TextPlused,
    placed: &Placed,
    centre: [f64; 2],
    followed: Followed,
    raster: f64,
) -> Option<Affine> {
    let transform = settings.transform;
    let scale = transform.scale * followed.scale;
    let own = turn(transform.angle + followed.angle);
    let linear = times(frame.whole, own).map(|row| row.map(|cell| cell * scale));
    // The offsets are shares of the height of the picture, `y` up.
    let offset = [
        (transform.offset[0] + followed.offset[0]) * frame.height,
        -(transform.offset[1] + followed.offset[1]) * frame.height,
    ];
    let moved = put(frame.whole, [centre[0] + offset[0], centre[1] + offset[1]]);
    let kept = put(linear, centre);
    let pen = put(linear, placed.pen);
    let map = Affine {
        a: linear[0][0] / raster,
        b: linear[0][1] / raster,
        c: linear[1][0] / raster,
        d: linear[1][1] / raster,
        tx: frame.anchor[0] + moved[0] - kept[0] + pen[0],
        ty: frame.anchor[1] + moved[1] - kept[1] + pen[1],
    };
    map.inverse().map(|_| map)
}

/// `map`, where it only moves a glyph, made to move it by whole pixels: across by the whole
/// pixels of its move, the share of a pixel that is left given to cosmic-text to draw the glyph
/// at (the nudge), as the glyphs of a title are drawn between pixels; and down to the nearest
/// pixel, as the lines of a title are put. So a glyph that is not turned nor made larger is as
/// sharp as that of a title, each pixel of its image a pixel of the picture. Any other map is
/// as it is, with no nudge.
#[allow(clippy::cast_possible_truncation, reason = "a share of a pixel")]
fn snapped(map: Affine) -> (Affine, f32) {
    let near = |value: f64, wanted: f64| (value - wanted).abs() < 1e-9;
    if !(near(map.a, 1.0) && near(map.d, 1.0) && near(map.b, 0.0) && near(map.c, 0.0)) {
        return (map, 0.0);
    }
    let across = map.tx.floor();
    let moved = Affine {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        tx: across,
        ty: map.ty.round(),
    };
    (moved, (map.tx - across) as f32)
}

/// The plane of `made`, the image of a glyph whose pen is at `at` among the pixels that it is
/// drawn in (as cosmic-text places the image of a glyph), put in the picture by `map`.
fn plane(made: Outline, at: (i32, i32), map: &Affine) -> Plane {
    let origin = (
        f64::from(at.0) + f64::from(made.left),
        f64::from(at.1) - f64::from(made.top),
    );
    Plane {
        columns: made.width,
        rows: made.height,
        cover: made.mask,
        map: Affine {
            tx: map.a * origin.0 + map.b * origin.1 + map.tx,
            ty: map.c * origin.0 + map.d * origin.1 + map.ty,
            ..*map
        },
    }
}

/// How much of each pixel the stroke `width` pixels wide about the glyph of `key` covers, the
/// glyph with it, as the stroke of a title is made; none for a glyph of no pixels.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a width of some hundreds of pixels at most"
)]
fn stroked(
    system: &mut FontSystem,
    glyphs: &mut SwashCache,
    key: CacheKey,
    width: f64,
) -> Option<Outline> {
    let radius = (width.ceil() as usize).saturating_add(1);
    let mut made = covered(system, glyphs, key, radius)?;
    made.mask = planes::stroke(&made.mask, made.width, made.height, width as f32);
    Some(made)
}

/// The box of the border of a Text+ laid out, x0, y0, x1, y1 from its centre, and the radius of
/// its corners: the box of the cells of all its glyphs grown by `extend` heights of the picture
/// on every side, its corners rounded by `round` of half its shorter side. None of a text of no
/// glyphs.
fn border_box(layout: &Layout, border: &Bordered, height: f64) -> Option<([f64; 4], f64)> {
    let block = layout
        .glyphs
        .iter()
        .map(|placed| placed.cell)
        .reduce(union)?;
    let grown = border.extend.max(0.0) * height;
    let rect = [
        block[0] - grown,
        block[1] - grown,
        block[2] + grown,
        block[3] + grown,
    ];
    let shorter = (rect[2] - rect[0]).min(rect[3] - rect[1]);
    Some((rect, border.round.clamp(0.0, 1.0) * shorter / 2.0))
}

/// The corners of the image of a plane, in its pixels.
#[allow(clippy::cast_precision_loss, reason = "sides of an image of a glyph")]
fn corners(plane: &Plane) -> [(f64, f64); 4] {
    let (columns, rows) = (plane.columns as f64, plane.rows as f64);
    [(0.0, 0.0), (columns, 0.0), (0.0, rows), (columns, rows)]
}

/// `bounds` grown to hold the point (x, y), where it is a place.
fn spread(bounds: &mut Option<[f64; 4]>, (x, y): (f64, f64)) {
    if !(x.is_finite() && y.is_finite()) {
        return;
    }
    let point = [x, y, x, y];
    *bounds = Some(bounds.map_or(point, |within| union(within, point)));
}

/// The pixels of `within`, x0, y0, x1, y1 the last beyond, that `bounds` reaches into, and a
/// pixel more on each side for what is read between pixels; none where it reaches none.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "places held within a picture"
)]
fn pixels_within(bounds: [f64; 4], within: [u32; 4]) -> Option<[u32; 4]> {
    let held =
        |value: f64, low: u32, high: u32| value.clamp(f64::from(low), f64::from(high)) as u32;
    let pixels = [
        held(bounds[0].floor() - 1.0, within[0], within[2]),
        held(bounds[1].floor() - 1.0, within[1], within[3]),
        held(bounds[2].ceil() + 1.0, within[0], within[2]),
        held(bounds[3].ceil() + 1.0, within[1], within[3]),
    ];
    (pixels[0] < pixels[2] && pixels[1] < pixels[3]).then_some(pixels)
}

/// The pixels of a picture of `size` that a Text+ may cover, x0, y0, x1, y1 the last beyond:
/// those that its border and the images of its glyphs reach; none where they reach none.
fn reach_of(
    inked: &[Inked],
    backing: Option<&Backing>,
    frame: &Frame,
    size: (u32, u32),
) -> Option<[u32; 4]> {
    let mut bounds = None;
    if let Some(back) = backing {
        let [x0, y0, x1, y1] = back.rect;
        for corner in [[x0, y0], [x1, y0], [x0, y1], [x1, y1]] {
            let turned = put(frame.whole, corner);
            spread(
                &mut bounds,
                (frame.anchor[0] + turned[0], frame.anchor[1] + turned[1]),
            );
        }
    }
    for glyph in inked {
        for plane in glyph.outline.iter().chain(&glyph.fill) {
            for corner in corners(plane) {
                spread(&mut bounds, plane.map.apply(corner));
            }
        }
    }
    pixels_within(bounds?, [0, 0, size.0, size.1])
}

/// A colour, red, green, blue and alpha, held within 0 and 1 and premultiplied.
fn premultiplied(rgba: [f64; 4]) -> [f64; 4] {
    let held = rgba.map(|channel| if channel > 0.0 { channel.min(1.0) } else { 0.0 });
    [
        held[0] * held[3],
        held[1] * held[3],
        held[2] * held[3],
        held[3],
    ]
}

/// `colour`, premultiplied, laid over `pixel`.
#[allow(
    clippy::cast_possible_truncation,
    reason = "a colour from 0 to 1, as the card takes it"
)]
fn over(pixel: &mut [f32; 4], colour: [f64; 4]) {
    let keep = 1.0 - colour[3];
    for (under, own) in pixel.iter_mut().zip(colour) {
        *under = (own + f64::from(*under) * keep) as f32;
    }
}

/// How much of the place (`across`, `down`) of its image, in its pixels from its top left
/// corner, a plane covers: between the four pixels whose middles are about it, nothing beyond
/// its edges. Between four that are alike it is theirs exactly, so that a glyph covers whole
/// where it covers whole.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a place within an image, checked to be"
)]
fn sampled(plane: &Plane, across: f64, down: f64) -> f64 {
    let (column, row) = (across - 0.5, down - 0.5);
    let (left, top) = (column.floor(), row.floor());
    let (right_share, lower_share) = (column - left, row - top);
    let texel = |column: f64, row: f64| {
        if column < 0.0 || row < 0.0 {
            return 0.0;
        }
        let (column, row) = (column as usize, row as usize);
        if column >= plane.columns || row >= plane.rows {
            return 0.0;
        }
        f64::from(plane.cover[row * plane.columns + column]) / 255.0
    };
    let between = |first: f64, second: f64, share: f64| first + (second - first) * share;
    let upper = between(texel(left, top), texel(left + 1.0, top), right_share);
    let lower = between(
        texel(left, top + 1.0),
        texel(left + 1.0, top + 1.0),
        right_share,
    );
    between(upper, lower, lower_share)
}

/// Lay `plane` in `colour`, premultiplied, its alpha multiplied by `opacity`, over `out`, the
/// pixels `region` of the picture: each pixel as much as the plane covers where its middle
/// lies in it.
#[allow(clippy::cast_possible_truncation, reason = "places within a picture")]
fn lay(out: &mut [[f32; 4]], region: [u32; 4], plane: &Plane, colour: [f64; 4], opacity: f64) {
    let Some(back) = plane.map.inverse() else {
        return;
    };
    let mut bounds = None;
    for corner in corners(plane) {
        spread(&mut bounds, plane.map.apply(corner));
    }
    let Some(within) = bounds.and_then(|bounds| pixels_within(bounds, region)) else {
        return;
    };
    let columns = (region[2] - region[0]) as usize;
    for row in within[1]..within[3] {
        for column in within[0]..within[2] {
            let (across, down) = back.apply((f64::from(column) + 0.5, f64::from(row) + 0.5));
            let cover = sampled(plane, across, down) * opacity;
            if cover <= 0.0 {
                continue;
            }
            let at = (row - region[1]) as usize * columns + (column - region[0]) as usize;
            over(&mut out[at], colour.map(|channel| channel * cover));
        }
    }
}

/// Lay the border `back` over `out`, the pixels `region` of the picture: each pixel as much as
/// the box covers where its middle lies in it, turned back about the centre of the text.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    reason = "places within a picture"
)]
fn boxed(out: &mut [[f32; 4]], region: [u32; 4], back: &Backing, frame: &Frame) {
    let columns = (region[2] - region[0]) as usize;
    let whole = frame.whole;
    let undone = [[whole[0][0], whole[1][0]], [whole[0][1], whole[1][1]]];
    for (at, pixel) in out.iter_mut().enumerate() {
        let across = f64::from(region[0]) + (at % columns) as f64 + 0.5;
        let down = f64::from(region[1]) + (at / columns) as f64 + 0.5;
        let from = put(undone, [across - frame.anchor[0], down - frame.anchor[1]]);
        let cover = planes::rounded_box(back.rect, back.radius, (from[0], from[1]));
        if cover > 0.0 {
            over(pixel, back.colour.map(|channel| channel * cover));
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::float_cmp,
        clippy::cast_precision_loss,
        reason = "colours drawn opaque, which floats hold exactly; places of small pictures"
    )]

    use columbina_model::fusion::{FollowerOrder, Outlined, TextFollower, TextPlus};
    use columbina_model::grade::{Interp, Key, Param};

    use super::*;

    /// The size of the pictures of the tests: letters of a quarter of its height are 24 pixels
    /// high.
    const SIZE: (u32, u32) = (192, 96);

    /// A Text+ that says `text`, its em a quarter of the height of the picture, at a frame.
    fn says(text: &str) -> TextPlused {
        TextPlus {
            text: text.into(),
            size: Param::Fixed(0.25),
            ..TextPlus::default()
        }
        .at(0.0)
    }

    /// The whole picture of a Text+, row after row.
    fn picture(settings: &TextPlused) -> Vec<[f32; 4]> {
        let mut whole = vec![[0.0; 4]; (SIZE.0 * SIZE.1) as usize];
        if let Some(lettering) = draw(settings, SIZE) {
            assert_eq!(
                lettering.pixels.len(),
                (lettering.width * lettering.height) as usize
            );
            for row in 0..lettering.height {
                for column in 0..lettering.width {
                    let into = (lettering.at.1 + row) * SIZE.0 + lettering.at.0 + column;
                    let from = row * lettering.width + column;
                    whole[into as usize] = lettering.pixels[from as usize];
                }
            }
        }
        whole
    }

    /// The smallest box of the pixels of a picture that `keep` keeps: x0, y0, x1, y1, the last
    /// beyond.
    fn bounds(picture: &[[f32; 4]], keep: impl Fn([f32; 4]) -> bool) -> [u32; 4] {
        let mut found = [u32::MAX, u32::MAX, 0, 0];
        for (index, pixel) in picture.iter().enumerate() {
            if keep(*pixel) {
                let index = u32::try_from(index).unwrap();
                let (x, y) = (index % SIZE.0, index / SIZE.0);
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

    /// Whether a pixel is covered at all.
    fn inked(pixel: [f32; 4]) -> bool {
        pixel[3] > 0.0
    }

    #[test]
    fn write_on_draws_the_first_characters_where_they_lie_and_none_draws_nothing() {
        let all = says("HHHH");
        let mut half = all.clone();
        half.written = [0, 2];
        let (whole, first) = (picture(&all), picture(&half));
        let (every, some) = (bounds(&whole, inked), bounds(&first, inked));
        // The first two letters, left of the middle, lie where they lie with all four.
        assert_eq!(some[0], every[0]);
        assert!(some[2] <= SIZE.0 / 2 + 1, "{some:?} of {every:?}");
        assert!(every[2] > SIZE.0 / 2 + 20, "{every:?}");
        for (index, pixel) in first.iter().enumerate() {
            if inked(*pixel) {
                assert_eq!(*pixel, whole[index], "{index}");
            }
        }
        // Nothing written, no text and no size draw nothing.
        let mut none = all.clone();
        none.written = [2, 2];
        assert_eq!(draw(&none, SIZE), None);
        none.written = [3, 1];
        assert_eq!(draw(&none, SIZE), None);
        assert_eq!(draw(&says(""), SIZE), None);
        let mut small = all;
        small.size = 0.0;
        assert_eq!(draw(&small, SIZE), None);
    }

    #[test]
    fn the_outline_lies_outside_the_fill_and_under_it() {
        let mut bare = says("I");
        bare.fill = [1.0, 0.0, 0.0, 1.0];
        let mut outlined = bare.clone();
        outlined.outline = Some(Outlined {
            color: [0.0, 0.0, 1.0, 1.0],
            thickness: 0.125,
        });
        let (letter, both) = (picture(&bare), picture(&outlined));
        let mut beside = 0;
        for (index, (alone, with)) in letter.iter().zip(&both).enumerate() {
            if alone[3] == 1.0 {
                // Within the letter, its fill alone: the outline is under it.
                assert_eq!(*with, [1.0, 0.0, 0.0, 1.0], "{index}");
            } else if alone[3] == 0.0 && with[3] > 0.0 {
                // Beyond it, the outline alone.
                assert_eq!([with[0], with[1]], [0.0, 0.0], "{index}");
                beside += 1;
            }
        }
        assert!(beside > 50, "{beside}");
        // It reaches its thickness beyond the letter, three pixels, on every side.
        let (inside, around) = (bounds(&letter, inked), bounds(&both, inked));
        for reach in [
            inside[0] - around[0],
            inside[1] - around[1],
            around[2] - inside[2],
            around[3] - inside[3],
        ] {
            assert!((2..=5).contains(&reach), "{inside:?} in {around:?}");
        }
    }

    #[test]
    fn the_border_is_a_box_behind_the_text_grown_on_every_side() {
        let bare = says("I");
        let mut bordered = bare.clone();
        bordered.border = Some(Bordered {
            color: [0.0, 1.0, 0.0, 1.0],
            extend: 0.125,
            round: 0.0,
        });
        let (letter, both) = (picture(&bare), picture(&bordered));
        // The letter over the box, white where it covers whole; green about it.
        for (index, (alone, with)) in letter.iter().zip(&both).enumerate() {
            if alone[3] == 1.0 {
                assert_eq!(*with, [1.0, 1.0, 1.0, 1.0], "{index}");
            }
        }
        let boxed = bounds(&both, inked);
        let inside = bounds(&letter, inked);
        // Twelve pixels beyond the cell of the letter, which holds its ink: so at least as far
        // beyond its ink, and opaque all over.
        assert!(
            inside[0] - boxed[0] >= 12 && boxed[2] - inside[2] >= 12,
            "{boxed:?}"
        );
        assert!(
            inside[1] - boxed[1] >= 12 && boxed[3] - inside[3] >= 12,
            "{boxed:?}"
        );
        let middle = (boxed[1] + 2) * SIZE.0 + u32::midpoint(boxed[0], boxed[2]);
        assert_eq!(both[middle as usize], [0.0, 1.0, 0.0, 1.0]);
        let (across, down) = (boxed[0] + 1..boxed[2] - 1, boxed[1] + 1..boxed[3] - 1);
        for y in down {
            for x in across.clone() {
                assert_eq!(both[(y * SIZE.0 + x) as usize][3], 1.0, "({x}, {y})");
            }
        }
        // Rounded whole, its corners are clear.
        if let Some(border) = &mut bordered.border {
            border.round = 1.0;
        }
        let round = picture(&bordered);
        assert_eq!(bounds(&round, inked), boxed);
        let corner = boxed[1] * SIZE.0 + boxed[0];
        assert_eq!(round[corner as usize][3], 0.0);
    }

    #[test]
    fn the_follower_fades_each_character_in_after_the_one_before_it() {
        let fade = Param::Keys(vec![
            Key {
                frame: 0,
                value: 0.0,
                interp: Interp::Linear,
            },
            Key {
                frame: 10,
                value: 1.0,
                interp: Interp::Linear,
            },
        ]);
        let text = TextPlus {
            text: "III".into(),
            size: Param::Fixed(0.25),
            follower: Some(TextFollower {
                order: FollowerOrder::LeftToRight,
                delay: 5.0,
                opacity: fade,
                ..TextFollower::default()
            }),
            ..TextPlus::default()
        };
        // At frame 10 the first is in, the second half in and the third not yet; at frame 20
        // all of them are.
        let followed = picture(&text.at(10.0));
        let later = picture(&text.at(20.0));
        // Each letter alone, where it lies.
        let mut alone = TextPlus {
            follower: None,
            ..text
        }
        .at(10.0);
        for (letter, wanted) in [(0, 1.0), (1, 0.5), (2, 0.0)] {
            alone.written = [letter, letter + 1];
            let own = picture(&alone);
            let most = own
                .iter()
                .zip(&followed)
                .filter(|(pixel, _)| pixel[3] == 1.0)
                .map(|(_, seen)| seen[3])
                .fold(0.0_f32, f32::max);
            assert!((most - wanted).abs() < 1e-6, "{letter}: {most}");
        }
        assert_eq!(bounds(&later, inked), bounds(&picture(&says("III")), inked));
    }

    #[test]
    fn the_angle_turns_the_whole_about_its_centre() {
        let flat = says("IIIIIIII");
        let mut upright = flat.clone();
        upright.angle = 90.0;
        let lying = bounds(&picture(&flat), inked);
        let standing = bounds(&picture(&upright), inked);
        let wide = |found: [u32; 4]| (found[2] - found[0], found[3] - found[1]);
        let (across, down) = wide(lying);
        assert!(across > 2 * down, "{lying:?}");
        // Turned a quarter, it is as high as it was wide, and as wide as it was high, about the
        // centre, the middle of the picture.
        let (turned_across, turned_down) = wide(standing);
        assert!(
            turned_down.abs_diff(across) <= 2,
            "{standing:?} of {lying:?}"
        );
        assert!(
            turned_across.abs_diff(down) <= 2,
            "{standing:?} of {lying:?}"
        );
        let centre = (SIZE.0 / 2, SIZE.1 / 2);
        for found in [lying, standing] {
            let (columns, rows) = (found[0]..found[2], found[1]..found[3]);
            assert!(
                columns.contains(&centre.0) && rows.contains(&centre.1),
                "{found:?}"
            );
        }
    }

    #[test]
    fn a_unit_is_moved_up_by_its_offset_and_made_larger_about_its_middle() {
        let plain = says("I");
        let mut moved = plain.clone();
        moved.transform.offset = [0.0, 0.125];
        let (still, up) = (
            bounds(&picture(&plain), inked),
            bounds(&picture(&moved), inked),
        );
        // A share of the height, up: twelve pixels.
        assert_eq!(still[1] - up[1], 12);
        assert_eq!(still[3] - up[3], 12);
        let mut larger = plain;
        larger.transform.scale = 2.0;
        let grown = bounds(&picture(&larger), inked);
        assert!(
            grown[3] - grown[1] > 2 * (still[3] - still[1]) - 3,
            "{grown:?}"
        );
        // About the middle of its cell, which is near that of its ink.
        let middle = |found: [u32; 4]| found[1] + found[3];
        assert!(
            middle(grown).abs_diff(middle(still)) <= 8,
            "{grown:?} of {still:?}"
        );
    }

    #[test]
    fn a_picture_is_known_by_all_that_decides_it() {
        let text = says("Hi");
        assert_eq!(key(&text, SIZE), key(&text.clone(), SIZE));
        assert_ne!(key(&text, SIZE), key(&text, (SIZE.0, SIZE.1 + 1)));
        let mut turned = text.clone();
        turned.angle = 1.0;
        assert_ne!(key(&text, SIZE), key(&turned, SIZE));
        let mut followed = text.clone();
        followed.followed = vec![Followed::NONE; 2];
        assert_ne!(key(&text, SIZE), key(&followed, SIZE));
        let mut later = followed.clone();
        later.followed[1].opacity = 0.5;
        assert_ne!(key(&followed, SIZE), key(&later, SIZE));
        // Half floats, to the nearest.
        assert_eq!(half_bits(1.0), 0x3C00);
        assert_eq!(half_bits(0.5), 0x3800);
        assert_eq!(half_bits(0.0), 0);
        assert_eq!(half_bits(f32::NAN), 0);
        assert_eq!(half_bits(1.0 + 1.0 / 2048.0), 0x3C00);
        assert_eq!(half_bits(1.0 + 3.0 / 2048.0), 0x3C02);
        assert_eq!(half_bits(70_000.0), 0x7C00);
        assert_eq!(half_bits(2.0_f32.powi(-24)), 1);
    }
}
