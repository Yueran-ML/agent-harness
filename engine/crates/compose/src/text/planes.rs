//! Planes of how much of each pixel something covers, worked on the processor for the dress of
//! a title ([`super::dress`]): the stroke about letters, a Gaussian blur, and a box with rounded
//! corners. Each is a plane of bytes, `columns` × `rows` of them, row after row.

/// How much of each pixel a stroke `width` pixels wide about letters covers, the letters with
/// it: `cover` is how much of each pixel the letters cover, `columns` × `rows` of them, with
/// room about them for the stroke.
///
/// The edge of the letters is taken half a pixel beyond the middle of a pixel that they cover
/// whole, and nearer that middle as they cover less of it; a pixel is covered by the stroke as
/// far as it is within `width` of that edge, smoothed across a pixel. The pixel that they
/// cover nearest to each is found by an exact Euclidean distance transform in two passes
/// (Felzenszwalb and Huttenlocher), down each column and then along each row, each in time
/// linear in the pixels whatever the width: so the stroke is round about every corner, however
/// wide it is, and its edge is within half a pixel of where it would be about the outline.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a width of at most some thousands of pixels; a share of 255"
)]
pub(super) fn stroke(cover: &[u8], columns: usize, rows: usize, width: f32) -> Vec<u8> {
    let mut out = cover.to_vec();
    if columns == 0 || rows == 0 || width.is_nan() || width <= 0.0 {
        return out;
    }
    // No pixel farther than this from the letters is covered.
    let far = u16::try_from((width.ceil() as usize).saturating_add(2)).unwrap_or(u16::MAX);
    let (gaps, nearest) = down_columns(cover, columns, rows, far);
    let mut envelope = Envelope::default();
    for (row, line) in out.chunks_mut(columns).enumerate() {
        let start = row * columns;
        envelope.build(&gaps[start..start + columns], far);
        if envelope.sites.is_empty() {
            continue;
        }
        for (place, value) in line.iter_mut().enumerate() {
            let (site, squared) = envelope.nearest(place);
            let near = f64::from(nearest[start + site]) / 255.0;
            let covered = (f64::from(width) - squared.sqrt() + near).clamp(0.0, 1.0);
            *value = (*value).max((covered * 255.0).round() as u8);
        }
    }
    out
}

/// Down each column of `cover`, how far each pixel is from the nearest that the letters cover
/// at all in its column, up or down (`far` where it is not nearer), and how much they cover of
/// that one: of two as near, the one covered more.
fn down_columns(cover: &[u8], columns: usize, rows: usize, far: u16) -> (Vec<u16>, Vec<u8>) {
    let mut gaps = vec![far; cover.len()];
    let mut nearest = vec![0_u8; cover.len()];
    let mut put = |at: usize, gap: usize, value: u8| {
        let Ok(gap) = u16::try_from(gap) else { return };
        if gap < far && (gap < gaps[at] || (gap == gaps[at] && value > nearest[at])) {
            gaps[at] = gap;
            nearest[at] = value;
        }
    };
    // From above: the last row covered in each column, and how much.
    let mut above: Vec<Option<(usize, u8)>> = vec![None; columns];
    for row in 0..rows {
        for (column, last) in above.iter_mut().enumerate() {
            let at = row * columns + column;
            if cover[at] > 0 {
                *last = Some((row, cover[at]));
            }
            if let Some((from, value)) = *last {
                put(at, row - from, value);
            }
        }
    }
    // From below.
    let mut below: Vec<Option<(usize, u8)>> = vec![None; columns];
    for row in (0..rows).rev() {
        for (column, next) in below.iter_mut().enumerate() {
            let at = row * columns + column;
            if cover[at] > 0 {
                *next = Some((row, cover[at]));
            }
            if let Some((to, value)) = *next {
                put(at, to - row, value);
            }
        }
    }
    (gaps, nearest)
}

