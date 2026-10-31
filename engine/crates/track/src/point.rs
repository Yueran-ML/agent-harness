//! The point tracker (pp. 3234, 3245–3249): crosshairs placed by the user, each followed on its
//! own, and the motion of the window fitted to all of them.

use crate::cloud::MIN_EIGEN;
use crate::fit::{least_squares, robust, Model, Robust};
use crate::image::{Luma, Pyramid};
use crate::linalg::{Mat3, Point};
use crate::lk::{par_map, threads, track, Lk};
use crate::{Step, TrackError};

/// The settings of a [`PointTracker`].
#[derive(Clone, Debug, PartialEq)]
pub struct PointOptions {
    /// Whether the motion is fitted with perspective (a homography from 4 points, an affine map
    /// from 3) or as a similarity (from 2 points; one point gives a shift).
    pub perspective: bool,
    /// The half side of the template window about each crosshair: 15 makes it 31×31. A
    /// crosshair has no inner and outer box to set (p. 3247), so this is fixed.
    pub radius: usize,
    /// The levels of the pyramid, or 0 for as many as keep the coarsest at least 48 high.
    pub levels: usize,
    /// The most steps of Lucas–Kanade at each level.
    pub iterations: usize,
    /// A point is lost when tracking it back lands farther than this from where it started.
    pub forward_backward: f64,
    /// A point is lost when its template and its window in the next frame correlate less.
    pub min_ncc: f64,
    /// With 5 points or more, those farther than this from the fitted motion are left out of
    /// the fit (they are still followed).
    pub threshold: f64,
    /// The seed of RANSAC.
    pub seed: u64,
    /// The threads to track points on, or 0 for those of the machine (at most 8).
    pub threads: usize,
}

impl Default for PointOptions {
    fn default() -> Self {
        Self {
            perspective: false,
            radius: 15,
            levels: 0,
            iterations: 20,
            forward_backward: 1.0,
            min_ncc: 0.7,
            threshold: 2.0,
            seed: 0x5EED_9017,
            threads: 0,
        }
    }
}

/// What a [`PointTracker`] found in a frame.
#[derive(Clone, Debug, PartialEq)]
pub struct PointStep {
    /// The motion of the window, as for the cloud.
    pub step: Step,
    /// Where each crosshair is, in the order they were given; `None` once it is lost.
    pub points: Vec<Option<Point>>,
}

/// A tracker of crosshairs from the frame it was made on, one frame at a time.
#[derive(Debug)]
pub struct PointTracker {
    options: PointOptions,
    prev: Pyramid,
    /// Where each was in the first frame.
    start: Vec<Point>,
    /// Where each is now, `None` once lost.
    now: Vec<Option<Point>>,
    motion: Mat3,
    before: Mat3,
}

impl PointTracker {
    /// A tracker of the crosshairs at `points` of `first`.
    ///
    /// # Errors
    /// [`TrackError::NoPoints`] when there are none, or one lies outside the frame.
    pub fn new(first: &Luma, points: &[Point], options: PointOptions) -> Result<Self, TrackError> {
        #[allow(clippy::cast_precision_loss, reason = "sides of at most 2^24")]
        let (w, h) = (first.width() as f64, first.height() as f64);
        let inside = |p: &Point| p[0] >= 0.0 && p[1] >= 0.0 && p[0] <= w - 1.0 && p[1] <= h - 1.0;
        if points.is_empty() || !points.iter().all(inside) {
            return Err(TrackError::NoPoints);
        }
        let levels = if options.levels == 0 {
            Pyramid::levels_for(first.width(), first.height(), 48, 6)
        } else {
            options.levels
        };
        Ok(Self {
            prev: Pyramid::on_threads(first.clone(), levels, threads(options.threads)),
            options,
            start: points.to_vec(),
            now: points.iter().map(|&p| Some(p)).collect(),
            motion: Mat3::IDENTITY,
            before: Mat3::IDENTITY,
        })
    }

    /// Where each crosshair is in the last frame.
    #[must_use]
    pub fn points(&self) -> &[Option<Point>] {
        &self.now
    }

