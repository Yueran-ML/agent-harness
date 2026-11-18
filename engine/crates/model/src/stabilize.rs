//! The stabilizer of the Edit and Cut pages (Reference Manual of Resolve 20, pp. 633–634 and
//! 1064–1065), as a clip keeps it: its Method and its controls, and what its analysis measured
//! of the motion of the picture of the clip, frame by frame.
//!
//! # What a clip keeps
//!
//! A clip of a track of picture may have `stabilize` ([`Stabilize`]): the Method and the controls
//! of the stabilizer, and, once the engine has measured the clip, its [`Analysis`]: for each
//! frame of its source, the motion of the whole picture since the first frame measured. A clip
//! that has none is written without the field, as it always was, and shows its source as it
//! did. A clip of sound has none.
//!
//! The controls are those of [`columbina_track::stabilize`], fixed numbers that take no keys:
//! Camera Lock, Zoom, a Cropping Ratio and a Smooth from 0 to 1, and a Strength from −1 to 1. A
//! clip that is given a stabilizer has them as the Inspector shows them at first
//! ([`Stabilize::default`]): Perspective, Zoom on, a Cropping Ratio of 0.5, a Smooth of 0.25 and
//! a Strength of 1.
//!
//! # What needs a new analysis
//!
//! The analysis holds the motion as it was measured, not what the controls make of it: so a
//! change of Camera Lock, Zoom, the Cropping Ratio, Smooth or Strength is drawn at once from the
//! analysis that the clip has, as the whole clip is worked out anew from it, and a client that
//! changes them sends them alone ([`Controls`]), not the analysis with them. The Method is
//! another matter. The motion is measured as a homography for Perspective and as a similarity
//! for Similarity and Translation, so an analysis says which Method it was measured for
//! ([`Analysis::method`]). One measured for another Method than the clip's is kept, and is
//! still drawn, by the part of its motion that the clip's Method corrects; and it says that the
//! clip is to be measured again ([`Stabilize::needs_analysis`]), as the manual has the Method
//! chosen before Stabilize is pressed.
//!
//! # The frames
//!
//! The frames of an analysis are those of the source of the clip, as the keys of its grade
//! count them ([`crate::keyframes::KeyClock`]): so a clip that is trimmed, cut in two, copied,
//! played faster, slower or backward, or along speed points, keeps the motion of each picture of
//! its source, and the stabilizer puts each picture where it put it before. The whole of the
//! analysis is stabilized as one path, whatever part of it the clip shows. A frame before the
//! first of the analysis is put as that one is, and any other that the analysis has not as the
//! one before it.
//!
//! # Coordinates
//!
//! The motion is in pixels of the frames that were measured ([`Analysis::size`]): `[x, y]` from
//! the top left corner of the picture, y down, a pixel one unit wide, so that the middle of the
//! picture is `[width / 2, height / 2]`; about the middle each motion is the eight numbers of a
//! [`Motion`], in the order of its fields: `pan`, `tilt`, `zoom`, `rotate`, `aspect`, `shear`,
//! `yaw` and `pitch`. What the stabilizer makes of it ([`Stabilize::transform_at`]) is a 3×3
//! matrix in shares of the picture, x of its width and y of its height from its top left
//! corner: the same at whatever size the picture is drawn.
//!
//! # Where it is drawn
//!
//! The stabilizer is done to the picture that the composition of the clip makes
//! ([`crate::fusion`]), or to its source where it has none: after the composition and before
//! the grade, the transform, the crop and the composite mode of the clip, as Resolve does the
//! Fusion page before the stabilization of the Inspector. So the Fusion page sees the picture
//! as it was shot (see [`crate::resolve`]). The analysis is measured on the frames of the
//! source: of a composition that moves the whole picture, it is not the motion of what is
//! stabilized.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use columbina_track::stabilize::{path, stabilized, Settings};
use columbina_track::{Component, Mat3, Motion};
use serde::{Deserialize, Serialize};

use crate::keyframes::KeyClock;
use crate::ops::{OpError, LARGEST};
use crate::project::{Clip, Project};
use crate::time::Rate;

pub use columbina_track::stabilize::Method;

