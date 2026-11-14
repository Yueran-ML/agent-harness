//! Planes of luma and their pyramids.

use crate::TrackError;

/// A plane of luma, `width × height` samples of `f32` row after row, normally from 0 (black) to
/// 1 (white). Only the differences of the samples matter to the tracker, and their scale only
/// through a few thresholds that assume this range.
#[derive(Clone, Debug, PartialEq)]
pub struct Luma {
    width: usize,
    height: usize,
    data: Vec<f32>,
}

/// Check that `len` samples hold `height` rows of `width` with `stride` from one row to the next.
fn check_plane(len: usize, width: usize, height: usize, stride: usize) -> Result<(), TrackError> {
    let needed = if height == 0 {
        0
    } else {
        stride.saturating_mul(height - 1).saturating_add(width)
    };
    if width == 0 || height == 0 || stride < width || len < needed {
        return Err(TrackError::Plane {
            width,
            height,
            stride,
            len,
        });
    }
    Ok(())
}

impl Luma {
    /// A plane of `width × height` from samples packed row after row.
    ///
    /// # Errors
    /// [`TrackError::Plane`] when `data` does not hold exactly that many samples or a side is 0.
    pub fn new(width: usize, height: usize, data: Vec<f32>) -> Result<Self, TrackError> {
        if data.len() != width.saturating_mul(height) {
            return Err(TrackError::Plane {
                width,
                height,
                stride: width,
                len: data.len(),
            });
        }
        check_plane(data.len(), width, height, width)?;
        Ok(Self {
            width,
            height,
            data,
        })
    }

    /// A plane copied from samples of `f32` with `stride` samples from one row to the next.
    ///
    /// # Errors
    /// [`TrackError::Plane`] when `data` is too short for the plane.
    pub fn from_f32(
        data: &[f32],
        width: usize,
        height: usize,
        stride: usize,
    ) -> Result<Self, TrackError> {
        Self::convert(data, width, height, stride, |v| v)
    }

    /// A plane from samples of 16 bits, `max` being white: 65535 for planes whose samples fill
    /// the 16 bits (P010 and P016 put their 10 or 16 bits at the top, so 65535 serves both), 1023
    /// for 10 bits kept at the bottom.
    ///
    /// # Errors
    /// [`TrackError::Plane`] when `data` is too short for the plane.
    pub fn from_u16(
        data: &[u16],
        width: usize,
        height: usize,
        stride: usize,
        max: u16,
    ) -> Result<Self, TrackError> {
        let k = 1.0 / f32::from(max.max(1));
        Self::convert(data, width, height, stride, |v| f32::from(v) * k)
    }

    /// A plane from samples of 8 bits (the luma of NV12), 255 being white.
    ///
    /// # Errors
    /// [`TrackError::Plane`] when `data` is too short for the plane.
    pub fn from_u8(
        data: &[u8],
        width: usize,
        height: usize,
        stride: usize,
    ) -> Result<Self, TrackError> {
        Self::convert(data, width, height, stride, |v| f32::from(v) / 255.0)
    }

    fn convert<T: Copy>(
        data: &[T],
        width: usize,
        height: usize,
        stride: usize,
        f: impl Fn(T) -> f32,
    ) -> Result<Self, TrackError> {
        check_plane(data.len(), width, height, stride)?;
        let mut out = Vec::with_capacity(width * height);
        for row in data.chunks(stride).take(height) {
            out.extend(row[..width].iter().map(|&v| f(v)));
        }
        Ok(Self {
            width,
            height,
            data: out,
        })
    }

    /// Its width.
    #[must_use]
    pub fn width(&self) -> usize {
        self.width
    }

    /// Its height.
    #[must_use]
    pub fn height(&self) -> usize {
        self.height
    }

    /// Its samples, row after row.
    #[must_use]
    pub fn data(&self) -> &[f32] {
        &self.data
    }

