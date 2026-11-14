//! The little linear algebra that the tracker needs: 3×3 matrices of the plane in homogeneous
//! coordinates, and the solution of small square systems.

use serde::{Deserialize, Serialize};

/// A point of the frame, `[x, y]` in pixels, the centre of the pixel at column `i` and row `j`
/// being `[i, j]`.
pub type Point = [f64; 2];

/// A projective transform of the plane, as a 3×3 matrix acting on `[x, y, 1]` (row major).
///
/// Every transform of the crate (a translation, a similarity, an affine map, a homography) is
/// one of these, so they compose by [`Mat3::mul`] whatever their kind.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mat3(pub [[f64; 3]; 3]);

impl Default for Mat3 {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Mat3 {
    /// The transform that leaves every point where it is.
    pub const IDENTITY: Self = Self([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);

    /// A shift by `[x, y]`.
    #[must_use]
    pub fn translation(x: f64, y: f64) -> Self {
        Self([[1.0, 0.0, x], [0.0, 1.0, y], [0.0, 0.0, 1.0]])
    }

    /// `self · other`: `other` first, then `self`.
    #[must_use]
    pub fn mul(&self, other: &Self) -> Self {
        let (a, b) = (&self.0, &other.0);
        let mut out = [[0.0; 3]; 3];
        for (i, row) in out.iter_mut().enumerate() {
            for (j, cell) in row.iter_mut().enumerate() {
                *cell = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j];
            }
        }
        Self(out)
    }

    /// Where `p` goes. A point sent to infinity (its `w` zero) comes back as non-finite numbers.
    #[must_use]
    pub fn apply(&self, p: Point) -> Point {
        let m = &self.0;
        let w = m[2][0] * p[0] + m[2][1] * p[1] + m[2][2];
        [
            (m[0][0] * p[0] + m[0][1] * p[1] + m[0][2]) / w,
            (m[1][0] * p[0] + m[1][1] * p[1] + m[1][2]) / w,
        ]
    }

    /// The determinant.
    #[must_use]
    pub fn det(&self) -> f64 {
        let m = &self.0;
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    }

    /// The inverse, scaled so that its corner is 1 when it can be; `None` when the matrix is
    /// singular.
    #[must_use]
    pub fn inverse(&self) -> Option<Self> {
        let m = &self.0;
        let det = self.det();
        if !det.is_finite() || det.abs() < 1e-300 {
            return None;
        }
        let cof = |r0: usize, r1: usize, c0: usize, c1: usize| {
            m[r0][c0] * m[r1][c1] - m[r0][c1] * m[r1][c0]
        };
        let inv = Self([
            [cof(1, 2, 1, 2), -cof(0, 2, 1, 2), cof(0, 1, 1, 2)],
            [-cof(1, 2, 0, 2), cof(0, 2, 0, 2), -cof(0, 1, 0, 2)],
            [cof(1, 2, 0, 1), -cof(0, 2, 0, 1), cof(0, 1, 0, 1)],
        ]);
        Some(inv.scaled(1.0 / det).normalized())
    }

    /// Every entry times `k` (the same transform, for `k` not zero).
    #[must_use]
    pub fn scaled(&self, k: f64) -> Self {
        let mut out = self.0;
        for row in &mut out {
            for cell in row {
                *cell *= k;
            }
        }
        Self(out)
    }

    /// The same transform with its corner made 1, or as it is when the corner is (nearly) 0.
    #[must_use]
    pub fn normalized(&self) -> Self {
        let w = self.0[2][2];
        if w.abs() > 1e-12 {
            self.scaled(1.0 / w)
        } else {
            *self
        }
    }

    /// The largest distance between where `self` and `other` send the points of `points`.
    #[must_use]
    pub fn distance_on(&self, other: &Self, points: &[Point]) -> f64 {
        points
            .iter()
            .map(|&p| {
                let (a, b) = (self.apply(p), other.apply(p));
                (a[0] - b[0]).hypot(a[1] - b[1])
            })
            .fold(0.0, f64::max)
    }
}

/// The solution of `a · x = b` by elimination with partial pivoting, or `None` when `a` is
/// singular (to the precision of its largest entry).
pub(crate) fn solve<const N: usize>(mut a: [[f64; N]; N], mut b: [f64; N]) -> Option<[f64; N]> {
    let scale = a
        .iter()
        .flat_map(|row| row.iter())
        .fold(0.0_f64, |m, v| m.max(v.abs()));
    if !(scale.is_finite() && scale > 0.0) {
        return None;
    }
    let tiny = scale * 1e-13;
    for col in 0..N {
        let pivot = (col..N).max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))?;
        if a[pivot][col].abs() <= tiny {
            return None;
        }
        a.swap(col, pivot);
        b.swap(col, pivot);
        for row in col + 1..N {
            let f = a[row][col] / a[col][col];
            if f != 0.0 {
                let pivot_row = a[col];
                for (cell, p) in a[row][col..].iter_mut().zip(&pivot_row[col..]) {
                    *cell -= f * p;
                }
                b[row] -= f * b[col];
            }
        }
    }
    let mut x = [0.0; N];
    for row in (0..N).rev() {
        let mut s = b[row];
        for k in row + 1..N {
            s -= a[row][k] * x[k];
        }
        x[row] = s / a[row][row];
    }
    x.iter().all(|v| v.is_finite()).then_some(x)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_inverse_undoes_its_matrix() {
        let m = Mat3([[1.1, 0.2, 5.0], [-0.1, 0.9, -3.0], [1e-4, -2e-4, 1.0]]);
        let back = m.mul(&m.inverse().unwrap()).normalized();
        for (i, row) in back.0.iter().enumerate() {
            for (j, v) in row.iter().enumerate() {
                let want = if i == j { 1.0 } else { 0.0 };
                assert!((v - want).abs() < 1e-12, "{back:?}");
            }
        }
        assert!(Mat3([[1.0, 2.0, 0.0], [2.0, 4.0, 0.0], [0.0, 0.0, 1.0]])
            .inverse()
            .is_none());
    }

    #[test]
    fn a_small_system_is_solved_and_a_singular_one_refused() {
        let x = solve(
            [[2.0, 1.0, 0.0], [1.0, 3.0, 1.0], [0.0, 1.0, 4.0]],
            [3.0, 5.0, 5.0],
        )
        .unwrap();
        for v in x {
            assert!((v - 1.0).abs() < 1e-12);
        }
        assert!(solve([[1.0, 2.0], [2.0, 4.0]], [1.0, 2.0]).is_none());
    }
}
