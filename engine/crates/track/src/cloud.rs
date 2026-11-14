//! The cloud tracker (p. 3234): a cloud of features found within the window, each followed from
//! frame to frame, and the motion of the window fitted to those that move together.

use crate::features::{detect, Detect};
use crate::fit::{residual, robust, Model, Robust};
use crate::image::{Luma, Pyramid};
use crate::linalg::{Mat3, Point};
use crate::lk::{par_map, threads, track, Lk};
use crate::region::Region;
use crate::{Step, TrackError};

/// The settings of a [`CloudTracker`]. The defaults suit frames from SD to UHD.
#[derive(Clone, Debug, PartialEq)]
pub struct CloudOptions {
    /// Whether the motion is fitted with perspective (a homography, from 4 features) or as a
    /// similarity. The other boxes of the palette only mask what is applied
    /// ([`crate::Motion::masked`]).
    pub perspective: bool,
    /// The most features in the cloud (100–300 is usual).
    pub max_features: usize,
    /// When fewer than this share of `max_features` are left, more are found within the window
    /// where it now is. Off in Interactive Mode, where the cloud is the user's ([`Self::replenish`]).
    pub replenish_below: f64,
    /// Whether lost features are replaced.
    pub replenish: bool,
    /// The least distance between features, in pixels.
    pub min_distance: f64,
    /// A feature's corner strength must be at least this share of the strongest in the window.
    pub quality: f64,
    /// The half side of the window of Lucas–Kanade: 10 makes it 21×21.
    pub radius: usize,
    /// The levels of the pyramid, or 0 for as many as keep the coarsest level at least 48
    /// samples high (5 for 1080p), which follows motions of up to about `radius · 2^(levels−1)`
    /// pixels from one frame to the next.
    pub levels: usize,
    /// The most steps of Lucas–Kanade at each level.
    pub iterations: usize,
    /// A feature is dropped when tracking it back to the frame before lands farther than this
    /// from where it started, in pixels.
    pub forward_backward: f64,
    /// A feature is dropped when the correlation of its windows in the two frames is below this.
    pub min_ncc: f64,
    /// The distance in pixels within which a feature agrees with the fitted motion.
    pub threshold: f64,
    /// The most hypotheses of RANSAC per frame.
    pub ransac_iterations: usize,
    /// The seed of RANSAC.
    pub seed: u64,
    /// The threads to track features on, or 0 for those of the machine (at most 8).
    pub threads: usize,
}

impl Default for CloudOptions {
    fn default() -> Self {
        Self {
            perspective: false,
            max_features: 120,
            replenish_below: 0.7,
            replenish: true,
            min_distance: 8.0,
            quality: 0.01,
            radius: 10,
            levels: 0,
            iterations: 10,
            forward_backward: 1.0,
            min_ncc: 0.8,
            threshold: 1.5,
            ransac_iterations: 200,
            seed: 0x5EED_C10D,
            threads: 0,
        }
    }
}

/// The least smallest eigenvalue of a window, per sample, for luma from 0 to 1: a window of
/// noise of 0.01 stays below it.
pub(crate) const MIN_EIGEN: f32 = 1e-5;

/// A feature of the cloud.
#[derive(Clone, Copy, Debug)]
struct Feature {
    /// Where it is now.
    pos: Point,
    /// Where it was in the first frame, by the motion when it was found.
    reference: Point,
    /// The frames it has been followed through since it was found.
    age: u32,
}

/// A cloud tracker that follows a window from the frame it was made on, one frame at a time.
///
/// Its [`Step::motion`] is the motion from the first frame, fitted to where the features now
/// are against where they were then (features found later are taken back to the first frame by
/// the motion of their frame), so that errors do not pile up from frame to frame.
#[derive(Debug)]
pub struct CloudTracker {
    options: CloudOptions,
    region: Region,
    prev: Pyramid,
    features: Vec<Feature>,
    motion: Mat3,
    /// The motion of the frame before, for the guess of the next.
    before: Mat3,
}

