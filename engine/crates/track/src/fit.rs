//! Fitting a transform to pairs of points: closed forms for each kind, RANSAC against points
//! that move otherwise, and reweighted least squares (Huber) for the final fit.

use crate::linalg::{solve, Mat3, Point};

/// The kind of transform that is fitted, from the fewest numbers to the most.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Model {
    /// A shift: 2 numbers, 1 point.
    Translation,
    /// A shift and a rotation: 3 numbers, 2 points.
    Rigid,
    /// A shift, a uniform scale and a rotation: 4 numbers, 2 points (but see
    /// [`Model::for_points`]: it is fitted from 3).
    Similarity,
    /// Any linear map and a shift: 6 numbers, 3 points.
    Affine,
    /// Any homography: 8 numbers, 4 points.
    Homography,
}

impl Model {
    /// The fewest pairs that fix it.
    #[must_use]
    pub fn min_points(self) -> usize {
        match self {
            Self::Translation => 1,
            Self::Rigid | Self::Similarity => 2,
            Self::Affine => 3,
            Self::Homography => 4,
        }
    }

    /// The kind fitted to `n` points, as the manual allows: "at least two tracking points
    /// [...] to track rotation, and at least three to track zoom" (p. 3237). So one point gives a
    /// shift, two a shift and a rotation (the zoom stays 1), and three or more a similarity; with
    /// Perspective 3D on, three give an affine map and four or more a homography.
    ///
    /// Zoom and rotation are fitted even when their boxes are off, and left out only from what
    /// is applied to the window ([`crate::Motion::masked`]): a fit that ignored a rotation of the
    /// subject would put its features far from the anchor out of place.
    #[must_use]
    pub fn for_points(perspective: bool, n: usize) -> Option<Self> {
        match n {
            0 => None,
            1 => Some(Self::Translation),
            2 => Some(Self::Rigid),
            3 if perspective => Some(Self::Affine),
            _ if perspective => Some(Self::Homography),
            _ => Some(Self::Similarity),
        }
    }
}

/// A generator of numbers that are the same on every run: `SplitMix64`.
#[derive(Clone, Debug)]
pub(crate) struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A number below `n` (which is not 0).
    pub fn below(&mut self, n: usize) -> usize {
        #[allow(clippy::cast_possible_truncation, reason = "below n, which is a usize")]
        let v = ((u128::from(self.next_u64()) * n as u128) >> 64) as usize;
        v
    }
}

/// Where `h` sends `src[i]`, less `dst[i]`, as a distance.
pub(crate) fn residual(h: &Mat3, src: Point, dst: Point) -> f64 {
    let p = h.apply(src);
    let d = (p[0] - dst[0]).hypot(p[1] - dst[1]);
    if d.is_finite() {
        d
    } else {
        f64::INFINITY
    }
}