    /// The sample at column `x` and row `y`, which must lie in the plane.
    #[must_use]
    pub fn at(&self, x: usize, y: usize) -> f32 {
        self.data[y * self.width + x]
    }

    /// The plane halved by the filter `[1 2 1] / 4` across and down, each sample of the result at
    /// twice its place in this one, so that a point `p` here is at `p / 2` there. The edges are
    /// repeated outward.
    #[must_use]
    pub fn half(&self) -> Self {
        self.half_on(1)
    }

    /// [`Luma::half`] on up to `threads` threads, each making a band of rows (the same result).
    pub(crate) fn half_on(&self, threads: usize) -> Self {
        let (w, h) = (self.width, self.height);
        let (hw, hh) = (w.div_ceil(2), h.div_ceil(2));
        let mut data = vec![0.0_f32; hw * hh];
        let row = |y: usize| &self.data[y * w..][..w];
        let band = |first: usize, out: &mut [f32]| {
            let mut down = vec![0.0_f32; w];
            for (i, dst) in out.chunks_exact_mut(hw).enumerate() {
                let c = 2 * (first + i);
                let (u, m, d) = (row(c.saturating_sub(1)), row(c), row((c + 1).min(h - 1)));
                for (((t, &u), &m), &d) in down.iter_mut().zip(u).zip(m).zip(d) {
                    *t = 0.25 * (u + d) + 0.5 * m;
                }
                // Across: the first and the last by the edge, the rest from windows of three.
                let at = |x: usize| down[x.min(w - 1)];
                dst[0] = 0.25 * (at(0) + at(1)) + 0.5 * at(0);
                let mut done = 1;
                for (o, win) in dst[1..].iter_mut().zip(down[1..].windows(3).step_by(2)) {
                    *o = 0.25 * (win[0] + win[2]) + 0.5 * win[1];
                    done += 1;
                }
                for (x, o) in dst.iter_mut().enumerate().skip(done) {
                    *o = 0.25 * (at(2 * x - 1) + at(2 * x + 1)) + 0.5 * at(2 * x);
                }
            }
        };
        let threads = threads.clamp(1, hh.div_ceil(64).max(1));
        if threads == 1 {
            band(0, &mut data);
        } else {
            let rows = hh.div_ceil(threads);
            std::thread::scope(|scope| {
                for (k, part) in data.chunks_mut(rows * hw).enumerate() {
                    let band = &band;
                    scope.spawn(move || band(k * rows, part));
                }
            });
        }
        Self {
            width: hw,
            height: hh,
            data,
        }
    }

