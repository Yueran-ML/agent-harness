//! The window that is tracked, as the region of the frame it covers.

use serde::{Deserialize, Serialize};

use crate::linalg::{Mat3, Point};

/// The part of a frame that a window covers, in the coordinates of the frame ([`Point`]).
///
/// The server makes it from the shape of the window as it stands at the frame where tracking
/// starts: the outline of a polygon or a curve (a circle sampled along its edge), taken out to
/// the outer edge of its softness when the features at the soft edge should count.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Region {
    /// A closed polygon; the inside by the even–odd rule.
    Polygon(Vec<Point>),
    /// A mask over the whole frame, `width × height` bytes row after row, inside where not 0.
    Mask {
        /// Its width.
        width: usize,
        /// Its height.
        height: usize,
        /// Its bytes.
        data: Vec<u8>,
    },
}

/// The pixels of a region, within the box that bounds it.
#[derive(Clone, Debug)]
pub(crate) struct Raster {
    pub x0: usize,
    pub y0: usize,
    pub width: usize,
    pub height: usize,
    pub inside: Vec<bool>,
}

impl Raster {
    /// Whether the pixel at column `x` and row `y` of the frame is inside.
    pub fn contains(&self, x: usize, y: usize) -> bool {
        x >= self.x0
            && y >= self.y0
            && x < self.x0 + self.width
            && y < self.y0 + self.height
            && self.inside[(y - self.y0) * self.width + (x - self.x0)]
    }

    /// Whether no pixel is inside.
    pub fn is_empty(&self) -> bool {
        !self.inside.iter().any(|&b| b)
    }
}

impl Region {
    /// The rectangle from `[x0, y0]` to `[x1, y1]`.
    #[must_use]
    pub fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Self {
        Self::Polygon(vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]])
    }

    /// Whether `p` is inside.
    #[must_use]
    pub fn contains(&self, p: Point) -> bool {
        match self {
            Self::Polygon(points) => inside_polygon(points, p),
            Self::Mask {
                width,
                height,
                data,
            } => {
                let (x, y) = (p[0].round(), p[1].round());
                #[allow(clippy::cast_precision_loss, reason = "sides of at most 2^24")]
                let fits = x >= 0.0 && y >= 0.0 && x < *width as f64 && y < *height as f64;
                #[allow(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "checked to lie within the mask"
                )]
                let at = || data[y as usize * width + x as usize] != 0;
                fits && at()
            }
        }
    }

    /// The centre of the region: of the area of a polygon (of its corners when it has none), of
    /// the pixels of a mask. `None` when the region is empty.
    #[must_use]
    pub fn centroid(&self) -> Option<Point> {
        match self {
            Self::Polygon(points) => {
                if points.is_empty() {
                    return None;
                }
                let (mut area, mut cx, mut cy) = (0.0, 0.0, 0.0);
                for (i, a) in points.iter().enumerate() {
                    let b = points[(i + 1) % points.len()];
                    let cross = a[0] * b[1] - b[0] * a[1];
                    area += cross;
                    cx += (a[0] + b[0]) * cross;
                    cy += (a[1] + b[1]) * cross;
                }
                if area.abs() > 1e-9 {
                    Some([cx / (3.0 * area), cy / (3.0 * area)])
                } else {
                    #[allow(clippy::cast_precision_loss, reason = "a small count")]
                    let n = points.len() as f64;
                    let sum = points
                        .iter()
                        .fold([0.0, 0.0], |s, p| [s[0] + p[0], s[1] + p[1]]);
                    Some([sum[0] / n, sum[1] / n])
                }
            }
            Self::Mask { width, data, .. } => {
                let (mut n, mut sx, mut sy) = (0.0, 0.0, 0.0);
                for (i, &v) in data.iter().enumerate() {
                    if v != 0 {
                        #[allow(clippy::cast_precision_loss, reason = "a count of pixels")]
                        let (x, y) = ((i % width) as f64, (i / width) as f64);
                        n += 1.0;
                        sx += x;
                        sy += y;
                    }
                }
                (n > 0.0).then(|| [sx / n, sy / n])
            }
        }
    }

    /// The region carried by `h`: the corners of a polygon moved (exact, since a homography
    /// keeps straight lines straight), a mask read back through the inverse.
    #[must_use]
    pub fn transformed(&self, h: &Mat3) -> Self {
        match self {
            Self::Polygon(points) => Self::Polygon(points.iter().map(|&p| h.apply(p)).collect()),
            Self::Mask {
                width,
                height,
                data,
            } => {
                let Some(inv) = h.inverse() else {
                    return self.clone();
                };
                let mut out = vec![0; data.len()];
                for (i, v) in out.iter_mut().enumerate() {
                    #[allow(clippy::cast_precision_loss, reason = "a count of pixels")]
                    let p = inv.apply([(i % width) as f64, (i / width) as f64]);
                    if self.contains(p) {
                        *v = 1;
                    }
                }
                Self::Mask {
                    width: *width,
                    height: *height,
                    data: out,
                }
            }
        }
    }

    /// The pixels inside, within a frame of `width × height`.
    pub(crate) fn raster(&self, width: usize, height: usize) -> Raster {
        let empty = Raster {
            x0: 0,
            y0: 0,
            width: 0,
            height: 0,
            inside: Vec::new(),
        };
        match self {
            Self::Polygon(points) => {
                if points.len() < 3
                    || points
                        .iter()
                        .any(|p| !(p[0].is_finite() && p[1].is_finite()))
                {
                    return empty;
                }
                let lo = points
                    .iter()
                    .fold([f64::MAX; 2], |m, p| [m[0].min(p[0]), m[1].min(p[1])]);
                let hi = points
                    .iter()
                    .fold([f64::MIN; 2], |m, p| [m[0].max(p[0]), m[1].max(p[1])]);
                #[allow(clippy::cast_precision_loss, reason = "sides of at most 2^24")]
                let (fw, fh) = (width as f64, height as f64);
                let clamp = |v: f64, max: f64| v.clamp(0.0, max);
                #[allow(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "clamped into the frame"
                )]
                let (x0, y0, x1, y1) = (
                    clamp(lo[0].floor(), fw) as usize,
                    clamp(lo[1].floor(), fh) as usize,
                    clamp(hi[0].ceil() + 1.0, fw) as usize,
                    clamp(hi[1].ceil() + 1.0, fh) as usize,
                );
                if x1 <= x0 || y1 <= y0 {
                    return empty;
                }
                let (w, h) = (x1 - x0, y1 - y0);
                let mut inside = vec![false; w * h];
                let mut crossings = Vec::new();
                for row in 0..h {
                    #[allow(clippy::cast_precision_loss, reason = "a place in a frame")]
                    let y = (y0 + row) as f64;
                    crossings.clear();
                    for (i, a) in points.iter().enumerate() {
                        let b = points[(i + 1) % points.len()];
                        if (a[1] <= y) != (b[1] <= y) {
                            crossings.push(a[0] + (y - a[1]) * (b[0] - a[0]) / (b[1] - a[1]));
                        }
                    }
                    crossings.sort_by(f64::total_cmp);
                    for pair in crossings.as_chunks::<2>().0 {
                        #[allow(clippy::cast_precision_loss, reason = "a place in a frame")]
                        let (from, to) = (pair[0] - x0 as f64, pair[1] - x0 as f64);
                        #[allow(
                            clippy::cast_possible_truncation,
                            clippy::cast_sign_loss,
                            clippy::cast_precision_loss,
                            reason = "clamped into the box"
                        )]
                        let (a, b) = (
                            from.ceil().clamp(0.0, w as f64) as usize,
                            (to.ceil()).clamp(0.0, w as f64) as usize,
                        );
                        for v in &mut inside[row * w + a..row * w + b.max(a)] {
                            *v = true;
                        }
                    }
                }
                Raster {
                    x0,
                    y0,
                    width: w,
                    height: h,
                    inside,
                }
            }
            Self::Mask {
                width: mw,
                height: mh,
                data,
            } => {
                let (w, h) = ((*mw).min(width), (*mh).min(height));
                let mut inside = vec![false; w * h];
                for y in 0..h {
                    for x in 0..w {
                        inside[y * w + x] = data[y * mw + x] != 0;
                    }
                }
                Raster {
                    x0: 0,
                    y0: 0,
                    width: w,
                    height: h,
                    inside,
                }
            }
        }
    }
}