impl CloudTracker {
    /// A tracker of `region` (where the window is in `first`), with its cloud found.
    ///
    /// # Errors
    /// [`TrackError::EmptyRegion`] when the region holds no pixel of the frame, and
    /// [`TrackError::NoFeatures`] when it holds nothing that can be tracked (a flat area).
    pub fn new(first: &Luma, region: Region, options: CloudOptions) -> Result<Self, TrackError> {
        if region.raster(first.width(), first.height()).is_empty() {
            return Err(TrackError::EmptyRegion);
        }
        let levels = if options.levels == 0 {
            Pyramid::levels_for(first.width(), first.height(), 48, 6)
        } else {
            options.levels
        };
        let mut out = Self {
            prev: Pyramid::on_threads(first.clone(), levels, threads(options.threads)),
            options,
            region,
            features: Vec::new(),
            motion: Mat3::IDENTITY,
            before: Mat3::IDENTITY,
        };
        out.replenish(None);
        if out.features.is_empty() {
            return Err(TrackError::NoFeatures);
        }
        Ok(out)
    }

    /// The settings.
    #[must_use]
    pub fn options(&self) -> &CloudOptions {
        &self.options
    }

    /// Where the features of the cloud are in the last frame, to be drawn.
    #[must_use]
    pub fn features(&self) -> Vec<Point> {
        self.features.iter().map(|f| f.pos).collect()
    }

    /// The motion from the first frame to the last.
    #[must_use]
    pub fn motion(&self) -> Mat3 {
        self.motion
    }

    /// Interactive Mode's Insert (p. 3237): features found within the box from `a` to `b` of the
    /// last frame, added to the cloud. Returns how many; none when nothing there can be tracked
    /// (p. 3242).
    pub fn insert_in(&mut self, a: Point, b: Point) -> usize {
        let rect = Region::rect(
            a[0].min(b[0]),
            a[1].min(b[1]),
            a[0].max(b[0]),
            a[1].max(b[1]),
        );
        let before = self.features.len();
        self.replenish(Some(&rect));
        self.features.len() - before
    }

    /// Interactive Mode's Delete (p. 3237): the features within the box from `a` to `b` taken
    /// out of the cloud. Returns how many.
    pub fn delete_in(&mut self, a: Point, b: Point) -> usize {
        let (x0, x1) = (a[0].min(b[0]), a[0].max(b[0]));
        let (y0, y1) = (a[1].min(b[1]), a[1].max(b[1]));
        let before = self.features.len();
        self.features
            .retain(|f| !(f.pos[0] >= x0 && f.pos[0] <= x1 && f.pos[1] >= y0 && f.pos[1] <= y1));
        before - self.features.len()
    }