/// The most pixels of a side of the frames that an analysis measured.
pub const MOST_SIDE: u32 = 16384;

/// A transform of the plane, as a 3×3 matrix acting on `[x, y, 1]`, row by row.
pub type Matrix = [[f64; 3]; 3];

/// The transform that leaves every point where it is.
pub const IDENTITY: Matrix = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

/// The stabilizer of a clip (see the module).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stabilize {
    /// How much of the motion of the picture is taken to be the camera's: all of a homography
    /// (Perspective), a shift with a turn and a zoom (Similarity), or a shift alone
    /// (Translation).
    pub method: Method,
    /// Camera Lock: the camera is taken to have stood still where it was at the first frame,
    /// and Smooth and the Cropping Ratio have no say.
    pub camera_lock: bool,
    /// Zoom: the picture is enlarged about its middle, by one factor for the whole clip, until
    /// no frame shows an edge.
    pub zoom: bool,
    /// The share of the frame, about its middle, that the corrected picture always covers,
    /// from 0 to 1: at 1 nothing is corrected, and lower values let the stabilizer do more.
    pub cropping_ratio: f64,
    /// How much the path of the camera is smoothed, from 0 to 1.
    pub smooth: f64,
    /// How much of the correction is made, from −1 to 1: 1 all of it, 0 none, and −1 its
    /// opposite, which gives a still picture the shake of the clip.
    pub strength: f64,
    /// What was measured of the motion of the picture of the clip; none before it is measured,
    /// and nothing is corrected. It may be left out, as none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub analysis: Option<Analysis>,
}

impl Default for Stabilize {
    /// The stabilizer as the Inspector of `DaVinci` shows it at first: Perspective, Camera
    /// Lock off, Zoom on, a Cropping Ratio of 0.5, a Smooth of 0.25 and a Strength of 1, with
    /// no analysis.
    fn default() -> Self {
        let settings = Settings::default();
        Self {
            method: Method::default(),
            camera_lock: settings.camera_lock,
            zoom: settings.zoom,
            cropping_ratio: settings.cropping_ratio,
            smooth: settings.smooth,
            strength: settings.strength,
            analysis: None,
        }
    }
}

impl Stabilize {
    /// Its controls, as the mathematics of the stabilizer takes them.
    #[must_use]
    pub fn settings(&self) -> Settings {
        Settings {
            camera_lock: self.camera_lock,
            zoom: self.zoom,
            cropping_ratio: self.cropping_ratio,
            smooth: self.smooth,
            strength: self.strength,
        }
    }

    /// Whether the clip is to be measured before it is stabilized as its Method says: it has
    /// no analysis, or one that was measured for another Method (see the module).
    #[must_use]
    pub fn needs_analysis(&self) -> bool {
        self.analysis
            .as_ref()
            .is_none_or(|analysis| analysis.method != self.method)
    }

    /// The transform that the picture of frame `frame` of the source of the clip is drawn
    /// through: where each point of the picture is put, in shares of the picture (see the
    /// module). It is that of [`columbina_track::stabilize::stabilized`] of the path of the
    /// camera that the analysis gives for the Method of the clip, by its controls, at the frame
    /// of the analysis that is `frame`, or the one before it, or the first. None where there is
    /// no analysis, or where its path turns the picture over, which no camera does.
    #[must_use]
    pub fn transform_at(&self, frame: i64) -> Option<Matrix> {
        self.analysis
            .as_ref()?
            .transform_at(frame, self.method, &self.settings())
    }

    /// It with each frame of its analysis at the frame `to(frame)`, as
    /// [`crate::grade::Param::with_key_frames`] moves the keys of a setting: frames that come to
    /// one are one, the last of them.
    #[must_use]
    pub fn with_key_frames(&self, to: &impl Fn(i64) -> i64) -> Stabilize {
        Stabilize {
            method: self.method,
            camera_lock: self.camera_lock,
            zoom: self.zoom,
            cropping_ratio: self.cropping_ratio,
            smooth: self.smooth,
            strength: self.strength,
            analysis: self
                .analysis
                .as_ref()
                .map(|analysis| analysis.with_key_frames(to)),
        }
    }

