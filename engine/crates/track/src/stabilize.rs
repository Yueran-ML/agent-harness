//! The mathematics of the stabilizer of the Edit, Cut and Color pages (Reference Manual of
//! Resolve 20, pp. 633–634, 1064–1065 and 3435–3437): from how the picture moves between the
//! frames of a clip, where to put each frame so that the camera seems to have moved smoothly, or
//! not at all.
//!
//! # What it is given, and what it gives
//!
//! The tracker fits how the picture moves from each frame to the next ([`crate::Model`]). Put
//! one after another, those steps are the path of the camera ([`path`]): for each frame, the
//! motion of the picture since the first, about the centre of the frame. The stabilizer smooths
//! that path ([`smoothed`]) and gives, for each frame, the transform that puts the frame where a
//! camera on the smooth path would have seen it ([`stabilized`]). Finding the steps in the
//! frames of a clip, and drawing the frames through the transforms, are not done here.
//!
//! # The controls
//!
//! - **Method** ([`Method`]): Perspective, Similarity or Translation, which is how much of the
//!   motion is taken to be the camera's: all of a homography, a shift with a turn and a zoom, or
//!   a shift alone. What is left out is not corrected.
//! - **Camera Lock**: the camera is taken to have stood still where it was at the first frame.
//!   Smooth and Cropping Ratio then have no say, as the manual has it.
//! - **Smooth**, from 0 to 1: how much the path is smoothed. At 0 the smooth path is the path,
//!   and nothing is corrected. The manual gives no measure for it: here each number of the path
//!   is averaged over its neighbours with the weights of a Gaussian, whose width at 1 is
//!   [`SMOOTH_FRAMES`] frames.
//! - **Strength**, from −1 to 1: how much of the correction is made. 1 makes it whole, 0 none,
//!   and −1 its opposite, which gives a still picture the shake of the clip (a match move).
//! - **Cropping Ratio**, from 0 to 1: the share of the frame, about its centre, that the
//!   corrected picture must always cover. A frame whose correction would leave less is
//!   corrected only as far as it may be. At 1 nothing is corrected, as the manual has it;
//!   lower values let the stabilizer do more.
//! - **Zoom**: the picture is enlarged about its centre, by one factor for the whole clip, as
//!   far as it takes for no frame to show an edge. The factor is never more than one over the
//!   Cropping Ratio.
//!
//! # Coordinates
//!
//! As the tracker's: pixels of the frame, `[x, y]` with y down. The motion of a frame is about
//! the centre of the frame.

use crate::linalg::{Mat3, Point};
use crate::motion::{Component, Components, Motion};

/// The width of the Gaussian that smooths the path where Smooth is 1, in frames: its standard
/// deviation. Ours: the manual gives no measure.
pub const SMOOTH_FRAMES: f64 = 40.0;

/// How many times a share is halved to find how far a correction may go.
const HALVINGS: u32 = 40;

/// How much of the motion of the picture is taken to be the camera's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Method {
    /// All of a homography: shift, turn, zoom and perspective.
    Perspective,
    /// Shift, turn and zoom.
    Similarity,
    /// Shift alone.
    Translation,
}

impl Method {
    /// The components of a motion that the method corrects.
    #[must_use]
    pub fn components(self) -> Components {
        match self {
            Method::Perspective => Components::ALL,
            Method::Similarity => Components {
                perspective: false,
                ..Components::ALL
            },
            Method::Translation => Components {
                zoom: false,
                rotate: false,
                perspective: false,
                ..Components::ALL
            },
        }
    }
}

/// The controls of the stabilizer (see the module).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// Whether the camera is taken to have stood still.
    pub camera_lock: bool,
    /// Whether the picture is enlarged until no frame shows an edge.
    pub zoom: bool,
    /// The share of the frame that the corrected picture must cover, from 0 to 1.
    pub cropping_ratio: f64,
    /// How much the path is smoothed, from 0 to 1.
    pub smooth: f64,
    /// How much of the correction is made, from −1 to 1.
    pub strength: f64,
}

impl Default for Settings {
    /// The controls as `DaVinci` shows them at first, to be checked against it: Zoom on, a
    /// Cropping Ratio of 0.5, a Smooth of 0.25 and a Strength of 1.
    fn default() -> Self {
        Self {
            camera_lock: false,
            zoom: true,
            cropping_ratio: 0.5,
            smooth: 0.25,
            strength: 1.0,
        }
    }
}