    /// Find features within the window where it now is (and within `only` when given), away
    /// from those there are, up to the most.
    fn replenish(&mut self, only: Option<&Region>) {
        let base = self.prev.base();
        let here = self.region.transformed(&self.motion);
        let mut raster = here.raster(base.width(), base.height());
        if let Some(only) = only {
            let box_ = only.raster(base.width(), base.height());
            for y in 0..raster.height {
                for x in 0..raster.width {
                    let k = y * raster.width + x;
                    raster.inside[k] &= box_.contains(raster.x0 + x, raster.y0 + y);
                }
            }
        }
        let room = self
            .options
            .max_features
            .saturating_sub(self.features.len());
        if room == 0 {
            return;
        }
        #[allow(
            clippy::cast_possible_truncation,
            reason = "a point in a frame, a share"
        )]
        let existing: Vec<[f32; 2]> = self
            .features
            .iter()
            .map(|f| [f.pos[0] as f32, f.pos[1] as f32])
            .collect();
        #[allow(clippy::cast_possible_truncation, reason = "settings of a few units")]
        let settings = Detect {
            max: room,
            quality: self.options.quality as f32,
            min_eigen: MIN_EIGEN,
            min_distance: self.options.min_distance as f32,
            border: self.options.radius + 2,
        };
        let Some(back) = self.motion.inverse() else {
            return;
        };
        for p in detect(base, &raster, &existing, &settings) {
            let pos = [f64::from(p[0]), f64::from(p[1])];
            self.features.push(Feature {
                pos,
                reference: back.apply(pos),
                age: 0,
            });
        }
    }

    /// Follow the window into `next`, the frame after (or before, when tracking in reverse).
    ///
    /// When too few features agree on a motion, the step is [`Step::lost`]: the motion stays
    /// where it was, and the cloud is found again within the window there, in `next`.
    ///
    /// # Errors
    /// [`TrackError::FrameSize`] when `next` is not the size of the first frame.
    pub fn step(&mut self, next: &Luma) -> Result<Step, TrackError> {
        self.step_owned(next.clone())
    }

    /// [`Self::step`] taking the frame, which then becomes the base of the next pyramid with no
    /// copy.
    ///
    /// # Errors
    /// As [`Self::step`].
    #[allow(clippy::too_many_lines, reason = "one frame, step by step")]
    pub fn step_owned(&mut self, next: Luma) -> Result<Step, TrackError> {
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
        // Guess by the motion of the frame before.
        let velocity = self
            .before
            .inverse()
            .map_or(Mat3::IDENTITY, |inv| self.motion.mul(&inv));
        let fb = self.options.forward_backward;
        let min_ncc = self.options.min_ncc;
        let prev = &self.prev;
        let moved: Vec<Option<Point>> = par_map(
            &self.features,
            threads(self.options.threads),
            |f, scratch| {
                #[allow(clippy::cast_possible_truncation, reason = "a point in a frame")]
                let to32 = |p: Point| [p[0] as f32, p[1] as f32];
                let start = to32(f.pos);
                let guess = to32(velocity.apply(f.pos));
                let ahead = track(prev, &next, start, guess, &lk, scratch)?;
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
        // Those that could vote, for the share that agrees.
        let (total, seasoned) = (
            self.features.len(),
            self.features.iter().filter(|f| f.age > 0).count(),
        );
        let mut kept: Vec<Feature> = self
            .features
            .iter()
            .zip(&moved)
            .filter_map(|(f, m)| {
                m.map(|pos| Feature {
                    pos,
                    reference: f.reference,
                    age: f.age,
                })
            })
            .collect();
        self.prev = next;
        // The motion is fitted to the features followed before this frame; one just found has
        // its place by the motion of the frame it was found in, so it agrees with that motion
        // whatever it is on (an occluder that stands still, say) and is let in only once it has
        // agreed with a motion it took no part in, more closely than the threshold.
        let threshold = self.options.threshold;
        let old = kept.iter().filter(|f| f.age > 0).count();
        let enough = |n: usize| {
            Model::for_points(self.options.perspective, n).is_some_and(|m| n >= m.min_points() + 2)
        };
        let voters: Vec<&Feature> = kept.iter().filter(|f| f.age > 0 || !enough(old)).collect();
        let fit = Model::for_points(self.options.perspective, voters.len()).and_then(|model| {
            let (src, dst): (Vec<Point>, Vec<Point>) =
                voters.iter().map(|f| (f.reference, f.pos)).unzip();
            let settings = Robust {
                threshold,
                iterations: self.options.ransac_iterations,
                seed: self.options.seed,
            };
            robust(model, &src, &dst, settings)
                .filter(|fit| fit.count >= model.min_points() + 2)
                .map(|fit| (model, fit))
        });
        let Some((model, fit)) = fit else {
            // Lost: hold the motion and look again where the window is (unless the cloud is the
            // user's, in Interactive Mode).
            self.features.clear();
            self.before = self.motion;
            if self.options.replenish {
                self.replenish(None);
            }
            return Ok(Step {
                motion: self.motion,
                confidence: 0.0,
                features: 0,
                lost: true,
            });
        };
        let all_vote = !enough(old);
        kept.retain(|f| {
            let r = residual(&fit.matrix, f.reference, f.pos);
            if f.age > 0 || all_vote {
                r < threshold
            } else {
                r < 0.5 * threshold
            }
        });
        for f in &mut kept {
            f.age = f.age.saturating_add(1);
        }
        self.features = kept;
        self.before = self.motion;
        self.motion = fit.matrix;
        if self.options.replenish {
            #[allow(
                clippy::cast_precision_loss,
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "a share of a small count"
            )]
            let low = (self.options.replenish_below * self.options.max_features as f64) as usize;
            if self.features.len() < low {
                self.replenish(None);
            }
        }
        #[allow(clippy::cast_precision_loss, reason = "small counts")]
        let confidence = {
            let voted = if all_vote { total } else { seasoned };
            let agree = fit.count as f64 / voted.max(1) as f64;
            let enough = (fit.count as f64 / (4 * model.min_points()).max(12) as f64).min(1.0);
            let tight = (-(fit.rms / self.options.threshold).powi(2)).exp();
            agree * enough * tight
        };
        #[allow(clippy::cast_possible_truncation, reason = "a share")]
        Ok(Step {
            motion: self.motion,
            confidence: confidence as f32,
            features: fit.count,
            lost: false,
        })
    }
}