    /// Follow the crosshairs into `next`.
    ///
    /// The step is [`Step::lost`] when every crosshair is lost; the motion then stays.
    ///
    /// # Errors
    /// [`TrackError::FrameSize`] when `next` is not the size of the first frame.
    pub fn step(&mut self, next: &Luma) -> Result<PointStep, TrackError> {
        self.step_owned(next.clone())
    }

    /// [`Self::step`] taking the frame, which then becomes the base of the next pyramid with no
    /// copy.
    ///
    /// # Errors
    /// As [`Self::step`].
    pub fn step_owned(&mut self, next: Luma) -> Result<PointStep, TrackError> {
        let base = self.prev.base();
        if (next.width(), next.height()) != (base.width(), base.height()) {
            return Err(TrackError::FrameSize {
                expected: [base.width(), base.height()],
                got: [next.width(), next.height()],
            });
        }
        let next = Pyramid::on_threads(next, self.prev.len(), threads(self.options.threads));
        let lk = Lk {
            radius: self.options.radius,
            iterations: self.options.iterations,
            epsilon: 0.01,
            min_eigen: MIN_EIGEN,
        };
        let velocity = self
            .before
            .inverse()
            .map_or(Mat3::IDENTITY, |inv| self.motion.mul(&inv));
        let (fb, min_ncc, prev) = (
            self.options.forward_backward,
            self.options.min_ncc,
            &self.prev,
        );
        self.now = par_map(
            &self.now,
            threads(self.options.threads),
            |p, scratch| {
                let p = (*p)?;
                #[allow(clippy::cast_possible_truncation, reason = "a point in a frame")]
                let to32 = |p: Point| [p[0] as f32, p[1] as f32];
                let start = to32(p);
                let ahead = track(prev, &next, start, to32(velocity.apply(p)), &lk, scratch)?;
                if f64::from(ahead.ncc) < min_ncc {
                    return None;
                }
                let back = track(&next, prev, ahead.pos, start, &lk, scratch)?;
                let off =
                    f64::from(back.pos[0] - start[0]).hypot(f64::from(back.pos[1] - start[1]));
                (off <= fb).then(|| [f64::from(ahead.pos[0]), f64::from(ahead.pos[1])])
            },
            self.options.radius,
        );
        self.prev = next;
        let (src, dst): (Vec<Point>, Vec<Point>) = self
            .start
            .iter()
            .zip(&self.now)
            .filter_map(|(&s, &n)| n.map(|n| (s, n)))
            .unzip();
        let n = src.len();
        let fitted = Model::for_points(self.options.perspective, n).and_then(|model| {
            if n >= model.min_points() + 3 {
                let settings = Robust {
                    threshold: self.options.threshold,
                    iterations: 200,
                    seed: self.options.seed,
                };
                robust(model, &src, &dst, settings).map(|fit| (fit.matrix, fit.count, fit.rms))
            } else {
                least_squares(model, &src, &dst, &vec![1.0; n]).map(|h| {
                    let sum: f64 = src
                        .iter()
                        .zip(&dst)
                        .map(|(&p, &q)| crate::fit::residual(&h, p, q).powi(2))
                        .sum();
                    #[allow(clippy::cast_precision_loss, reason = "a small count")]
                    let rms = (sum / n as f64).sqrt();
                    (h, n, rms)
                })
            }
        });
        let Some((matrix, count, rms)) = fitted else {
            self.before = self.motion;
            return Ok(PointStep {
                step: Step {
                    motion: self.motion,
                    confidence: 0.0,
                    features: 0,
                    lost: true,
                },
                points: self.now.clone(),
            });
        };
        self.before = self.motion;
        self.motion = matrix;
        #[allow(clippy::cast_precision_loss, reason = "small counts")]
        let confidence = count as f64 / self.start.len() as f64
            * (-(rms / self.options.threshold).powi(2)).exp();
        #[allow(clippy::cast_possible_truncation, reason = "a share")]
        Ok(PointStep {
            step: Step {
                motion: matrix,
                confidence: confidence as f32,
                features: count,
                lost: false,
            },
            points: self.now.clone(),
        })
    }
}