    /// The samples of the square of side `2r + 1` about `[cx, cy]`, each read bilinearly, row
    /// after row into `out`. Within the plane the four weights are the same for every sample, so
    /// that is done once; at the edges the plane is repeated outward.
    pub(crate) fn patch(&self, cx: f32, cy: f32, r: usize, out: &mut [f32]) {
        let side = 2 * r + 1;
        debug_assert_eq!(out.len(), side * side);
        #[allow(clippy::cast_precision_loss, reason = "a small radius")]
        let (left, top) = (cx - r as f32, cy - r as f32);
        let (fx, fy) = (left.floor(), top.floor());
        let (ax, ay) = (left - fx, top - fy);
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_precision_loss,
            reason = "places within a plane of at most 2^24 samples a side"
        )]
        let inside = fx >= 0.0
            && fy >= 0.0
            && fx + (side as f32) < self.width as f32
            && fy + (side as f32) < self.height as f32;
        if inside {
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "checked to lie within the plane"
            )]
            let (x0, y0) = (fx as usize, fy as usize);
            let (w00, w01) = ((1.0 - ax) * (1.0 - ay), ax * (1.0 - ay));
            let (w10, w11) = ((1.0 - ax) * ay, ax * ay);
            for (j, dst) in out.chunks_exact_mut(side).enumerate() {
                let row0 = &self.data[(y0 + j) * self.width + x0..][..=side];
                let row1 = &self.data[(y0 + j + 1) * self.width + x0..][..=side];
                for (i, out) in dst.iter_mut().enumerate() {
                    *out = w00 * row0[i] + w01 * row0[i + 1] + w10 * row1[i] + w11 * row1[i + 1];
                }
            }
        } else {
            for (j, dst) in out.chunks_exact_mut(side).enumerate() {
                for (i, out) in dst.iter_mut().enumerate() {
                    #[allow(clippy::cast_precision_loss, reason = "a small patch")]
                    let (x, y) = (left + i as f32, top + j as f32);
                    *out = self.sample(x, y);
                }
            }
        }
    }

    /// The plane read bilinearly at `[x, y]`, repeated outward beyond its edges.
    #[must_use]
    pub fn sample(&self, x: f32, y: f32) -> f32 {
        #[allow(clippy::cast_precision_loss, reason = "sides of at most 2^24")]
        let (mw, mh) = ((self.width - 1) as f32, (self.height - 1) as f32);
        let (x, y) = (x.clamp(0.0, mw), y.clamp(0.0, mh));
        let (fx, fy) = (x.floor(), y.floor());
        let (ax, ay) = (x - fx, y - fy);
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "clamped into the plane"
        )]
        let (x0, y0) = (fx as usize, fy as usize);
        let (x1, y1) = ((x0 + 1).min(self.width - 1), (y0 + 1).min(self.height - 1));
        let row0 = &self.data[y0 * self.width..];
        let row1 = &self.data[y1 * self.width..];
        (1.0 - ay) * ((1.0 - ax) * row0[x0] + ax * row0[x1])
            + ay * ((1.0 - ax) * row1[x0] + ax * row1[x1])
    }
}

/// A plane and its halvings: level `l` is the plane halved `l` times, a point `p` of the plane
/// being `p / 2^l` there.
#[derive(Clone, Debug)]
pub struct Pyramid {
    levels: Vec<Luma>,
}

impl Pyramid {
    /// The pyramid of `base` with `levels` levels (at least 1, the plane itself), stopping early
    /// when a level would be smaller than 8 samples a side.
    #[must_use]
    pub fn new(base: Luma, levels: usize) -> Self {
        Self::on_threads(base, levels, 1)
    }

    /// [`Pyramid::new`] with each level made on up to `threads` threads.
    #[must_use]
    pub fn on_threads(base: Luma, levels: usize, threads: usize) -> Self {
        let mut out = vec![base];
        while out.len() < levels.max(1) {
            let last = &out[out.len() - 1];
            if last.width < 16 || last.height < 16 {
                break;
            }
            let next = last.half_on(threads);
            out.push(next);
        }
        Self { levels: out }
    }

    /// How many levels a pyramid for a plane of `width × height` gets when the coarsest level
    /// should still be at least `smallest` samples on its shorter side, at most `most` levels.
    #[must_use]
    pub fn levels_for(width: usize, height: usize, smallest: usize, most: usize) -> usize {
        let mut side = width.min(height);
        let mut levels = 1;
        while levels < most && side / 2 >= smallest {
            side /= 2;
            levels += 1;
        }
        levels
    }

    /// Level `l`.
    #[must_use]
    pub fn level(&self, l: usize) -> &Luma {
        &self.levels[l]
    }

    /// How many levels it has.
    #[must_use]
    pub fn len(&self) -> usize {
        self.levels.len()
    }