    /// Its own values of the controls that `given` gives, and none of the others: what puts
    /// those controls back as they are once `given` has been set ([`Stabilize::set_controls`]).
    #[must_use]
    pub fn controls(&self, given: &Controls) -> Controls {
        Controls {
            method: given.method.map(|_| self.method),
            camera_lock: given.camera_lock.map(|_| self.camera_lock),
            zoom: given.zoom.map(|_| self.zoom),
            cropping_ratio: given.cropping_ratio.map(|_| self.cropping_ratio),
            smooth: given.smooth.map(|_| self.smooth),
            strength: given.strength.map(|_| self.strength),
        }
    }

    /// Put each control that `given` gives in place of its own, and keep the others and its
    /// analysis as they are. A change of the Method keeps the analysis too, which is then one
    /// of another Method ([`Stabilize::needs_analysis`]).
    pub fn set_controls(&mut self, given: &Controls) {
        self.method = given.method.unwrap_or(self.method);
        self.camera_lock = given.camera_lock.unwrap_or(self.camera_lock);
        self.zoom = given.zoom.unwrap_or(self.zoom);
        self.cropping_ratio = given.cropping_ratio.unwrap_or(self.cropping_ratio);
        self.smooth = given.smooth.unwrap_or(self.smooth);
        self.strength = given.strength.unwrap_or(self.strength);
    }

    /// Check its controls and its analysis: the Cropping Ratio and Smooth from 0 to 1, the
    /// Strength from −1 to 1; and of the analysis, frames of 1 to [`MOST_SIDE`] pixels a side,
    /// one frame at least, each within [`LARGEST`] of 0, and motions of finite numbers whose
    /// `zoom` and `aspect` are above 0.
    pub(crate) fn check(&self) -> Result<(), OpError> {
        let checks = [
            (
                (0.0..=1.0).contains(&self.cropping_ratio),
                "stabilize.cropping_ratio",
            ),
            ((0.0..=1.0).contains(&self.smooth), "stabilize.smooth"),
            ((-1.0..=1.0).contains(&self.strength), "stabilize.strength"),
            (
                self.analysis.as_ref().is_none_or(Analysis::is_valid),
                "stabilize.analysis",
            ),
        ];
        match checks.into_iter().find(|(holds, _)| !holds) {
            Some((_, what)) => Err(OpError::OutOfRange { what }),
            None => Ok(()),
        }
    }
}

/// Some of the controls of a stabilizer, each given or not: those that an operation changes
/// alone, its analysis kept as it is ([`crate::ops::Op::SetStabilizeSettings`]), so that a
/// change of a control does not carry the whole analysis. It is written as the controls of a
/// [`Stabilize`] are, each left out where it is not given: `{"smooth": 0.5, "strength": 1.0}`.
/// Each is within the range of the control that it is ([`Stabilize`]'s check).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Controls {
    /// The Method ([`Stabilize::method`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<Method>,
    /// Camera Lock ([`Stabilize::camera_lock`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera_lock: Option<bool>,
    /// Zoom ([`Stabilize::zoom`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zoom: Option<bool>,
    /// The Cropping Ratio, from 0 to 1 ([`Stabilize::cropping_ratio`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cropping_ratio: Option<f64>,
    /// Smooth, from 0 to 1 ([`Stabilize::smooth`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smooth: Option<f64>,
    /// Strength, from −1 to 1 ([`Stabilize::strength`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strength: Option<f64>,
}

/// What the analysis of a clip measured (see the module): the frames that were measured, the
/// Method that they were measured for, and for each frame of the source the motion of the whole
/// picture since the first.
///
/// It is written `{"method": "similarity", "size": [1280, 720], "path": {"12": [pan, tilt,
/// zoom, rotate, aspect, shear, yaw, pitch], ...}}`. What the stabilizer makes of it by a
/// Method and controls is worked out once for each, and kept with it while they stay.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Analysis {
    /// The Method that the motion was measured for: as a homography for Perspective, and as a
    /// similarity for the others.
    method: Method,
    /// The width and the height of the frames that were measured, upright, in pixels.
    size: [u32; 2],
    /// For each frame of the source, the motion of the picture since the first frame, about
    /// the middle of the frame, as the eight numbers of a [`Motion`].
    #[serde(deserialize_with = "frames")]
    path: BTreeMap<i64, [f64; 8]>,
    /// What the stabilizer made of it last.
    #[serde(skip)]
    solved: Solved,
}