/// The lower envelope of the parabolas of the places of a row that letters are near: each the
/// squared distance to the nearest pixel that they cover in its column, plus the squared
/// distance along the row from it (Felzenszwalb and Huttenlocher).
#[derive(Default)]
struct Envelope {
    /// The places whose parabolas are lowest somewhere, in order.
    sites: Vec<usize>,
    /// The height of each, the squared distance down its column.
    heights: Vec<f64>,
    /// Where each begins to be the lowest; the first from far before the row.
    starts: Vec<f64>,
    /// The one that the place asked last is under, as places are asked in order.
    at: usize,
}

impl Envelope {
    /// Build it for a row whose places are `gaps` from the nearest pixel covered in their
    /// columns: those that are `far` or farther have none.
    #[allow(clippy::cast_precision_loss, reason = "places of a row")]
    fn build(&mut self, gaps: &[u16], far: u16) {
        self.sites.clear();
        self.heights.clear();
        self.starts.clear();
        self.at = 0;
        for (place, gap) in gaps.iter().enumerate() {
            if *gap >= far {
                continue;
            }
            let height = f64::from(*gap).powi(2);
            let here = place as f64;
            let mut start = f64::NEG_INFINITY;
            while let (Some(&site), Some(&top)) = (self.sites.last(), self.heights.last()) {
                let there = site as f64;
                start = ((height + here * here) - (top + there * there)) / (2.0 * (here - there));
                let last = self.starts.last().copied().unwrap_or(f64::NEG_INFINITY);
                if start > last {
                    break;
                }
                // The last is lowest nowhere.
                self.sites.pop();
                self.heights.pop();
                self.starts.pop();
                start = f64::NEG_INFINITY;
            }
            self.sites.push(place);
            self.heights.push(height);
            self.starts.push(start);
        }
    }

    /// The place of the row that is nearest `place`, through its column, and the squared
    /// distance to it; asked of places in order.
    #[allow(clippy::cast_precision_loss, reason = "places of a row")]
    fn nearest(&mut self, place: usize) -> (usize, f64) {
        let here = place as f64;
        while self.at + 1 < self.starts.len() && self.starts[self.at + 1] < here {
            self.at += 1;
        }
        let site = self.sites[self.at];
        let along = here - site as f64;
        (site, along * along + self.heights[self.at])
    }
}

/// How a blur of a standard deviation is done.
enum Blur {
    /// None: no blur.
    None,
    /// By the weights of a Gaussian, about the middle one, for a deviation below 2 pixels.
    Kernel(Vec<f32>),
    /// By three boxes, each a size and how many of it are before the place it is of, as SVG
    /// blurs (`feGaussianBlur`), which is within some hundredths of a Gaussian from 2 pixels on.
    Boxes([(usize, usize); 3]),
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "a deviation of at most some thousands of pixels"
)]
fn blur_of(sigma: f32) -> Blur {
    if sigma.is_nan() || sigma <= 0.0 {
        return Blur::None;
    }
    if sigma < 2.0 {
        let radius = (3.0 * sigma).ceil() as usize;
        let mut weights: Vec<f32> = (0..=2 * radius)
            .map(|place| {
                let from = place as f32 - radius as f32;
                (-from * from / (2.0 * sigma * sigma)).exp()
            })
            .collect();
        let total: f32 = weights.iter().sum();
        for weight in &mut weights {
            *weight /= total;
        }
        return Blur::Kernel(weights);
    }
    let size = (sigma * 3.0 * (2.0 * std::f32::consts::PI).sqrt() / 4.0 + 0.5).floor() as usize;
    let half = size / 2;
    if size % 2 == 1 {
        Blur::Boxes([(size, half); 3])
    } else {
        Blur::Boxes([(size, half), (size, half - 1), (size + 1, half)])
    }
}