/// The path of the camera: for each frame, the motion of the picture since the first frame,
/// about `centre`, with only the components that `method` corrects. `steps[i]` is how the
/// picture moves from frame `i` to frame `i + 1`, so the path has one motion more than there
/// are steps, the first being no motion. Turns are counted on: a camera that goes round does not
/// jump back by a whole turn.
///
/// None where the steps, put together, turn the picture over or send its centre away, which no
/// camera does.
#[must_use]
pub fn path(steps: &[Mat3], centre: Point, method: Method) -> Option<Vec<Motion>> {
    let enabled = method.components();
    let mut whole = Mat3::IDENTITY;
    let mut path = vec![Motion::IDENTITY];
    let mut turn = 0.0_f64;
    for step in steps {
        whole = step.mul(&whole).normalized();
        let mut motion = Motion::from_matrix(&whole, centre)?;
        // The angle of a matrix is within half a turn of none: it is brought to within half a
        // turn of the one before.
        let turns = ((turn - motion.rotate) / std::f64::consts::TAU).round();
        motion.rotate += turns * std::f64::consts::TAU;
        turn = motion.rotate;
        path.push(motion.masked(enabled));
    }
    Some(path)
}

/// Whether a component is a factor, which is averaged in its logarithm.
fn is_factor(component: Component) -> bool {
    matches!(component, Component::Zoom | Component::Aspect)
}

/// A path with each number averaged over its neighbours by the weights of a Gaussian of
/// standard deviation `sigma`, in frames. Near the ends the average is of the frames that there
/// are, so a path that stands still stays where it is. A `sigma` of nothing leaves the path as
/// it is.
#[must_use]
pub fn smoothed(path: &[Motion], sigma: f64) -> Vec<Motion> {
    if sigma.is_nan() || sigma <= 0.0 || path.len() < 2 {
        return path.to_vec();
    }
    // Beyond three deviations a Gaussian weighs less than a hundredth of its middle.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a count of frames, positive and far below 2^32"
    )]
    let reach = (3.0 * sigma).ceil().min(1e6) as usize;
    let weights: Vec<f64> = (0..=reach)
        .map(|away| {
            #[allow(clippy::cast_precision_loss, reason = "a count of frames")]
            let away = away as f64 / sigma;
            (-0.5 * away * away).exp()
        })
        .collect();
    let read = |motion: &Motion, component: Component| {
        let value = motion.get(component);
        if is_factor(component) {
            value.ln()
        } else {
            value
        }
    };
    (0..path.len())
        .map(|at| {
            let from = at.saturating_sub(reach);
            let to = (at + reach).min(path.len() - 1);
            let mut out = path[at];
            for component in Component::ALL {
                let (mut sum, mut weight) = (0.0, 0.0);
                for (near, motion) in path.iter().enumerate().take(to + 1).skip(from) {
                    let w = weights[near.abs_diff(at)];
                    sum += w * read(motion, component);
                    weight += w;
                }
                let mean = sum / weight;
                out.set(
                    component,
                    if is_factor(component) {
                        mean.exp()
                    } else {
                        mean
                    },
                );
            }
            out
        })
        .collect()
}

/// Whether the picture, put through `transform`, covers the part of a frame of `size` that is
/// `ratio` of its width and of its height, about its centre.
#[must_use]
pub fn covers(transform: &Mat3, size: (f64, f64), ratio: f64) -> bool {
    let Some(back) = transform.inverse() else {
        return false;
    };
    let (width, height) = size;
    let (across, down) = (0.5 * ratio * width, 0.5 * ratio * height);
    // A little is forgiven: the sums that find a share end a hair inside or outside.
    let slack = 1e-9 * width.max(height);
    [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]]
        .iter()
        .all(|corner| {
            let [x, y] = back.apply([
                0.5 * width + corner[0] * across,
                0.5 * height + corner[1] * down,
            ]);
            x.is_finite()
                && y.is_finite()
                && (-slack..=width + slack).contains(&x)
                && (-slack..=height + slack).contains(&y)
        })
}

/// The largest value from 0 to 1 that `holds` for, where it holds for 0 and, once it fails,
/// fails for everything larger.
fn largest(holds: impl Fn(f64) -> bool) -> f64 {
    if holds(1.0) {
        return 1.0;
    }
    let (mut low, mut high) = (0.0_f64, 1.0_f64);
    for _ in 0..HALVINGS {
        let middle = f64::midpoint(low, high);
        if holds(middle) {
            low = middle;
        } else {
            high = middle;
        }
    }
    low
}