/// Read the frames of an analysis, whose keys JSON writes as strings ("12"). Serde reads such a
/// key as a number from the text of JSON, but not from an operation that it has already read
/// once to find its kind (`set_stabilize` among the operations): so they are read as strings,
/// and then as frames.
fn frames<'de, D>(deserializer: D) -> Result<BTreeMap<i64, [f64; 8]>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let written = BTreeMap::<String, [f64; 8]>::deserialize(deserializer)?;
    written
        .into_iter()
        .map(|(key, motion)| match key.parse::<i64>() {
            Ok(frame) => Ok((frame, motion)),
            Err(_) => Err(serde::de::Error::custom(format!(
                "{key:?} is not a frame of an analysis"
            ))),
        })
        .collect()
}

impl PartialEq for Analysis {
    /// Two analyses are one where they measured the same: what was worked out of them is not
    /// looked at.
    fn eq(&self, other: &Self) -> bool {
        self.method == other.method && self.size == other.size && self.path == other.path
    }
}

impl fmt::Debug for Analysis {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Analysis")
            .field("method", &self.method)
            .field("size", &self.size)
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

/// The motion that eight numbers of an analysis stand for, in the order of the fields of a
/// [`Motion`].
#[must_use]
pub fn motion_of(numbers: &[f64; 8]) -> Motion {
    let [pan, tilt, zoom, rotate, aspect, shear, yaw, pitch] = *numbers;
    Motion {
        pan,
        tilt,
        zoom,
        rotate,
        aspect,
        shear,
        yaw,
        pitch,
    }
}

/// The eight numbers of a motion, as an analysis keeps them: [`motion_of`] the other way.
#[must_use]
pub fn numbers_of(motion: &Motion) -> [f64; 8] {
    Component::ALL.map(|component| motion.get(component))
}

/// Whether a transform moves a point of the picture: whether it is not [`IDENTITY`] to the
/// bit.
#[must_use]
#[allow(
    clippy::float_cmp,
    reason = "the numbers of the identity, which a transform that moves nothing has exactly"
)]
pub fn moves(matrix: &Matrix) -> bool {
    *matrix != IDENTITY
}

/// The transform that the stabilizer of `clip`, a clip of a timeline of `rate` of `project`,
/// draws its picture through at frame `frame` of the timeline ([`Stabilize::transform_at`] at
/// the frame of its source that it shows there). None of a clip that has no stabilizer or no
/// analysis, and of a title, a colour, a still or media that the project has not, which have
/// no frames of a source to be stabilized.
#[must_use]
pub fn transform_of(clip: &Clip, project: &Project, rate: Rate, frame: i64) -> Option<Matrix> {
    let stabilize = clip.stabilize.as_ref()?;
    stabilize.analysis.as_ref()?;
    let clock = KeyClock::of(clip, project, rate);
    if !clock.counts_source() {
        return None;
    }
    stabilize.transform_at(clock.at(frame))
}

impl Analysis {
    /// An analysis of frames of `size` pixels, measured for `method`: `path` has, for each
    /// frame of the source that was measured, the motion of the picture since the first, about
    /// the middle of the frame ([`numbers_of`]).
    #[must_use]
    pub fn new(method: Method, size: [u32; 2], path: BTreeMap<i64, [f64; 8]>) -> Analysis {
        Analysis {
            method,
            size,
            path,
            solved: Solved::default(),
        }
    }

    /// The Method that it was measured for.
    #[must_use]
    pub fn method(&self) -> Method {
        self.method
    }

    /// The width and the height of the frames that were measured, upright, in pixels.
    #[must_use]
    pub fn size(&self) -> [u32; 2] {
        self.size
    }

    /// For each frame of the source that was measured, the motion of the picture since the
    /// first, as eight numbers ([`motion_of`]).
    #[must_use]
    pub fn path(&self) -> &BTreeMap<i64, [f64; 8]> {
        &self.path
    }