/// The least-squares fit of `model` to the pairs `src[i] → dst[i]` weighed by `w[i]`, or `None`
/// when they do not fix it (too few, all in a line, all weights 0).
pub(crate) fn least_squares(model: Model, src: &[Point], dst: &[Point], w: &[f64]) -> Option<Mat3> {
    let total: f64 = w.iter().sum();
    if src.len() < model.min_points() || total <= 0.0 {
        return None;
    }
    let mean = |ps: &[Point]| {
        let s = ps
            .iter()
            .zip(w)
            .fold([0.0, 0.0], |s, (p, &w)| [s[0] + w * p[0], s[1] + w * p[1]]);
        [s[0] / total, s[1] / total]
    };
    let (ms, md) = (mean(src), mean(dst));
    match model {
        Model::Translation => Some(Mat3::translation(md[0] - ms[0], md[1] - ms[1])),
        Model::Rigid | Model::Similarity => {
            // x' = a·x − b·y + tx, y' = b·x + a·y + ty, about the means; for a rigid motion,
            // (a, b) made a unit vector (the rotation of Procrustes).
            let (mut num_a, mut num_b, mut den) = (0.0, 0.0, 0.0);
            for ((p, q), &w) in src.iter().zip(dst).zip(w) {
                let (x, y) = (p[0] - ms[0], p[1] - ms[1]);
                let (u, v) = (q[0] - md[0], q[1] - md[1]);
                num_a += w * (x * u + y * v);
                num_b += w * (x * v - y * u);
                den += w * (x * x + y * y);
            }
            if den <= 1e-12 * total {
                return None;
            }
            let (a, b) = if model == Model::Rigid {
                let norm = num_a.hypot(num_b);
                if norm <= 1e-12 * den {
                    return None;
                }
                (num_a / norm, num_b / norm)
            } else {
                (num_a / den, num_b / den)
            };
            Some(Mat3([
                [a, -b, md[0] - (a * ms[0] - b * ms[1])],
                [b, a, md[1] - (b * ms[0] + a * ms[1])],
                [0.0, 0.0, 1.0],
            ]))
        }
        Model::Affine => {
            // Each row of the map from the 3×3 normal equations, about the means.
            let mut ata = [[0.0; 3]; 3];
            let (mut atu, mut atv) = ([0.0; 3], [0.0; 3]);
            for ((p, q), &w) in src.iter().zip(dst).zip(w) {
                let r = [p[0] - ms[0], p[1] - ms[1], 1.0];
                for i in 0..3 {
                    for j in 0..3 {
                        ata[i][j] += w * r[i] * r[j];
                    }
                    atu[i] += w * r[i] * (q[0] - md[0]);
                    atv[i] += w * r[i] * (q[1] - md[1]);
                }
            }
            let (a, b) = (solve(ata, atu)?, solve(ata, atv)?);
            let local = Mat3([
                [a[0], a[1], a[2] + md[0]],
                [b[0], b[1], b[2] + md[1]],
                [0.0, 0.0, 1.0],
            ]);
            Some(local.mul(&Mat3::translation(-ms[0], -ms[1])))
        }
        Model::Homography => homography(src, dst, w),
    }
}

/// The similarity that moves the weighted points to the centre and gives them a mean distance
/// of √2 from it (Hartley's normalisation).
fn normaliser(ps: &[Point], w: &[f64]) -> Option<Mat3> {
    let total: f64 = w.iter().sum();
    let m = ps
        .iter()
        .zip(w)
        .fold([0.0, 0.0], |s, (p, &w)| [s[0] + w * p[0], s[1] + w * p[1]]);
    let m = [m[0] / total, m[1] / total];
    let spread: f64 = ps
        .iter()
        .zip(w)
        .map(|(p, &w)| w * (p[0] - m[0]).hypot(p[1] - m[1]))
        .sum::<f64>()
        / total;
    if spread <= 1e-9 {
        return None;
    }
    let k = std::f64::consts::SQRT_2 / spread;
    Some(Mat3([
        [k, 0.0, -k * m[0]],
        [0.0, k, -k * m[1]],
        [0.0, 0.0, 1.0],
    ]))
}

/// A homography by the normalised direct linear transform (with `h₃₃ = 1`), then a few steps of
/// Gauss–Newton on the distances in the normalised frame.
fn homography(src: &[Point], dst: &[Point], w: &[f64]) -> Option<Mat3> {
    let (ns, nd) = (normaliser(src, w)?, normaliser(dst, w)?);
    let s: Vec<Point> = src.iter().map(|&p| ns.apply(p)).collect();
    let d: Vec<Point> = dst.iter().map(|&p| nd.apply(p)).collect();
    // h = [h11 h12 h13 h21 h22 h23 h31 h32].
    let mut ata = [[0.0; 8]; 8];
    let mut atb = [0.0; 8];
    for ((p, q), &w) in s.iter().zip(&d).zip(w) {
        let (x, y, u, v) = (p[0], p[1], q[0], q[1]);
        let rows = [
            ([x, y, 1.0, 0.0, 0.0, 0.0, -x * u, -y * u], u),
            ([0.0, 0.0, 0.0, x, y, 1.0, -x * v, -y * v], v),
        ];
        for (r, b) in rows {
            for i in 0..8 {
                for j in 0..8 {
                    ata[i][j] += w * r[i] * r[j];
                }
                atb[i] += w * r[i] * b;
            }
        }
    }
    let mut h = solve(ata, atb)?;
    for _ in 0..4 {
        let mut jtj = [[0.0; 8]; 8];
        let mut jtr = [0.0; 8];
        for ((p, q), &w) in s.iter().zip(&d).zip(w) {
            let (x, y) = (p[0], p[1]);
            let den = h[6] * x + h[7] * y + 1.0;
            if den.abs() < 1e-9 {
                return None;
            }
            let u = (h[0] * x + h[1] * y + h[2]) / den;
            let v = (h[3] * x + h[4] * y + h[5]) / den;
            let ju = [
                x / den,
                y / den,
                1.0 / den,
                0.0,
                0.0,
                0.0,
                -u * x / den,
                -u * y / den,
            ];
            let jv = [
                0.0,
                0.0,
                0.0,
                x / den,
                y / den,
                1.0 / den,
                -v * x / den,
                -v * y / den,
            ];
            for (j, r) in [(ju, q[0] - u), (jv, q[1] - v)] {
                for a in 0..8 {
                    for b in 0..8 {
                        jtj[a][b] += w * j[a] * j[b];
                    }
                    jtr[a] += w * j[a] * r;
                }
            }
        }
        let Some(step) = solve(jtj, jtr) else { break };
        for (h, s) in h.iter_mut().zip(step) {
            *h += s;
        }
        if step.iter().map(|s| s * s).sum::<f64>() < 1e-20 {
            break;
        }
    }
    let hn = Mat3([[h[0], h[1], h[2]], [h[3], h[4], h[5]], [h[6], h[7], 1.0]]);
    let out = nd.inverse()?.mul(&hn).mul(&ns).normalized();
    out.0.iter().flatten().all(|v| v.is_finite()).then_some(out)
}