/// What the stabilizer makes of a path.
#[derive(Clone, Debug, PartialEq)]
pub struct Stabilized {
    /// For each frame, the transform that its picture is drawn through: where a point of the
    /// frame goes.
    pub transforms: Vec<Mat3>,
    /// For each frame, how much of the correction that was wanted could be made within the
    /// Cropping Ratio, from 0 to 1.
    pub shares: Vec<f64>,
    /// The factor that Zoom enlarges every frame by: 1 where Zoom is off.
    pub zoom: f64,
}

/// Stabilize a path ([`path`]) of a clip whose frames are `size` wide and high, by the controls
/// (see the module).
#[must_use]
pub fn stabilized(path: &[Motion], settings: &Settings, size: (f64, f64)) -> Stabilized {
    let centre = [0.5 * size.0, 0.5 * size.1];
    let ratio = settings.cropping_ratio.clamp(0.0, 1.0);
    let strength = settings.strength.clamp(-1.0, 1.0);
    let target = if settings.camera_lock {
        vec![path.first().copied().unwrap_or(Motion::IDENTITY); path.len()]
    } else {
        smoothed(path, settings.smooth.clamp(0.0, 1.0) * SMOOTH_FRAMES)
    };
    let mut motions = Vec::with_capacity(path.len());
    let mut shares = Vec::with_capacity(path.len());
    for (seen, wanted) in path.iter().zip(&target) {
        // The frame is taken back to where the first one was, and from there to where the
        // smooth camera is.
        let whole = seen
            .matrix(centre)
            .inverse()
            .map(|back| wanted.matrix(centre).mul(&back));
        let whole = match (whole, strength < 0.0) {
            (Some(whole), false) => Some(whole),
            // The opposite of the correction is the shake itself.
            (Some(whole), true) => whole.inverse(),
            (None, _) => None,
        };
        let correction = whole
            .and_then(|whole| Motion::from_matrix(&whole.normalized(), centre))
            .map_or(Motion::IDENTITY, |whole| {
                Motion::IDENTITY.lerp(&whole, strength.abs())
            });
        let part = |share: f64| Motion::IDENTITY.lerp(&correction, share);
        let share = if settings.camera_lock {
            1.0
        } else if ratio >= 1.0 {
            // The whole frame is to stay covered, which the manual has as no stabilizing at
            // all: not even a correction that enlarges the picture as far as it shifts it.
            0.0
        } else {
            largest(|share| covers(&part(share).matrix(centre), size, ratio))
        };
        shares.push(share);
        motions.push(part(share));
    }
    let matrices: Vec<Mat3> = motions.iter().map(|motion| motion.matrix(centre)).collect();
    let zoom = if settings.zoom {
        // The share of the frame that every frame covers is the share that fills the frame once
        // it is enlarged.
        let covered = matrices
            .iter()
            .map(|matrix| largest(|share| covers(matrix, size, share)))
            .fold(1.0_f64, f64::min);
        if covered > 0.0 {
            1.0 / covered
        } else {
            1.0
        }
    } else {
        1.0
    };
    let enlarge = Motion {
        zoom,
        ..Motion::IDENTITY
    }
    .matrix(centre);
    Stabilized {
        transforms: matrices.iter().map(|matrix| enlarge.mul(matrix)).collect(),
        shares,
        zoom,
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::float_cmp,
        reason = "a share of all or none, and a zoom of none, are those numbers as written"
    )]

    use super::*;
    use proptest::prelude::*;

    const SIZE: (f64, f64) = (1920.0, 1080.0);
    const CENTRE: Point = [960.0, 540.0];

    fn shift(x: f64, y: f64) -> Motion {
        Motion {
            pan: x,
            tilt: y,
            ..Motion::IDENTITY
        }
    }

    /// The steps of a path: how the picture goes from each of its frames to the next.
    fn steps_of(path: &[Motion]) -> Vec<Mat3> {
        path.windows(2)
            .map(|pair| {
                pair[1]
                    .matrix(CENTRE)
                    .mul(&pair[0].matrix(CENTRE).inverse().unwrap())
            })
            .collect()
    }

    fn close(a: Point, b: Point, within: f64) -> bool {
        (a[0] - b[0]).abs() <= within && (a[1] - b[1]).abs() <= within
    }

    /// A camera that drifts to the right and shakes: a shift, a small turn and a breath of zoom.
    fn shaky(frames: usize) -> Vec<Motion> {
        (0..frames)
            .map(|frame| {
                #[allow(clippy::cast_precision_loss, reason = "a few frames")]
                let t = frame as f64;
                Motion {
                    pan: 2.0 * t + 9.0 * (t * 1.7).sin(),
                    tilt: 6.0 * (t * 2.3).cos() - 6.0,
                    zoom: 1.0 + 0.01 * (t * 0.9).sin(),
                    rotate: 0.01 * (t * 1.3).sin(),
                    ..Motion::IDENTITY
                }
            })
            .collect()
    }

    #[test]
    fn the_path_is_the_steps_put_one_after_another() {
        let wanted = shaky(40);
        let found = path(&steps_of(&wanted), CENTRE, Method::Similarity).unwrap();
        assert_eq!(found.len(), 40);
        assert_eq!(found[0], Motion::IDENTITY);
        for (found, wanted) in found.iter().zip(&wanted) {
            for component in Component::ALL {
                let (a, b) = (found.get(component), wanted.get(component));
                assert!((a - b).abs() < 1e-9, "{component:?}: {a} for {b}");
            }
        }
        assert_eq!(
            path(&[], CENTRE, Method::Perspective),
            Some(vec![Motion::IDENTITY])
        );
    }

    #[test]
    fn a_method_takes_the_part_of_the_motion_that_it_corrects() {
        let turning = Motion {
            pan: 5.0,
            tilt: -3.0,
            zoom: 1.2,
            rotate: 0.3,
            yaw: 1e-5,
            ..Motion::IDENTITY
        };
        let steps = [turning.matrix(CENTRE)];
        let of = |method| path(&steps, CENTRE, method).unwrap()[1];
        assert!((of(Method::Perspective).yaw - 1e-5).abs() < 1e-12);
        let similar = of(Method::Similarity);
        assert_eq!((similar.yaw, similar.aspect), (0.0, 1.0));
        assert!((similar.rotate - 0.3).abs() < 1e-12 && (similar.zoom - 1.2).abs() < 1e-12);
        let shifted = of(Method::Translation);
        assert!((shifted.pan - 5.0).abs() < 1e-9 && (shifted.tilt + 3.0).abs() < 1e-9);
        assert_eq!((shifted.zoom, shifted.rotate, shifted.yaw), (1.0, 0.0, 0.0));
        assert!(Method::Perspective.components().perspective);
        assert!(
            !Method::Similarity.components().perspective && Method::Similarity.components().rotate
        );
        assert!(!Method::Translation.components().zoom && Method::Translation.components().tilt);
    }

    #[test]
    fn a_camera_that_goes_round_does_not_jump_back_by_a_turn() {
        // A fifth of a turn with each frame: three turns in fifteen frames.
        let step = Motion {
            rotate: std::f64::consts::TAU / 5.0,
            ..Motion::IDENTITY
        }
        .matrix(CENTRE);
        let found = path(&[step; 15], CENTRE, Method::Similarity).unwrap();
        for (frame, motion) in found.iter().enumerate() {
            #[allow(clippy::cast_precision_loss, reason = "a few frames")]
            let wanted = frame as f64 * std::f64::consts::TAU / 5.0;
            assert!((motion.rotate - wanted).abs() < 1e-9, "frame {frame}");
        }
        // A picture turned over is no path of a camera.
        let mirror = Mat3([[-1.0, 0.0, 1920.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);
        assert_eq!(path(&[mirror], CENTRE, Method::Perspective), None);
    }

    #[test]
    fn smoothing_leaves_a_still_path_and_a_steady_one_and_takes_the_shake_out() {
        let still = vec![shift(4.0, -2.0); 30];
        assert_eq!(smoothed(&still, 10.0), still);
        // A steady pan stays a steady pan, away from the ends.
        let steady: Vec<Motion> = (0..200).map(|f| shift(f64::from(f) * 3.0, 0.0)).collect();
        let smooth = smoothed(&steady, 5.0);
        for frame in 20..180 {
            assert!(
                (smooth[frame].pan - steady[frame].pan).abs() < 1e-6,
                "{frame}"
            );
        }
        // A shake about a steady pan is much smaller afterwards.
        let shaken: Vec<Motion> = (0..200)
            .map(|f| {
                let t = f64::from(f);
                shift(t * 3.0 + 8.0 * (t * 2.0).sin(), 0.0)
            })
            .collect();
        let smooth = smoothed(&shaken, 5.0);
        for frame in 20..180 {
            assert!(
                (smooth[frame].pan - steady[frame].pan).abs() < 0.2,
                "{frame}"
            );
        }
        // No width leaves the path as it is, and so does a path of one frame.
        assert_eq!(smoothed(&shaken, 0.0), shaken);
        assert_eq!(smoothed(&shaken, f64::NAN), shaken);
        assert_eq!(smoothed(&shaken[..1], 5.0), shaken[..1].to_vec());
        assert_eq!(smoothed(&[], 5.0), Vec::new());
    }

    #[test]
    fn a_zoom_is_smoothed_as_a_factor() {
        // Twice as large, and half as large, in turns: the middle of the two is no zoom.
        let breathing: Vec<Motion> = (0..100)
            .map(|frame| Motion {
                zoom: if frame % 2 == 0 { 2.0 } else { 0.5 },
                ..Motion::IDENTITY
            })
            .collect();
        let smooth = smoothed(&breathing, 8.0);
        for motion in &smooth[30..70] {
            assert!((motion.zoom - 1.0).abs() < 0.01, "{}", motion.zoom);
        }
    }

    #[test]
    fn a_locked_camera_puts_every_frame_where_the_first_was() {
        let seen = shaky(60);
        let locked = Settings {
            camera_lock: true,
            zoom: false,
            // Neither has a say where the camera is locked.
            cropping_ratio: 1.0,
            smooth: 0.0,
            strength: 1.0,
        };
        let made = stabilized(&seen, &locked, SIZE);
        assert_eq!((made.zoom, made.shares.len()), (1.0, 60));
        assert!(made.shares.iter().all(|share| *share == 1.0));
        for (transform, seen) in made.transforms.iter().zip(&seen) {
            // A thing of the first frame, where the shake put it, comes back.
            for thing in [[300.0, 200.0], [960.0, 540.0], [1700.0, 900.0]] {
                let shaken = seen.matrix(CENTRE).apply(thing);
                assert!(close(transform.apply(shaken), thing, 1e-6));
            }
        }
    }

    #[test]
    fn strength_makes_the_correction_whole_or_none_or_its_opposite() {
        let seen = shaky(60);
        let with = |strength: f64| {
            let settings = Settings {
                camera_lock: true,
                zoom: false,
                strength,
                ..Settings::default()
            };
            stabilized(&seen, &settings, SIZE).transforms
        };
        let thing = [400.0, 300.0];
        for (frame, seen) in seen.iter().enumerate() {
            // None: the frame stays where it is.
            assert!(close(with(0.0)[frame].apply(thing), thing, 1e-9));
            // The opposite: a still thing is given the shake of the clip.
            let shaken = seen.matrix(CENTRE).apply(thing);
            assert!(
                close(with(-1.0)[frame].apply(thing), shaken, 1e-6),
                "{frame}"
            );
            // Half of a shift is half the way.
            let whole = with(1.0)[frame].apply(CENTRE);
            let half = with(0.5)[frame].apply(CENTRE);
            assert!(close(
                half,
                [
                    f64::midpoint(whole[0], CENTRE[0]),
                    f64::midpoint(whole[1], CENTRE[1])
                ],
                1e-6
            ));
        }
        // More than whole is whole.
        assert_eq!(with(7.0), with(1.0));
    }

    #[test]
    fn without_smoothing_and_with_the_whole_frame_kept_nothing_is_corrected() {
        let seen = shaky(60);
        let unsmoothed = Settings {
            smooth: 0.0,
            zoom: false,
            ..Settings::default()
        };
        for transform in stabilized(&seen, &unsmoothed, SIZE).transforms {
            assert!(close(transform.apply([100.0, 100.0]), [100.0, 100.0], 1e-6));
        }
        let uncropped = Settings {
            cropping_ratio: 1.0,
            ..Settings::default()
        };
        let made = stabilized(&seen, &uncropped, SIZE);
        assert!((made.zoom - 1.0).abs() < 1e-6);
        for (transform, share) in made.transforms.iter().zip(&made.shares) {
            assert_eq!(*share, 0.0);
            assert_eq!(*transform, Mat3::IDENTITY);
            assert!(close(transform.apply([100.0, 100.0]), [100.0, 100.0], 1e-3));
        }
    }

    #[test]
    fn the_shake_is_taken_out_and_the_drift_is_kept() {
        let seen = shaky(120);
        let settings = Settings {
            zoom: false,
            smooth: 0.25,
            ..Settings::default()
        };
        let made = stabilized(&seen, &settings, SIZE);
        // Where the centre of the first frame is seen, frame by frame: as the camera shook, and
        // after the correction. The jumps between frames are far smaller afterwards, and the
        // picture has still gone to the right.
        let before: Vec<Point> = seen
            .iter()
            .map(|m| m.matrix(CENTRE).apply(CENTRE))
            .collect();
        let after: Vec<Point> = made
            .transforms
            .iter()
            .zip(&before)
            .map(|(transform, at)| transform.apply(*at))
            .collect();
        let roughness = |points: &[Point]| {
            points[20..100]
                .windows(3)
                .map(|w| (w[0][0] - 2.0 * w[1][0] + w[2][0]).abs())
                .fold(0.0, f64::max)
        };
        assert!(roughness(&after) < roughness(&before) / 20.0);
        assert!(after[100][0] - after[20][0] > 150.0);
        assert!(made.shares.iter().all(|share| *share == 1.0));
    }

    #[test]
    fn zoom_enlarges_by_one_factor_until_no_frame_shows_an_edge() {
        let seen = shaky(120);
        let made = stabilized(&seen, &Settings::default(), SIZE);
        assert!(made.zoom > 1.0 && made.zoom <= 2.0 + 1e-6, "{}", made.zoom);
        for transform in &made.transforms {
            assert!(covers(transform, SIZE, 1.0));
        }
        // Without it the frames are as they were corrected, and some show an edge.
        let unzoomed = Settings {
            zoom: false,
            ..Settings::default()
        };
        let plain = stabilized(&seen, &unzoomed, SIZE);
        assert_eq!(plain.zoom, 1.0);
        assert!(plain.transforms.iter().any(|t| !covers(t, SIZE, 1.0)));
        // A still camera needs none.
        let still = stabilized(&vec![Motion::IDENTITY; 10], &Settings::default(), SIZE);
        assert_eq!(still.zoom, 1.0);
        assert!(still.transforms.iter().all(|t| *t == Mat3::IDENTITY));
        assert_eq!(
            stabilized(&[], &Settings::default(), SIZE).transforms,
            Vec::new()
        );
    }

    #[test]
    fn a_picture_covers_what_lies_within_it() {
        assert!(covers(&Mat3::IDENTITY, SIZE, 1.0));
        let right = shift(96.0, 0.0).matrix(CENTRE);
        // A tenth of the width is gone on one side: nine tenths about the centre... are not
        // covered, eight tenths are.
        assert!(!covers(&right, SIZE, 1.0));
        assert!(!covers(&right, SIZE, 0.95));
        assert!(covers(&right, SIZE, 0.9));
        assert!(covers(&right, SIZE, 0.5));
        let singular = Mat3([[0.0; 3]; 3]);
        assert!(!covers(&singular, SIZE, 0.1));
    }

    proptest! {
        #[test]
        fn what_must_stay_covered_stays_covered(
            shakes in prop::collection::vec((-300.0_f64..300.0, -200.0_f64..200.0, -0.2_f64..0.2, 0.8_f64..1.25), 2..40),
            ratio in 0.3_f64..1.0,
            smooth in 0.0_f64..1.0,
            strength in -1.0_f64..1.0,
            zoom in any::<bool>(),
        ) {
            let seen: Vec<Motion> = shakes
                .iter()
                .map(|(pan, tilt, rotate, zoom)| Motion {
                    pan: *pan,
                    tilt: *tilt,
                    rotate: *rotate,
                    zoom: *zoom,
                    ..Motion::IDENTITY
                })
                .collect();
            let settings = Settings { camera_lock: false, zoom, cropping_ratio: ratio, smooth, strength };
            let made = stabilized(&seen, &settings, SIZE);
            prop_assert_eq!(made.transforms.len(), seen.len());
            prop_assert!(made.zoom >= 1.0 && made.zoom <= 1.0 / ratio + 1e-6, "{}", made.zoom);
            let kept = if zoom { 1.0 } else { ratio };
            for (transform, share) in made.transforms.iter().zip(&made.shares) {
                prop_assert!((0.0..=1.0).contains(share));
                // Enlarged, the picture covers the whole frame; as it is, the share that it must.
                prop_assert!(covers(transform, SIZE, kept));
            }
        }
    }
}