    /// It with each frame at the frame `to(frame)`: frames that come to one are one, the last
    /// of them.
    #[must_use]
    pub fn with_key_frames(&self, to: &impl Fn(i64) -> i64) -> Analysis {
        let path = self
            .path
            .iter()
            .map(|(&frame, motion)| (to(frame), *motion))
            .collect();
        Analysis::new(self.method, self.size, path)
    }

    /// Whether it is one that a clip may have (see [`Stabilize`]'s check).
    fn is_valid(&self) -> bool {
        let sides = self.size.iter().all(|side| (1..=MOST_SIDE).contains(side));
        let frames = self
            .path
            .keys()
            .all(|frame| frame.unsigned_abs() <= LARGEST.unsigned_abs());
        let motions = self.path.values().all(|numbers| {
            numbers.iter().all(|number| number.is_finite()) && numbers[2] > 0.0 && numbers[4] > 0.0
        });
        sides && frames && motions && !self.path.is_empty()
    }

    /// The transform at `frame` of the source for `method` and `settings` (see
    /// [`Stabilize::transform_at`]).
    fn transform_at(&self, frame: i64, method: Method, settings: &Settings) -> Option<Matrix> {
        let solution = self.solution(method, settings);
        let transforms = solution.transforms.as_ref()?;
        let at = solution
            .frames
            .partition_point(|measured| *measured <= frame)
            .saturating_sub(1);
        transforms.get(at).copied()
    }

    /// What the stabilizer makes of it for `method` and `settings`: kept from the last time
    /// where they are those, and worked out anew, and kept, where they are not.
    fn solution(&self, method: Method, settings: &Settings) -> Arc<Solution> {
        let mut kept = self.solved.0.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(solution) = kept
            .as_ref()
            .filter(|solution| solution.method == method && solution.settings == *settings)
        {
            return Arc::clone(solution);
        }
        let solution = Arc::new(self.solve(method, settings));
        *kept = Some(Arc::clone(&solution));
        solution
    }

    /// The transforms of every frame for `method` and `settings`: the steps from each frame to
    /// the next made the path of the camera for the Method
    /// ([`columbina_track::stabilize::path`]), and that stabilized by the controls
    /// ([`columbina_track::stabilize::stabilized`]), in pixels of the frames that were measured,
    /// and then made shares of them.
    fn solve(&self, method: Method, settings: &Settings) -> Solution {
        let frames: Vec<i64> = self.path.keys().copied().collect();
        let size = (f64::from(self.size[0]), f64::from(self.size[1]));
        let middle = [0.5 * size.0, 0.5 * size.1];
        let motions: Vec<Mat3> = self
            .path
            .values()
            .map(|numbers| motion_of(numbers).matrix(middle))
            .collect();
        let steps: Option<Vec<Mat3>> = motions
            .windows(2)
            .map(|pair| pair[0].inverse().map(|back| pair[1].mul(&back)))
            .collect();
        let transforms: Option<Vec<Matrix>> = steps
            .and_then(|steps| path(&steps, middle, method))
            .map(|camera| {
                stabilized(&camera, settings, size)
                    .transforms
                    .iter()
                    .map(|transform| in_shares(transform, size))
                    .collect()
            });
        Solution {
            method,
            settings: *settings,
            frames,
            transforms,
        }
    }
}

/// `transform`, of pixels of a picture of `size`, as a transform of shares of the picture:
/// `S⁻¹ · transform · S`, with `S` the map from shares to pixels.
fn in_shares(transform: &Mat3, size: (f64, f64)) -> Matrix {
    let scale = [size.0, size.1, 1.0];
    let mut out = [[0.0; 3]; 3];
    for (row, (cells, own)) in out.iter_mut().zip(transform.0).enumerate() {
        for (column, cell) in cells.iter_mut().enumerate() {
            *cell = own[column] * scale[column] / scale[row];
        }
    }
    out
}

/// What the stabilizer made of an analysis last, shared by its copies, which measured the same.
#[derive(Clone, Default)]
struct Solved(Arc<Mutex<Option<Arc<Solution>>>>);