/// The settings of [`robust`].
#[derive(Clone, Copy, Debug)]
pub(crate) struct Robust {
    /// The distance in pixels within which a pair agrees with a transform.
    pub threshold: f64,
    /// The most hypotheses that RANSAC tries.
    pub iterations: usize,
    /// The seed of its draws.
    pub seed: u64,
}

/// What [`robust`] found.
#[derive(Clone, Debug)]
pub(crate) struct Fit {
    pub matrix: Mat3,
    /// How many pairs agree with it.
    pub count: usize,
    /// The root mean square of the distances of the inliers.
    pub rms: f64,
}

/// The least distance at which Huber's weights start to fall, in pixels: about the precision
/// of Lucas–Kanade on a clean frame.
const HUBER_FLOOR: f64 = 0.05;

/// Whether the points of a draw are too close together, or too near a line, to fix `model`.
fn degenerate(model: Model, ps: &[Point]) -> bool {
    let close = |a: Point, b: Point| (a[0] - b[0]).hypot(a[1] - b[1]) < 1.0;
    let area = |a: Point, b: Point, c: Point| {
        ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])).abs()
    };
    match model {
        Model::Translation => false,
        Model::Rigid | Model::Similarity => close(ps[0], ps[1]),
        Model::Affine => area(ps[0], ps[1], ps[2]) < 4.0,
        Model::Homography => (0..4).any(|skip| {
            let t: Vec<Point> = (0..4).filter(|&i| i != skip).map(|i| ps[i]).collect();
            area(t[0], t[1], t[2]) < 4.0
        }),
    }
}