/// How far a blur of standard deviation `sigma` ([`gaussian`]) moves what it blurs, at most,
/// in pixels.
pub(super) fn blur_reach(sigma: f32) -> usize {
    match blur_of(sigma) {
        Blur::None => 0,
        Blur::Kernel(weights) => weights.len() / 2,
        Blur::Boxes(boxes) => boxes
            .iter()
            .map(|&(size, lead)| lead.max(size - 1 - lead))
            .sum(),
    }
}

/// Blur `plane`, `columns` × `rows`, by a Gaussian of standard deviation `sigma` pixels, along
/// each row and then down each column, what is beyond it taken as nothing: by its weights below
/// a deviation of 2 pixels, and by three boxes, in time linear in the pixels whatever the
/// deviation, from 2 on ([`Blur`]).
pub(super) fn gaussian(plane: &mut [u8], columns: usize, rows: usize, sigma: f32) {
    let blur = blur_of(sigma);
    if matches!(blur, Blur::None) || columns == 0 || rows == 0 {
        return;
    }
    let (mut line, mut spare) = (Vec::new(), Vec::new());
    for row in plane.chunks_mut(columns) {
        line.clear();
        line.extend_from_slice(row);
        blur_line(&blur, &mut line, &mut spare);
        row.copy_from_slice(&line);
    }
    for column in 0..columns {
        line.clear();
        line.extend((0..rows).map(|row| plane[row * columns + column]));
        blur_line(&blur, &mut line, &mut spare);
        for (row, value) in line.iter().enumerate() {
            plane[row * columns + column] = *value;
        }
    }
}

/// Blur a line as `blur` says, with `spare` to work in.
fn blur_line(blur: &Blur, line: &mut Vec<u8>, spare: &mut Vec<u8>) {
    spare.clear();
    spare.resize(line.len(), 0);
    match blur {
        Blur::None => {}
        Blur::Kernel(weights) => {
            convolve(line, spare, weights);
            std::mem::swap(line, spare);
        }
        Blur::Boxes(boxes) => {
            for &(size, lead) in boxes {
                box_line(line, spare, size, lead);
                std::mem::swap(line, spare);
            }
        }
    }
}

/// Each place of `line` weighed with those about it by `weights`, the middle one its own.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a weighed mean of bytes"
)]
fn convolve(line: &[u8], out: &mut [u8], weights: &[f32]) {
    let radius = weights.len() / 2;
    for (place, value) in out.iter_mut().enumerate() {
        let mut sum = 0.0;
        for (offset, weight) in weights.iter().enumerate() {
            let source = (place + offset)
                .checked_sub(radius)
                .and_then(|at| line.get(at));
            if let Some(source) = source {
                sum += weight * f32::from(*source);
            }
        }
        *value = sum.round().clamp(0.0, 255.0) as u8;
    }
}

/// The mean of `size` places of `line` about each, `lead` of them before it, rounded: by a sum
/// that moves along the line.
#[allow(clippy::cast_possible_truncation, reason = "a mean of bytes")]
fn box_line(line: &[u8], out: &mut [u8], size: usize, lead: usize) {
    let value = |at: usize| line.get(at).map_or(0, |value| u32::from(*value));
    let count = u32::try_from(size.max(1)).unwrap_or(u32::MAX);
    let mut sum: u32 = (0..size - lead).map(value).sum();
    for (place, mean) in out.iter_mut().enumerate() {
        *mean = ((sum + count / 2) / count) as u8;
        sum += value(place + size - lead);
        if place >= lead {
            sum -= value(place - lead);
        }
    }
}