/// The transforms of an analysis for one Method and one set of controls.
struct Solution {
    /// The Method.
    method: Method,
    /// The controls.
    settings: Settings,
    /// The frames of the analysis, in their order.
    frames: Vec<i64>,
    /// The transform of each, in shares of the picture; none where the path of the analysis
    /// turns the picture over.
    transforms: Option<Vec<Matrix>>,
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::float_cmp,
        reason = "numbers that are written and read back, and that are given exactly"
    )]

    use super::*;

    /// A camera that drifts right and shakes, about the middle of a frame of 640 × 360: a
    /// shift, a small turn and a breath of zoom.
    fn shaking(frames: i64) -> BTreeMap<i64, [f64; 8]> {
        (0..frames)
            .map(|frame| {
                let t = f64::from(i32::try_from(frame).unwrap());
                let motion = Motion {
                    pan: 2.0 * t + 6.0 * (t * 1.7).sin(),
                    tilt: 4.0 * (t * 2.3).cos() - 4.0,
                    zoom: 1.0 + 0.01 * (t * 0.9).sin(),
                    rotate: 0.01 * (t * 1.3).sin(),
                    ..Motion::IDENTITY
                };
                (frame + 50, numbers_of(&motion))
            })
            .collect()
    }

    fn stabilizer(path: BTreeMap<i64, [f64; 8]>) -> Stabilize {
        Stabilize {
            method: Method::Similarity,
            analysis: Some(Analysis::new(Method::Similarity, [640, 360], path)),
            ..Stabilize::default()
        }
    }

    #[test]
    fn eight_numbers_are_a_motion_and_back() {
        let motion = Motion {
            pan: 1.0,
            tilt: 2.0,
            zoom: 3.0,
            rotate: 4.0,
            aspect: 5.0,
            shear: 6.0,
            yaw: 7.0,
            pitch: 8.0,
        };
        assert_eq!(
            numbers_of(&motion),
            [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0]
        );
        assert_eq!(motion_of(&numbers_of(&motion)), motion);
    }

    #[test]
    fn the_transform_of_a_frame_is_that_of_the_mathematics_in_shares_of_the_picture() {
        let path = shaking(60);
        let stabilize = stabilizer(path.clone());
        // As the mathematics makes it, in pixels.
        let middle = [320.0, 180.0];
        let motions: Vec<Mat3> = path
            .values()
            .map(|numbers| motion_of(numbers).matrix(middle))
            .collect();
        let steps: Vec<Mat3> = motions
            .windows(2)
            .map(|pair| pair[1].mul(&pair[0].inverse().unwrap()))
            .collect();
        let camera = columbina_track::stabilize::path(&steps, middle, Method::Similarity).unwrap();
        let made = stabilized(&camera, &stabilize.settings(), (640.0, 360.0));
        for (at, frame) in (50..110).enumerate() {
            let shares = stabilize.transform_at(frame).unwrap();
            let pixels = &made.transforms[at];
            // A point of the picture, in shares and in pixels, goes to the same place.
            for point in [[0.25, 0.5], [0.5, 0.5], [0.9, 0.1]] {
                let [x, y] = pixels.apply([point[0] * 640.0, point[1] * 360.0]);
                let row =
                    |at: usize| shares[at][0] * point[0] + shares[at][1] * point[1] + shares[at][2];
                let across = row(0) / row(2);
                let down = row(1) / row(2);
                assert!((across * 640.0 - x).abs() < 1e-9, "frame {frame}");
                assert!((down * 360.0 - y).abs() < 1e-9, "frame {frame}");
            }
        }
        // Before the first frame, the first; after the last, the last.
        assert_eq!(stabilize.transform_at(0), stabilize.transform_at(50));
        assert_eq!(stabilize.transform_at(500), stabilize.transform_at(109));
    }

    #[test]
    fn a_frame_that_the_analysis_has_not_is_put_as_the_one_before_it() {
        let mut path = shaking(20);
        path.remove(&60);
        let stabilize = stabilizer(path);
        assert_eq!(stabilize.transform_at(60), stabilize.transform_at(59));
        assert_ne!(stabilize.transform_at(61), stabilize.transform_at(59));
    }

    #[test]
    fn a_change_of_the_controls_is_worked_out_anew_and_the_same_controls_are_kept() {
        let mut stabilize = stabilizer(shaking(60));
        let before = stabilize.transform_at(70).unwrap();
        let analysis = stabilize.analysis.clone().unwrap();
        // A copy shares what was worked out, and gives the same.
        assert_eq!(
            analysis.transform_at(70, Method::Similarity, &stabilize.settings()),
            Some(before)
        );
        stabilize.smooth = 1.0;
        let smoother = stabilize.transform_at(70).unwrap();
        assert_ne!(smoother, before);
        stabilize.smooth = 0.25;
        assert_eq!(stabilize.transform_at(70), Some(before));
        // No strength, no correction: every point stays where it is.
        stabilize.strength = 0.0;
        stabilize.zoom = false;
        assert!(!moves(&stabilize.transform_at(70).unwrap()));
    }

    #[test]
    fn a_still_camera_is_not_moved() {
        let still: BTreeMap<i64, [f64; 8]> = (0..10)
            .map(|frame| (frame, numbers_of(&Motion::IDENTITY)))
            .collect();
        let stabilize = stabilizer(still);
        for frame in 0..10 {
            assert_eq!(stabilize.transform_at(frame), Some(IDENTITY));
        }
        assert!(moves(&[
            [2.0, 0.0, -0.5],
            [0.0, 2.0, -0.5],
            [0.0, 0.0, 1.0]
        ]));
        assert!(!moves(&IDENTITY));
    }

    #[test]
    fn controls_are_set_alone_and_give_back_what_they_were() {
        let mut stabilize = stabilizer(shaking(3));
        let analysis = stabilize.analysis.clone();
        let given = Controls {
            method: Some(Method::Translation),
            smooth: Some(0.75),
            ..Controls::default()
        };
        let old = stabilize.controls(&given);
        assert_eq!(
            old,
            Controls {
                method: Some(Method::Similarity),
                smooth: Some(0.25),
                ..Controls::default()
            }
        );
        stabilize.set_controls(&given);
        assert_eq!(
            (stabilize.method, stabilize.smooth),
            (Method::Translation, 0.75)
        );
        // The analysis is kept, and is now of another Method.
        assert_eq!(stabilize.analysis, analysis);
        assert!(stabilize.needs_analysis());
        stabilize.set_controls(&old);
        assert_eq!(stabilize, stabilizer(shaking(3)));
        // Nothing given changes nothing.
        let none = Controls::default();
        stabilize.set_controls(&none);
        assert_eq!(stabilize, stabilizer(shaking(3)));
        assert_eq!(stabilize.controls(&none), none);
        // Written with what is given alone; a field that is not a control is refused.
        assert_eq!(
            serde_json::to_value(given).unwrap(),
            serde_json::json!({"method": "translation", "smooth": 0.75})
        );
        let written = serde_json::json!({"camera_lock": true, "strength": -1});
        let read: Controls = serde_json::from_value(written).unwrap();
        assert_eq!(
            read,
            Controls {
                camera_lock: Some(true),
                strength: Some(-1.0),
                ..Controls::default()
            }
        );
        let unknown = serde_json::json!({"smooth": 0.5, "analysis": null});
        assert!(serde_json::from_value::<Controls>(unknown).is_err());
    }

    #[test]
    fn a_method_other_than_that_of_the_analysis_asks_for_a_new_one() {
        let mut stabilize = stabilizer(shaking(10));
        assert!(!stabilize.needs_analysis());
        stabilize.method = Method::Translation;
        assert!(stabilize.needs_analysis());
        // It is still drawn, by the shift alone.
        let shifted = stabilize.transform_at(55).unwrap();
        assert_eq!((shifted[0][1], shifted[1][0]), (0.0, 0.0));
        assert!(Stabilize::default().needs_analysis());
        assert_eq!(Stabilize::default().transform_at(0), None);
    }

    #[test]
    fn the_frames_of_an_analysis_are_moved_and_those_that_come_to_one_are_the_last() {
        let stabilize = stabilizer(shaking(4));
        let later = stabilize.with_key_frames(&|frame| frame + 10);
        let analysis = later.analysis.as_ref().unwrap();
        assert_eq!(
            analysis.path().keys().copied().collect::<Vec<_>>(),
            [60, 61, 62, 63]
        );
        let measured = stabilize.analysis.as_ref().unwrap();
        assert_eq!(analysis.path()[&61], measured.path()[&51]);
        assert_eq!(
            (analysis.method(), analysis.size()),
            (Method::Similarity, [640, 360])
        );
        let unmeasured = |stabilize: &Stabilize| Stabilize {
            analysis: None,
            ..stabilize.clone()
        };
        assert_eq!(unmeasured(&later), unmeasured(&stabilize));
        let halved = stabilize.with_key_frames(&|frame| frame / 2);
        let analysis = halved.analysis.as_ref().unwrap();
        assert_eq!(
            analysis.path().keys().copied().collect::<Vec<_>>(),
            [25, 26]
        );
        assert_eq!(analysis.path()[&25], measured.path()[&51]);
    }

    #[test]
    fn controls_and_analyses_out_of_their_ranges_are_refused() {
        type Case = (&'static str, fn(&mut Stabilize));
        let cases: [Case; 9] = [
            ("stabilize.cropping_ratio", |s| s.cropping_ratio = 1.5),
            ("stabilize.cropping_ratio", |s| s.cropping_ratio = f64::NAN),
            ("stabilize.smooth", |s| s.smooth = -0.1),
            ("stabilize.strength", |s| s.strength = -1.5),
            ("stabilize.strength", |s| s.strength = f64::INFINITY),
            ("stabilize.analysis", |s| {
                s.analysis = Some(Analysis::new(Method::Perspective, [0, 360], shaking(3)));
            }),
            ("stabilize.analysis", |s| {
                s.analysis = Some(Analysis::new(
                    Method::Perspective,
                    [640, 360],
                    BTreeMap::new(),
                ));
            }),
            ("stabilize.analysis", |s| {
                let mut path = shaking(3);
                path.insert(LARGEST + 1, numbers_of(&Motion::IDENTITY));
                s.analysis = Some(Analysis::new(Method::Perspective, [640, 360], path));
            }),
            ("stabilize.analysis", |s| {
                let mut path = shaking(3);
                path.insert(0, [0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0]);
                s.analysis = Some(Analysis::new(Method::Perspective, [640, 360], path));
            }),
        ];
        for (wanted, change) in cases {
            let mut stabilize = stabilizer(shaking(3));
            assert_eq!(stabilize.check(), Ok(()));
            change(&mut stabilize);
            assert_eq!(
                stabilize.check(),
                Err(OpError::OutOfRange { what: wanted }),
                "{wanted}"
            );
        }
        // The ends of the ranges are in them.
        let ends = Stabilize {
            cropping_ratio: 1.0,
            smooth: 0.0,
            strength: -1.0,
            ..Stabilize::default()
        };
        assert_eq!(ends.check(), Ok(()));
    }

    #[test]
    fn a_stabilizer_is_written_with_its_analysis_and_read_back_as_it_was() {
        let stabilize = stabilizer(shaking(3));
        let written = serde_json::to_value(&stabilize).unwrap();
        assert_eq!(written["method"], "similarity");
        assert_eq!(written["analysis"]["method"], "similarity");
        assert_eq!(written["analysis"]["size"], serde_json::json!([640, 360]));
        assert_eq!(
            written["analysis"]["path"]["50"],
            serde_json::to_value(numbers_of(&Motion::IDENTITY)).unwrap()
        );
        let read: Stabilize = serde_json::from_value(written).unwrap();
        assert_eq!(read, stabilize);
        // Without an analysis, the field is left out; and an unknown field is refused.
        let plain = serde_json::to_value(Stabilize::default()).unwrap();
        assert_eq!(
            plain,
            serde_json::json!({"method": "perspective", "camera_lock": false, "zoom": true,
                "cropping_ratio": 0.5, "smooth": 0.25, "strength": 1.0})
        );
        let mut unknown = plain.clone();
        unknown["shake"] = true.into();
        assert!(serde_json::from_value::<Stabilize>(unknown).is_err());
        let mut wrong = plain;
        wrong["method"] = "affine".into();
        assert!(serde_json::from_value::<Stabilize>(wrong).is_err());
    }
}