    /// Never: a pyramid has at least its base.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.levels.is_empty()
    }

    /// The plane itself.
    #[must_use]
    pub fn base(&self) -> &Luma {
        &self.levels[0]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plane_with_a_stride_is_copied_and_a_short_one_refused() {
        let data: Vec<u16> = (0..12).collect();
        let luma = Luma::from_u16(&data, 3, 3, 4, 10).unwrap();
        assert_eq!(luma.data().len(), 9);
        assert!((luma.at(2, 2) - 1.0).abs() < 1e-6);
        assert!((luma.at(0, 1) - 0.4).abs() < 1e-6);
        assert!(matches!(
            Luma::from_u16(&data, 3, 4, 4, 10),
            Err(TrackError::Plane { .. })
        ));
        assert!(Luma::from_u8(&[0; 4], 2, 2, 2).is_ok());
        assert!(Luma::new(2, 2, vec![0.0; 3]).is_err());
    }

    #[test]
    fn a_patch_reads_as_bilinear_samples_inside_and_at_the_edges() {
        let data: Vec<f32> = (0..64 * 48)
            .map(|i| {
                let (x, y) = (i % 64, i / 64);
                #[allow(clippy::cast_precision_loss, reason = "small")]
                let v = (x as f32 * 0.37).sin() + (y as f32 * 0.21).cos();
                v
            })
            .collect();
        let luma = Luma::new(64, 48, data).unwrap();
        let mut out = vec![0.0; 25];
        for (cx, cy) in [(20.3, 17.8), (1.2, 3.4), (62.5, 46.9), (-3.0, 50.0)] {
            luma.patch(cx, cy, 2, &mut out);
            for j in 0..5 {
                for i in 0..5 {
                    #[allow(clippy::cast_precision_loss, reason = "small")]
                    let want = luma.sample(cx - 2.0 + i as f32, cy - 2.0 + j as f32);
                    assert!((out[j * 5 + i] - want).abs() < 1e-5);
                }
            }
        }
    }

    #[test]
    fn a_ramp_halves_to_a_ramp_at_half_the_place() {
        let data: Vec<f32> = (0..40 * 30)
            .map(|i| {
                #[allow(clippy::cast_precision_loss, reason = "small")]
                let v = (i % 40) as f32 + 2.0 * (i / 40) as f32;
                v
            })
            .collect();
        let pyramid = Pyramid::new(Luma::new(40, 30, data).unwrap(), 3);
        assert_eq!(pyramid.len(), 2, "15 rows cannot be halved again");
        let half = pyramid.level(1);
        assert_eq!((half.width(), half.height()), (20, 15));
        // Away from the edges, sample (x, y) of the half is the plane at (2x, 2y).
        assert!((half.at(5, 4) - (10.0 + 2.0 * 8.0)).abs() < 1e-5);
        assert_eq!(Pyramid::levels_for(1920, 1080, 48, 6), 5);
        // Odd sides, on several threads, the same as the plain sum.
        let data: Vec<f32> = (0..301 * 203)
            .map(|i| {
                #[allow(clippy::cast_precision_loss, reason = "small")]
                let v = ((i * 7919) % 1000) as f32 / 1000.0;
                v
            })
            .collect();
        let odd = Luma::new(301, 203, data).unwrap();
        let half = odd.half();
        assert_eq!(half, odd.half_on(5));
        for (x, y) in [(0, 0), (150, 101), (75, 50), (150, 0), (0, 101)] {
            let mut want = 0.0;
            for (dy, wy) in [(-1, 0.25), (0, 0.5), (1, 0.25)] {
                for (dx, wx) in [(-1, 0.25), (0, 0.5), (1, 0.25)] {
                    let sx = (2 * x + dx).clamp(0, 300);
                    let sy = (2 * y + dy).clamp(0, 202);
                    #[allow(clippy::cast_sign_loss, reason = "clamped")]
                    let v = odd.at(sx as usize, sy as usize);
                    want += wx * wy * v;
                }
            }
            #[allow(clippy::cast_sign_loss, reason = "positive")]
            let got = half.at(x as usize, y as usize);
            assert!((got - want).abs() < 1e-6, "{x} {y}");
        }
        assert_eq!(Pyramid::levels_for(256, 192, 48, 6), 3);
    }
}