/// How much of the pixel whose middle is at `at` a box covers: x0, y0, x1, y1, its corners
/// rounded by `radius` (at most half its width and half its height), smoothed across a pixel at
/// its edge.
pub(super) fn rounded_box(rect: [f64; 4], radius: f64, at: (f64, f64)) -> f64 {
    let [x0, y0, x1, y1] = rect;
    let (half_width, half_height) = ((x1 - x0) / 2.0, (y1 - y0) / 2.0);
    if half_width <= 0.0 || half_height <= 0.0 {
        return 0.0;
    }
    let radius = radius.min(half_width).min(half_height).max(0.0);
    let across = (at.0 - f64::midpoint(x0, x1)).abs() - (half_width - radius);
    let down = (at.1 - f64::midpoint(y0, y1)).abs() - (half_height - radius);
    let outside = across.max(0.0).hypot(down.max(0.0));
    let inside = across.max(down).min(0.0);
    (0.5 - (outside + inside - radius)).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stroke_about_one_pixel_is_a_round_disc_of_its_width() {
        let (columns, rows) = (15, 15);
        let mut cover = vec![0; columns * rows];
        cover[7 * columns + 7] = 255;
        let out = stroke(&cover, columns, rows, 3.0);
        let at = |x: usize, y: usize| out[y * columns + x];
        // Within 3 of the edge, half a pixel beyond the middle: 3.5 of the middle, round.
        for (x, y) in [(7, 7), (10, 7), (7, 4), (9, 9), (4, 7)] {
            assert_eq!(at(x, y), 255, "({x}, {y})");
        }
        // 4 away straight across, and 3 across and 3 down (4.24), are beyond it.
        for (x, y) in [(11, 7), (7, 11), (10, 10), (4, 4), (3, 7)] {
            assert_eq!(at(x, y), 0, "({x}, {y})");
        }
        // 3 across and 2 down (3.61) is covered in part.
        assert!((90..=115).contains(&at(10, 9)), "{}", at(10, 9));
        assert_eq!(at(10, 9), at(4, 5));
    }

    #[test]
    fn a_stroke_reaches_its_width_beyond_a_straight_edge_and_is_smoothed_at_its_own() {
        let (columns, rows) = (24, 5);
        // Covered whole left of 10, half at 10.
        let mut cover = vec![0; columns * rows];
        for line in cover.chunks_mut(columns) {
            line[..10].fill(255);
            line[10] = 128;
        }
        let out = stroke(&cover, columns, rows, 2.0);
        let line = &out[2 * columns..3 * columns];
        // The edge is halfway across pixel 10: a stroke of 2 reaches halfway across pixel 12.
        assert_eq!(line[..12], [255; 12]);
        assert!((115..=140).contains(&line[12]), "{line:?}");
        assert_eq!(line[13..], [0; 11]);
        // A stroke of no width is the letters as they are.
        assert_eq!(stroke(&cover, columns, rows, 0.0), cover);
    }

    #[test]
    fn a_stroke_covers_what_is_nearest_to_any_of_many_letters() {
        let (columns, rows) = (40, 9);
        let mut cover = vec![0; columns * rows];
        for x in [5, 20, 33] {
            cover[4 * columns + x] = 255;
        }
        let out = stroke(&cover, columns, rows, 2.0);
        let line = &out[4 * columns..5 * columns];
        for x in [3, 5, 7, 18, 22, 31, 35] {
            assert_eq!(line[x], 255, "{x}");
        }
        for x in [0, 9, 12, 16, 25, 29, 38] {
            assert_eq!(line[x], 0, "{x}");
        }
    }

    /// The share of a whole at which the blur of a step at 200 across a line, at the middle row
    /// of a plane, is at each place: far enough from the edges of the plane that what is
    /// beyond them, nothing, is not blurred in.
    fn blurred_step(sigma: f32) -> Vec<f64> {
        let (columns, rows) = (400, 201);
        let mut plane = vec![0; columns * rows];
        for line in plane.chunks_mut(columns) {
            line[..200].fill(255);
        }
        gaussian(&mut plane, columns, rows, sigma);
        plane[100 * columns..101 * columns]
            .iter()
            .map(|value| f64::from(*value) / 255.0)
            .collect()
    }

    #[test]
    fn a_blur_spreads_a_step_as_a_gaussian_does() {
        // The share of a Gaussian below each place, for a step at 200.
        let below = |place: f64, sigma: f64| {
            let z = (200.0 - (place + 0.5)) / sigma;
            // Within a thousandth (Abramowitz and Stegun 7.1.26).
            let t = 1.0 / (1.0 + 0.327_591_1 * z.abs() / std::f64::consts::SQRT_2);
            let poly = t
                * (0.254_829_592
                    + t * (-0.284_496_736
                        + t * (1.421_413_741 + t * (-1.453_152_027 + t * 1.061_405_429))));
            let erf = 1.0 - poly * (-(z * z) / 2.0).exp();
            f64::midpoint(1.0, erf.copysign(z))
        };
        for sigma in [0.8_f32, 1.5, 3.0, 6.0, 12.5] {
            let line = blurred_step(sigma);
            for (place, share) in line.iter().enumerate().take(300).skip(100) {
                #[allow(clippy::cast_precision_loss, reason = "a place of a line")]
                let wanted = below(place as f64, f64::from(sigma));
                assert!(
                    (share - wanted).abs() < 0.04,
                    "sigma {sigma} at {place}: {share} for {wanted}"
                );
            }
            // Nothing moves farther than the reach of the blur.
            let reach = blur_reach(sigma);
            assert!(line[100..200 - reach].iter().all(|share| *share > 0.999));
            assert!(line[200 + reach..300].iter().all(|share| *share < 0.001));
        }
    }

    #[test]
    fn a_blur_keeps_what_it_spreads_and_none_is_no_blur() {
        let (columns, rows) = (61, 61);
        let mut plane = vec![0_u8; columns * rows];
        for y in 26..35 {
            plane[y * columns + 26..y * columns + 35].fill(255);
        }
        let before: u32 = plane.iter().map(|value| u32::from(*value)).sum();
        let mut kept = plane.clone();
        gaussian(&mut kept, columns, rows, 0.0);
        assert_eq!(kept, plane);
        gaussian(&mut plane, columns, rows, 3.0);
        let after: u32 = plane.iter().map(|value| u32::from(*value)).sum();
        assert!(after.abs_diff(before) * 50 < before, "{before} to {after}");
        assert!(plane[30 * columns + 30] < 255);
        assert!(plane[30 * columns + 22] > 0);
        // As much either way, but for rounding.
        let (left, right) = (plane[30 * columns + 24], plane[30 * columns + 36]);
        assert!(left.abs_diff(right) <= 1, "{left} {right}");
        let (above, below) = (plane[24 * columns + 30], plane[36 * columns + 30]);
        assert!(above.abs_diff(below) <= 1, "{above} {below}");
        assert_eq!(blur_reach(0.0), 0);
        assert_eq!(blur_reach(1.0), 3);
    }

    #[test]
    fn a_box_with_rounded_corners_covers_its_middle_and_not_its_corners() {
        let rect = [10.0, 10.0, 50.0, 30.0];
        let at = |radius: f64, x: f64, y: f64| rounded_box(rect, radius, (x + 0.5, y + 0.5));
        // Square: every pixel of it whole, none beyond, its edges sharp.
        for (x, y) in [(10.0, 10.0), (49.0, 29.0), (30.0, 20.0)] {
            assert!((at(0.0, x, y) - 1.0).abs() < 1e-9, "({x}, {y})");
        }
        for (x, y) in [(9.0, 10.0), (50.0, 20.0), (30.0, 30.0)] {
            assert!(at(0.0, x, y).abs() < 1e-9, "({x}, {y})");
        }
        // Rounded by 8: the corner pixel is beyond it, the middles of its sides are not.
        assert!(at(8.0, 10.0, 10.0).abs() < 1e-9);
        assert!((at(8.0, 30.0, 10.0) - 1.0).abs() < 1e-9);
        assert!((at(8.0, 10.0, 20.0) - 1.0).abs() < 1e-9);
        // A radius larger than half its height is half its height.
        assert!((at(100.0, 30.0, 20.0) - 1.0).abs() < 1e-9);
        assert!(at(100.0, 10.0, 10.0).abs() < 1e-9);
    }
}