/// The transform of kind `model` that most pairs agree with: RANSAC over draws of the fewest
/// pairs (scored by MSAC, the sum of the squared distances capped at the threshold, with a fixed
/// seed so that the same pairs give the same fit), then least squares over the pairs that agree,
/// reweighted five times by Huber's weights, and the pairs that agree with that.
#[allow(clippy::too_many_lines, reason = "the draws, then the reweighted fits")]
pub(crate) fn robust(model: Model, src: &[Point], dst: &[Point], settings: Robust) -> Option<Fit> {
    let n = src.len();
    let m = model.min_points();
    if n < m {
        return None;
    }
    let t = settings.threshold;
    let score = |h: &Mat3| -> f64 {
        src.iter()
            .zip(dst)
            .map(|(&p, &q)| residual(h, p, q).min(t).powi(2))
            .sum()
    };
    let mut best: Option<(f64, Mat3)> = None;
    if n == m {
        best = least_squares(model, src, dst, &vec![1.0; n]).map(|h| (score(&h), h));
    } else {
        let mut rng = Rng::new(settings.seed);
        let mut needed = settings.iterations;
        let mut picks = [0_usize; 4];
        let mut tries = 0;
        while tries < needed {
            tries += 1;
            for i in 0..m {
                picks[i] = loop {
                    let k = rng.below(n);
                    if !picks[..i].contains(&k) {
                        break k;
                    }
                };
            }
            let (s, d): (Vec<Point>, Vec<Point>) =
                picks[..m].iter().map(|&i| (src[i], dst[i])).unzip();
            if degenerate(model, &s) {
                continue;
            }
            let Some(h) = least_squares(model, &s, &d, &[1.0; 4][..m]) else {
                continue;
            };
            let sc = score(&h);
            if best.as_ref().is_none_or(|(b, _)| sc < *b) {
                best = Some((sc, h));
                // Stop when a better draw is unlikely (99.9 %) given the share that agrees.
                let agree = src
                    .iter()
                    .zip(dst)
                    .filter(|(&p, &q)| residual(&h, p, q) < t)
                    .count();
                #[allow(clippy::cast_precision_loss, reason = "small counts")]
                let share = agree as f64 / n as f64;
                #[allow(
                    clippy::cast_possible_wrap,
                    clippy::cast_possible_truncation,
                    reason = "at most 4"
                )]
                let all_good = share.powi(m as i32);
                if all_good >= 1.0 - 1e-12 {
                    needed = tries;
                } else if all_good > 0.0 {
                    let k = (1e-3_f64).ln() / (1.0 - all_good).ln();
                    #[allow(
                        clippy::cast_possible_truncation,
                        clippy::cast_sign_loss,
                        reason = "a positive count, capped"
                    )]
                    let k = k.ceil().min(1e6) as usize;
                    needed = needed.min(k.max(tries));
                }
            }
        }
    }
    let (_, mut h) = best?;
    let inliers_of = |h: &Mat3| -> Vec<bool> {
        src.iter()
            .zip(dst)
            .map(|(&p, &q)| residual(h, p, q) < t)
            .collect()
    };
    let mut inliers = inliers_of(&h);
    for _ in 0..5 {
        // Huber's weights at 1.345 times the spread of the inliers (from their median
        // distance), so that pairs that agree only loosely, such as features on the edge of an
        // occluder, weigh less the tighter the others agree.
        let mut spread: Vec<f64> = src
            .iter()
            .zip(dst)
            .zip(&inliers)
            .filter(|(_, &inside)| inside)
            .map(|((&p, &q), _)| residual(&h, p, q))
            .collect();
        if spread.is_empty() {
            break;
        }
        let mid = spread.len() / 2;
        let median = *spread.select_nth_unstable_by(mid, f64::total_cmp).1;
        let huber = (1.345 * 1.4826 * median).clamp(HUBER_FLOOR, 0.5 * t);
        let w: Vec<f64> = src
            .iter()
            .zip(dst)
            .zip(&inliers)
            .map(|((&p, &q), &inside)| {
                if !inside {
                    return 0.0;
                }
                let r = residual(&h, p, q);
                if r <= huber {
                    1.0
                } else {
                    huber / r
                }
            })
            .collect();
        match least_squares(model, src, dst, &w) {
            Some(next) => h = next,
            None => break,
        }
        inliers = inliers_of(&h);
    }
    let count = inliers.iter().filter(|&&b| b).count();
    if count < m {
        return None;
    }
    let sum: f64 = src
        .iter()
        .zip(dst)
        .zip(&inliers)
        .filter(|(_, &inside)| inside)
        .map(|((&p, &q), _)| residual(&h, p, q).powi(2))
        .sum();
    #[allow(clippy::cast_precision_loss, reason = "a small count")]
    let rms = (sum / count as f64).sqrt();
    Some(Fit {
        matrix: h,
        count,
        rms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid() -> Vec<Point> {
        let mut out = Vec::new();
        for j in 0..7 {
            for i in 0..9 {
                out.push([
                    100.0 + 37.0 * f64::from(i) + f64::from(j * i % 5),
                    80.0 + 29.0 * f64::from(j),
                ]);
            }
        }
        out
    }

    #[test]
    fn each_kind_is_found_exactly_from_its_pairs() {
        let h = Mat3([[1.02, -0.05, 4.0], [0.04, 0.99, -3.0], [2e-5, -1e-5, 1.0]]);
        let sim = Mat3([[0.98, -0.1, 7.0], [0.1, 0.98, 2.0], [0.0, 0.0, 1.0]]);
        let (sin, cos) = 0.1_f64.sin_cos();
        let rigid = Mat3([[cos, -sin, 7.0], [sin, cos, 2.0], [0.0, 0.0, 1.0]]);
        let aff = Mat3([[1.1, 0.2, -5.0], [-0.1, 0.9, 6.0], [0.0, 0.0, 1.0]]);
        let src = grid();
        for (model, truth) in [
            (Model::Translation, Mat3::translation(3.5, -2.25)),
            (Model::Rigid, rigid),
            (Model::Similarity, sim),
            (Model::Affine, aff),
            (Model::Homography, h),
        ] {
            let dst: Vec<Point> = src.iter().map(|&p| truth.apply(p)).collect();
            let fit = least_squares(model, &src, &dst, &vec![1.0; src.len()]).unwrap();
            assert!(fit.distance_on(&truth, &src) < 1e-8, "{model:?}");
            let m = model.min_points();
            let picks = [&src[0..1], &src[62..63], &src[8..9], &src[54..55]].concat();
            let to: Vec<Point> = picks.iter().map(|&p| truth.apply(p)).collect();
            let fit = least_squares(model, &picks[..m], &to[..m], &[1.0; 4][..m]).unwrap();
            assert!(fit.distance_on(&truth, &src) < 1e-6, "{model:?} from {m}");
        }
    }

    #[test]
    fn ransac_leaves_out_the_pairs_that_move_otherwise() {
        let truth = Mat3([[1.01, -0.03, 2.0], [0.03, 1.01, 1.0], [1e-5, 2e-5, 1.0]]);
        let src = grid();
        let mut rng = Rng::new(7);
        let dst: Vec<Point> = src
            .iter()
            .enumerate()
            .map(|(i, &p)| {
                let q = truth.apply(p);
                if i % 3 == 0 {
                    #[allow(clippy::cast_precision_loss, reason = "small")]
                    let off = (rng.below(40) as f64) - 20.0;
                    [q[0] + off + 5.0, q[1] - 7.0]
                } else {
                    q
                }
            })
            .collect();
        let settings = Robust {
            threshold: 1.0,
            iterations: 500,
            seed: 1,
        };
        let fit = robust(Model::Homography, &src, &dst, settings).unwrap();
        assert!(fit.matrix.distance_on(&truth, &src) < 1e-6);
        assert_eq!(fit.count, src.len() - src.len().div_ceil(3));
        for (i, (&p, &q)) in src.iter().zip(&dst).enumerate() {
            assert_eq!(residual(&fit.matrix, p, q) < 1.0, i % 3 != 0);
        }
        let again = robust(Model::Homography, &src, &dst, settings).unwrap();
        assert_eq!(again.matrix, fit.matrix);
    }

    #[test]
    fn a_rigid_fit_keeps_the_scale_of_a_zoom_out() {
        let src = grid();
        let sim = Mat3([[0.98, -0.1, 7.0], [0.1, 0.98, 2.0], [0.0, 0.0, 1.0]]);
        let dst: Vec<Point> = src.iter().map(|&p| sim.apply(p)).collect();
        let fit = least_squares(Model::Rigid, &src, &dst, &vec![1.0; src.len()]).unwrap();
        let m = fit.0;
        assert!((m[0][0].hypot(m[1][0]) - 1.0).abs() < 1e-12);
        assert!((m[1][0].atan2(m[0][0]) - 0.1_f64.atan2(0.98)).abs() < 1e-12);
    }

    #[test]
    fn the_kind_follows_the_points_and_perspective() {
        assert_eq!(Model::for_points(false, 0), None);
        assert_eq!(Model::for_points(true, 1), Some(Model::Translation));
        // Two points track rotation, three zoom (p. 3237).
        assert_eq!(Model::for_points(true, 2), Some(Model::Rigid));
        assert_eq!(Model::for_points(false, 2), Some(Model::Rigid));
        assert_eq!(Model::for_points(true, 3), Some(Model::Affine));
        assert_eq!(Model::for_points(false, 3), Some(Model::Similarity));
        assert_eq!(Model::for_points(true, 9), Some(Model::Homography));
        assert_eq!(Model::for_points(false, 9), Some(Model::Similarity));
    }
}