fn inside_polygon(points: &[Point], p: Point) -> bool {
    let mut inside = false;
    for (i, a) in points.iter().enumerate() {
        let b = points[(i + 1) % points.len()];
        if (a[1] <= p[1]) != (b[1] <= p[1]) {
            let x = a[0] + (p[1] - a[1]) * (b[0] - a[0]) / (b[1] - a[1]);
            if p[0] < x {
                inside = !inside;
            }
        }
    }
    inside
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rectangle_is_rastered_to_the_pixels_whose_centres_it_holds() {
        let region = Region::rect(2.5, 3.5, 6.5, 5.5);
        let raster = region.raster(20, 20);
        let mut count = 0;
        for y in 0..20 {
            for x in 0..20 {
                let inside = raster.contains(x, y);
                #[allow(clippy::cast_precision_loss, reason = "small")]
                let want = region.contains([x as f64, y as f64]);
                assert_eq!(inside, want, "{x} {y}");
                count += usize::from(inside);
            }
        }
        assert_eq!(count, 4 * 2);
        assert_eq!(region.centroid(), Some([4.5, 4.5]));
    }

    #[test]
    fn a_mask_is_carried_through_a_shift() {
        let mut data = vec![0; 100];
        data[3 * 10 + 4] = 1;
        let mask = Region::Mask {
            width: 10,
            height: 10,
            data,
        };
        let moved = mask.transformed(&Mat3::translation(2.0, 1.0));
        assert!(moved.contains([6.0, 4.0]));
        assert!(!moved.contains([4.0, 3.0]));
        assert_eq!(moved.centroid(), Some([6.0, 4.0]));
        assert!(moved.raster(10, 10).contains(6, 4));
        assert!(Region::Polygon(vec![]).raster(10, 10).is_empty());
    }
}
