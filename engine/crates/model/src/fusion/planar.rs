//! The Planar Tracker of a composition (Reference Manual 20, Chapter 83, pp. 1760–1766, and the
//! node, pp. 2749–2758): a region of the picture that is taken to be a plane, the homography by
//! which the plane moves from the reference frame to each frame (its path), and what the node
//! draws with it: the picture held still (Steady), or another picture laid on the plane (Corner
//! Pin). What it is in a composition is told with the composition (The Planar Tracker, in the
//! module above).
//!
//! Finding the plane in the pictures of a clip, and so writing its path, is not done here: the
//! path is a setting, and is set as one. The engine's track of the node fits the motion of the
//! features of the region by the cloud tracker of `columbina_track`.
//!
//! # Units
//!
//! As those of the Tracker: a place is a share of the width and of the height of the picture
//! from its lower left corner, `y` up, a pixel `p` of a picture reaching from `p` to `p + 1`. A
//! homography of the path acts on such places, `[x, y, 1]`, and its last number is 1
//! ([`Homography`]).

use std::collections::BTreeMap;

use columbina_track::{fitted, Components, Mat3, Model};
use serde::{Deserialize, Deserializer, Serialize};

use super::{bounds, Place};
use crate::grade::Bounds;
use crate::ops::{OpError, LARGEST};
use crate::stabilize::{Matrix, IDENTITY};

/// The fewest points of the region of a [`PlanarTracker`]: those of a triangle.
pub const LEAST_REGION_POINTS: usize = 3;

/// The most points of the region of a [`PlanarTracker`]. The manual gives none: this is ours.
pub const MOST_REGION_POINTS: usize = 64;

/// A homography of the path of a [`PlanarTracker`], as it is written: `[h11, h12, h13, h21, h22,
/// h23, h31, h32]`, the numbers of its matrix row by row but for the last, `h33`, which is 1.
/// It acts on places of the picture from its lower left corner, `y` up: the place `[x, y]` goes
/// to `[(h11 x + h12 y + h13) / w, (h21 x + h22 y + h23) / w]`, `w = h31 x + h32 y + 1`.
pub type Homography = [f64; 8];

/// The homography that moves nothing.
pub const STILL: Homography = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0];

/// The corners of the picture, from its lower left one round to its upper left one, as places.
const PICTURE: [[f64; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];

/// The least width and height, in shares of the picture, of the box that bounds the region for
/// the path to be read between its frames by it ([`PlanarTracker::homography`]).
const SMALLEST: f64 = 1e-6;

/// What a [`PlanarTracker`] does with its pictures (the Operation Mode, p. 2750). It is written
/// `"none"`, `"steady"` or `"corner_pin"`, and takes no keys. Fusion's Track is the engine's
/// track of the node (`track.start` with `planar`), which writes its path; its Stabilize is
/// Steady.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanarOperation {
    /// Nothing: the node is the picture that comes into its `input`. Every Planar Tracker has
    /// it at first.
    #[default]
    None,
    /// The picture that was tracked held still: each frame is drawn through the inverse of the
    /// motion of the plane there ([`PlanarTracker::motion_at`]), which brings the plane back to
    /// where it was at the reference frame.
    Steady,
    /// The foreground laid on the plane: its picture put so that its four corners are on
    /// `corners`, moved as the plane moves, over the picture that was tracked.
    CornerPin,
}

/// Which parts of the motion of the plane a track keeps (the Motion Type, p. 2752). It is
/// written `"translation"`, `"translation_rotation"`, `"translation_rotation_scale"` or
/// `"perspective"`, and takes no keys. It is read by the track, which writes the path: a path
/// is as it was tracked, and a change of it is seen once the region is tracked again. Fusion's
/// Affine is not kept.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanarMotion {
    /// A shift alone, of the middle of the region.
    Translation,
    /// A shift and a turn.
    TranslationRotation,
    /// A shift, a turn and a size.
    TranslationRotationScale,
    /// The whole homography, the plane seen in perspective. Every Planar Tracker has it at
    /// first.
    #[default]
    Perspective,
}

impl PlanarMotion {
    /// Whether the motion of the plane is fitted as a homography, from four features at least;
    /// with the others it is fitted as a shift, a turn and a size, of which the parts that they
    /// name are kept.
    #[must_use]
    pub fn perspective(self) -> bool {
        self == Self::Perspective
    }

    /// The parts of a motion that are kept, by the boxes by which the window tracker names them
    /// (`columbina_track::Components`).
    #[must_use]
    pub fn components(self) -> Components {
        let all = Components::ALL;
        match self {
            Self::Translation => Components {
                zoom: false,
                rotate: false,
                perspective: false,
                ..all
            },
            Self::TranslationRotation => Components {
                zoom: false,
                perspective: false,
                ..all
            },
            Self::TranslationRotationScale => Components {
                perspective: false,
                ..all
            },
            Self::Perspective => all,
        }
    }
}

/// Where the corners of the foreground of a [`PlanarTracker`] go with Corner Pin, at the
/// reference frame (the Corner Pin controls, p. 2754): places, as a [`Center`](super::Center)
/// counts them. They are numbers, and take no keys: the path moves them.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Corners {
    /// Where the upper left corner of the foreground goes.
    pub tl: Place,
    /// Where its upper right corner goes.
    pub tr: Place,
    /// Where its lower right corner goes.
    pub br: Place,
    /// Where its lower left corner goes.
    pub bl: Place,
}

impl Corners {
    /// The four, from the lower left one round to the upper left one: where the lower left, the
    /// lower right, the upper right and the upper left corner of the foreground go.
    #[must_use]
    pub fn places(&self) -> [[f64; 2]; 4] {
        [self.bl, self.br, self.tr, self.tl].map(|place| [place.x, place.y])
    }
}

/// The settings of a Planar Tracker (pp. 2749–2758): what it does, the motion that a track of it
/// keeps, its reference frame, its region, the corners of its Corner Pin and its path.
///
/// None of its numbers takes keys: its path is what moves it. The frames of the path, and
/// `reference`, are frames of the keys of the clip, and are moved with them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanarTracker {
    /// The Operation Mode (p. 2750).
    pub operation: PlanarOperation,
    /// The Motion Type (p. 2752), which a track reads.
    pub motion: PlanarMotion,
    /// The Reference Frame (p. 2751): the frame, of the keys of the clip, at which the region
    /// was drawn and the plane is where the region and the corners say, at which nothing moves
    /// ([`PlanarTracker::motion_at`]). The homography of the path there is what the others are
    /// counted from: a path that a track wrote from the reference frame has none there.
    pub reference: i64,
    /// The region, the polygon of the plane at the reference frame: from
    /// [`LEAST_REGION_POINTS`] to [`MOST_REGION_POINTS`] places. The list is set whole.
    pub region: Vec<Place>,
    /// Where the corners of the foreground go at the reference frame, with Corner Pin.
    pub corners: Corners,
    /// The path: for frames of the keys of the clip, the homography that takes the plane from
    /// where it is at the reference frame to where it is at that frame ([`Homography`]); the
    /// motion is counted from the homography that it has at the reference frame, which is the
    /// identity where it was tracked from there ([`PlanarTracker::motion_at`]). It is written as
    /// an object of the frames, `{"12": [h11, h12, h13, h21, h22, h23, h31, h32]}`; a frame
    /// written `null` is not there, so that a change of the settings that gives a frame `null`
    /// takes it away. A frame that it has not was not tracked ([`PlanarTracker::homography`]).
    #[serde(deserialize_with = "samples")]
    pub path: BTreeMap<i64, Homography>,
}

/// Read the frames of a path, whose keys JSON writes as strings ("12"), each with its
/// homography or `null`, which leaves the frame out. They are read as strings and then as
/// frames, as those of the analysis of a stabilizer are, whatever has read the settings before.
fn samples<'de, D>(deserializer: D) -> Result<BTreeMap<i64, Homography>, D::Error>
where
    D: Deserializer<'de>,
{
    let written = BTreeMap::<String, Option<Homography>>::deserialize(deserializer)?;
    let mut path = BTreeMap::new();
    for (key, sample) in written {
        let Ok(frame) = key.parse::<i64>() else {
            return Err(<D::Error as serde::de::Error>::custom(format!(
                "{key:?} is not a frame of a path"
            )));
        };
        if let Some(sample) = sample {
            path.insert(frame, sample);
        }
    }
    Ok(path)
}

impl Default for PlanarTracker {
    /// An Operation of None, the motion Perspective, frame 0 as the reference, no path, and a
    /// region of the middle half of the picture across and down, a box whose corners the
    /// corners are. The region is ours: Fusion has none until one is drawn.
    fn default() -> Self {
        let (low, high) = (0.25, 0.75);
        let place = |x, y| Place { x, y };
        Self {
            operation: PlanarOperation::None,
            motion: PlanarMotion::Perspective,
            reference: 0,
            region: vec![
                place(low, low),
                place(high, low),
                place(high, high),
                place(low, high),
            ],
            corners: Corners {
                tl: place(low, high),
                tr: place(high, high),
                br: place(high, low),
                bl: place(low, low),
            },
            path: BTreeMap::new(),
        }
    }
}

/// The matrix of `homography`, its last number 1.
#[must_use]
pub fn matrix_of(homography: &Homography) -> Mat3 {
    let [h11, h12, h13, h21, h22, h23, h31, h32] = *homography;
    Mat3([[h11, h12, h13], [h21, h22, h23], [h31, h32, 1.0]])
}

/// The homography of `matrix` as a path writes it, its last number made 1: [`matrix_of`] the
/// other way. None where that number is 0, or a number is not finite.
#[must_use]
pub fn homography_of(matrix: &Mat3) -> Option<Homography> {
    let corner = matrix.0[2][2];
    if !corner.is_finite() || corner.abs() < 1e-12 {
        return None;
    }
    let [row0, row1, row2] = matrix.scaled(1.0 / corner).0;
    let numbers = [
        row0[0], row0[1], row0[2], row1[0], row1[1], row1[2], row2[0], row2[1],
    ];
    numbers.iter().all(|n| n.is_finite()).then_some(numbers)
}

/// `h`, a homography of places from the lower left corner of the picture, `y` up, as a matrix of
/// shares of the picture from its top left corner, `y` down, as the stabilizer has its matrices
/// ([`crate::stabilize::Stabilize::transform_at`]): the same map, seen through the flip of `y`,
/// its corner made 1. None where it is not of finite numbers, or its corner is 0.
fn top_left(h: &Mat3) -> Option<Matrix> {
    let flip = Mat3([[1.0, 0.0, 0.0], [0.0, -1.0, 1.0], [0.0, 0.0, 1.0]]);
    let seen = flip.mul(h).mul(&flip);
    let corner = seen.0[2][2];
    if !corner.is_finite() || corner.abs() < 1e-12 {
        return None;
    }
    let out = seen.scaled(1.0 / corner).0;
    out.iter().flatten().all(|v| v.is_finite()).then_some(out)
}

/// Whether `value` is a number within `bounds`.
fn within(bounds: Bounds, value: f64) -> bool {
    value.is_finite() && (bounds.low..=bounds.high).contains(&value)
}

fn refused(what: &'static str) -> OpError {
    OpError::OutOfRange { what }
}

impl PlanarTracker {
    /// Whether its path has a frame: what it draws then changes with the frame.
    #[must_use]
    pub fn has_path(&self) -> bool {
        !self.path.is_empty()
    }

    /// The homography of the path from the reference frame to `frame` of its keys, which may
    /// lie between frames:
    ///
    /// - at a frame of the path, the homography written there, exactly;
    /// - between two frames of it, the homography that puts the corners of the box that bounds
    ///   the region (of the picture, where the box has no area) on the lines between where the
    ///   homographies of the two frames put them, as far along as the frame is between them;
    ///   where that cannot be made (a corner sent to infinity), that of the frame before;
    /// - before its first frame, that of the first, and after its last, that of the last;
    /// - the identity where the path has no frame, and at a frame that is no number.
    #[must_use]
    pub fn homography(&self, frame: f64) -> Mat3 {
        if frame.is_nan() {
            return Mat3::IDENTITY;
        }
        let (Some((&first, start)), Some((&last, end))) =
            (self.path.first_key_value(), self.path.last_key_value())
        else {
            return Mat3::IDENTITY;
        };
        #[allow(clippy::cast_precision_loss, reason = "frames are within 2^53")]
        let at = |key: i64| key as f64;
        if frame <= at(first) {
            return matrix_of(start);
        }
        if frame >= at(last) {
            return matrix_of(end);
        }
        // The frame is after the first of the path and before the last: one of the path is at
        // it or before it, and one after it.
        #[allow(
            clippy::cast_possible_truncation,
            reason = "a frame between two frames of the path"
        )]
        let whole = frame.floor() as i64;
        let Some((&before, from)) = self.path.range(..=whole).next_back() else {
            return matrix_of(start);
        };
        let Some((&after, to)) = self.path.range(whole.saturating_add(1)..).next() else {
            return matrix_of(end);
        };
        let share = (frame - at(before)) / (at(after) - at(before));
        if share <= 0.0 {
            return matrix_of(from);
        }
        self.between(from, to, share)
            .unwrap_or_else(|| matrix_of(from))
    }

    /// The motion of the plane from the reference frame to `frame` of its keys: the homography
    /// of the path there after the inverse of the one at the reference frame
    /// ([`PlanarTracker::homography`]), so that at the reference frame nothing moves, whatever
    /// the path has there. Where the two are one (at the reference frame, and wherever the path
    /// holds the same), and where the path has no frame, it is the identity, exactly. None where
    /// the homography at the reference frame cannot be undone.
    #[must_use]
    pub fn motion_at(&self, frame: f64) -> Option<Mat3> {
        #[allow(clippy::cast_precision_loss, reason = "frames are within 2^53")]
        let rest = self.homography(self.reference as f64);
        let there = self.homography(frame);
        if there == rest {
            return Some(Mat3::IDENTITY);
        }
        if rest == Mat3::IDENTITY {
            return Some(there);
        }
        Some(there.mul(&rest.inverse()?).normalized())
    }

    /// The homography that puts the corners of the box of the region `share` of the way from
    /// where `from` puts them to where `to` does (see [`PlanarTracker::homography`]).
    fn between(&self, from: &Homography, to: &Homography, share: f64) -> Option<Mat3> {
        let (earlier, later) = (matrix_of(from), matrix_of(to));
        let corners = self.bounding_box();
        let mut on = [[0.0; 2]; 4];
        for (place, corner) in on.iter_mut().zip(corners) {
            let (a, b) = (earlier.apply(corner), later.apply(corner));
            *place = [a[0] + (b[0] - a[0]) * share, a[1] + (b[1] - a[1]) * share];
        }
        if !on.iter().flatten().all(|v| v.is_finite()) {
            return None;
        }
        fitted(Model::Homography, &corners, &on)
    }

    /// The corners of the box that bounds the region, from its lower left one round to its
    /// upper left one; those of the picture where the box is less than [`SMALLEST`] across or
    /// down.
    fn bounding_box(&self) -> [[f64; 2]; 4] {
        let low = self.region.iter().fold([f64::INFINITY; 2], |least, place| {
            [least[0].min(place.x), least[1].min(place.y)]
        });
        let high = self
            .region
            .iter()
            .fold([f64::NEG_INFINITY; 2], |most, place| {
                [most[0].max(place.x), most[1].max(place.y)]
            });
        if high[0] - low[0] > SMALLEST && high[1] - low[1] > SMALLEST {
            [
                [low[0], low[1]],
                [high[0], low[1]],
                [high[0], high[1]],
                [low[0], high[1]],
            ]
        } else {
            PICTURE
        }
    }

    /// The homography that puts the corners of the picture of the foreground on `corners`: the
    /// Corner Pin at the reference frame. None where the corners make no quadrilateral.
    #[must_use]
    pub fn corner_pin(&self) -> Option<Mat3> {
        fitted(Model::Homography, &PICTURE, &self.corners.places())
    }

    /// What it draws at `frame` of its keys ([`PlanarTracked`]): with Steady, the picture that
    /// was tracked put through the inverse of the motion of the plane there
    /// ([`PlanarTracker::motion_at`]); with Corner Pin, the foreground put through the Corner Pin
    /// and then through that motion. Where there is no motion, where it cannot be undone, and
    /// where the corners make no quadrilateral, it is its `input`: its operation is given as
    /// None.
    #[must_use]
    pub fn at(&self, frame: f64) -> PlanarTracked {
        let still = PlanarTracked {
            operation: PlanarOperation::None,
            matrix: IDENTITY,
        };
        let Some(motion) = self.motion_at(frame) else {
            return still;
        };
        let moved = match self.operation {
            PlanarOperation::None => return still,
            PlanarOperation::Steady => {
                if motion == Mat3::IDENTITY {
                    return PlanarTracked {
                        operation: PlanarOperation::Steady,
                        matrix: IDENTITY,
                    };
                }
                motion.inverse()
            }
            PlanarOperation::CornerPin => self.corner_pin().map(|pin| motion.mul(&pin)),
        };
        moved
            .and_then(|moved| top_left(&moved))
            .map_or(still, |matrix| PlanarTracked {
                operation: self.operation,
                matrix,
            })
    }

    /// Check it: its region of [`LEAST_REGION_POINTS`] to [`MOST_REGION_POINTS`] places, each
    /// within its bounds, its corners within theirs, its path of finite numbers at frames within
    /// [`LARGEST`] of 0, and its reference within it too.
    pub(crate) fn check(&self) -> Result<(), OpError> {
        let placed = |place: &[f64; 2]| place.iter().all(|&at| within(bounds::PLANAR_PLACE, at));
        let count = self.region.len();
        let region: Vec<[f64; 2]> = self.region.iter().map(|at| [at.x, at.y]).collect();
        if !(LEAST_REGION_POINTS..=MOST_REGION_POINTS).contains(&count)
            || !region.iter().all(placed)
        {
            return Err(refused("planar_tracker.region"));
        }
        if !self.corners.places().iter().all(placed) {
            return Err(refused("planar_tracker.corners"));
        }
        let fits = self.path.iter().all(|(frame, numbers)| {
            frame.unsigned_abs() <= LARGEST.unsigned_abs() && numbers.iter().all(|n| n.is_finite())
        });
        if !fits {
            return Err(refused("planar_tracker.path"));
        }
        if self.reference.unsigned_abs() > LARGEST.unsigned_abs() {
            return Err(refused("planar_tracker.reference"));
        }
        Ok(())
    }

    /// It with each frame of its path, and its reference, at the frame `to` of its frame,
    /// within [`LARGEST`] of 0. Of the homographies that come to one frame the last is kept.
    pub(crate) fn with_key_frames(&self, to: &impl Fn(i64) -> i64) -> PlanarTracker {
        let at = |frame: i64| to(frame).clamp(-LARGEST, LARGEST);
        PlanarTracker {
            path: self
                .path
                .iter()
                .map(|(&frame, &homography)| (at(frame), homography))
                .collect(),
            reference: at(self.reference),
            ..self.clone()
        }
    }
}

/// What a [`PlanarTracker`] draws at one moment: its operation, and where the picture that is
/// moved is put.
///
/// The picture that is moved is that of `input` with Steady, which is held still, and that of
/// `foreground` with Corner Pin, which is laid on the plane over `input`. With an operation of
/// None the node is its `input`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlanarTracked {
    /// The operation: with None the node is its `input`, whatever the rest.
    pub operation: PlanarOperation,
    /// Where each point of the picture that is moved is put, in shares of the picture, `x` of
    /// its width and `y` of its height from its top left corner, `y` down: a 3×3 matrix acting
    /// on `[x, y, 1]`, row by row, its corner 1, as the stabilizer's are
    /// ([`super::FusedKind::Stabilize`]). The identity where nothing is moved.
    pub matrix: Matrix,
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::float_cmp,
        reason = "homographies that are written and read back, and shifts that floats hold whole"
    )]

    use serde_json::json;

    use super::*;

    /// What is set of a Planar Tracker for a case of a test.
    type Change = fn(&mut PlanarTracker);

    /// A shift of `x` across and `y` up, in shares, as a path writes it.
    fn shifted(x: f64, y: f64) -> Homography {
        [1.0, 0.0, x, 0.0, 1.0, y, 0.0, 0.0]
    }

    /// A homography of a little perspective, a turn and a shift.
    const TILTED: Homography = [1.02, -0.03, 0.05, 0.02, 0.97, -0.04, 0.08, -0.06];

    /// A Planar Tracker that does `operation`, with the path `samples`.
    fn planar(operation: PlanarOperation, samples: &[(i64, Homography)]) -> PlanarTracker {
        PlanarTracker {
            operation,
            path: samples.iter().copied().collect(),
            ..PlanarTracker::default()
        }
    }

    /// Whether two places are one, but for the rounding of a few floats.
    fn near(one: [f64; 2], other: [f64; 2]) -> bool {
        (one[0] - other[0]).abs() < 1e-9 && (one[1] - other[1]).abs() < 1e-9
    }

    /// Where `matrix`, of shares from the top left corner, puts `[x, y]`.
    fn put(matrix: &Matrix, [x, y]: [f64; 2]) -> [f64; 2] {
        Mat3(*matrix).apply([x, y])
    }

    #[test]
    fn a_planar_tracker_is_written_with_every_setting_and_read_back() {
        let first = PlanarTracker::default();
        let written = serde_json::to_value(&first).unwrap();
        assert_eq!(
            written,
            json!({
                "operation": "none", "motion": "perspective", "reference": 0,
                "region": [{"x": 0.25, "y": 0.25}, {"x": 0.75, "y": 0.25},
                           {"x": 0.75, "y": 0.75}, {"x": 0.25, "y": 0.75}],
                "corners": {"tl": {"x": 0.25, "y": 0.75}, "tr": {"x": 0.75, "y": 0.75},
                            "br": {"x": 0.75, "y": 0.25}, "bl": {"x": 0.25, "y": 0.25}},
                "path": {},
            })
        );
        assert_eq!(first.check(), Ok(()));
        assert!(!first.has_path());
        let mut whole = planar(
            PlanarOperation::CornerPin,
            &[(-2, STILL), (12, shifted(0.125, -0.5))],
        );
        whole.motion = PlanarMotion::TranslationRotation;
        whole.reference = -2;
        let written = serde_json::to_value(&whole).unwrap();
        assert_eq!(written["operation"], "corner_pin");
        assert_eq!(written["motion"], "translation_rotation");
        assert_eq!(
            serde_json::to_string(&written["path"]).unwrap(),
            r#"{"-2":[1.0,0.0,0.0,0.0,1.0,0.0,0.0,0.0],"12":[1.0,0.0,0.125,0.0,1.0,-0.5,0.0,0.0]}"#
        );
        assert_eq!(
            serde_json::from_value::<PlanarTracker>(written).unwrap(),
            whole
        );
        // A frame written null is not there.
        let mut nulled = serde_json::to_value(&whole).unwrap();
        nulled["path"]["12"] = json!(null);
        nulled["path"]["30"] = json!(null);
        let read: PlanarTracker = serde_json::from_value(nulled).unwrap();
        assert_eq!(read.path.keys().copied().collect::<Vec<_>>(), [-2]);
        // A frame that is no whole number, a homography of seven numbers or of nine, and a
        // setting that a Planar Tracker has not cannot be read.
        for wrong in [
            json!({"1.5": STILL}),
            json!({"first": STILL}),
            json!({"1": [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0]}),
            json!({"1": [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]}),
            json!([STILL]),
        ] {
            let mut written = serde_json::to_value(PlanarTracker::default()).unwrap();
            written["path"] = wrong.clone();
            assert!(
                serde_json::from_value::<PlanarTracker>(written).is_err(),
                "{wrong}"
            );
        }
        for (setting, value) in [
            ("operation", json!("track")),
            ("motion", json!("affine")),
            ("corners", json!([[0.0, 0.0]])),
            ("tracker", json!(0)),
        ] {
            let mut written = serde_json::to_value(PlanarTracker::default()).unwrap();
            written[setting] = value;
            assert!(
                serde_json::from_value::<PlanarTracker>(written).is_err(),
                "{setting}"
            );
        }
    }

    #[test]
    fn the_path_is_exact_at_its_frames_read_between_them_and_held_beyond_them() {
        let moving = planar(
            PlanarOperation::Steady,
            &[(10, STILL), (20, shifted(0.2, -0.1)), (22, TILTED)],
        );
        // At a frame of the path, the homography written there.
        assert_eq!(moving.homography(10.0), Mat3::IDENTITY);
        assert_eq!(moving.homography(22.0), matrix_of(&TILTED));
        assert_eq!(
            moving.homography(20.0).0,
            [[1.0, 0.0, 0.2], [0.0, 1.0, -0.1], [0.0, 0.0, 1.0]]
        );
        // Between two shifts, the shift on the line between them.
        let half = moving.homography(15.0);
        for corner in PICTURE {
            let to = half.apply(corner);
            assert!(near(to, [corner[0] + 0.1, corner[1] - 0.05]), "{to:?}");
        }
        // Between two others, the corners of the box of the region where they are between.
        let box_corners = moving.bounding_box();
        assert_eq!(box_corners[0], [0.25, 0.25]);
        assert_eq!(box_corners[2], [0.75, 0.75]);
        let (one, other) = (matrix_of(&shifted(0.2, -0.1)), matrix_of(&TILTED));
        let partway = moving.homography(20.5);
        for corner in box_corners {
            let (a, b) = (one.apply(corner), other.apply(corner));
            let wanted = [a[0] + (b[0] - a[0]) / 4.0, a[1] + (b[1] - a[1]) / 4.0];
            assert!(near(partway.apply(corner), wanted), "{corner:?}");
        }
        // Before the first frame and after the last.
        for frame in [9.0, -1000.0, f64::NEG_INFINITY] {
            assert_eq!(moving.homography(frame), Mat3::IDENTITY, "{frame}");
        }
        for frame in [22.5, 1e9, f64::INFINITY] {
            assert_eq!(moving.homography(frame), matrix_of(&TILTED), "{frame}");
        }
        // No path, and a frame that is no number: the identity.
        let still = planar(PlanarOperation::Steady, &[]);
        assert_eq!(still.homography(7.0), Mat3::IDENTITY);
        assert_eq!(moving.homography(f64::NAN), Mat3::IDENTITY);
        // A region of no area: the corners of the picture.
        let mut flat = moving.clone();
        for place in &mut flat.region {
            place.y = 0.5;
        }
        assert_eq!(flat.bounding_box(), PICTURE);
    }

    #[test]
    fn steady_puts_the_picture_through_the_inverse_of_the_homography_from_its_top_left() {
        let moving = planar(
            PlanarOperation::Steady,
            &[(10, STILL), (20, shifted(0.25, 0.125))],
        );
        // At the reference frame, and with no path, nothing moves.
        assert_eq!(
            moving.at(10.0),
            PlanarTracked {
                operation: PlanarOperation::Steady,
                matrix: IDENTITY
            }
        );
        assert_eq!(
            planar(PlanarOperation::Steady, &[]).at(3.0).matrix,
            IDENTITY
        );
        // A quarter of the width to the right and an eighth of the height up is taken back: a
        // quarter to the left, and an eighth down from the top left.
        let held = moving.at(20.0);
        assert_eq!(held.operation, PlanarOperation::Steady);
        assert_eq!(
            held.matrix,
            [[1.0, 0.0, -0.25], [0.0, 1.0, 0.125], [0.0, 0.0, 1.0]]
        );
        // Through a perspective, each place where the plane is at the frame goes back to where
        // it was at the reference frame.
        let tilted = planar(PlanarOperation::Steady, &[(0, STILL), (1, TILTED)]);
        let back = tilted.at(1.0).matrix;
        for place in [[0.3, 0.4], [0.7, 0.2], [0.5, 0.9]] {
            let there = matrix_of(&TILTED).apply(place);
            let from_top = [there[0], 1.0 - there[1]];
            let wanted = [place[0], 1.0 - place[1]];
            assert!(near(put(&back, from_top), wanted), "{place:?}");
        }
        // With an operation of None, nothing is drawn whatever the path.
        let idle = planar(PlanarOperation::None, &[(0, TILTED)]);
        assert_eq!(idle.at(0.0).operation, PlanarOperation::None);
        assert_eq!(idle.at(0.0).matrix, IDENTITY);
        // A homography that flattens the picture cannot be undone: the node is its input.
        let flat = planar(PlanarOperation::Steady, &[(0, STILL), (5, [0.0; 8])]);
        assert_eq!(flat.at(5.0).operation, PlanarOperation::None);
        // Nor one at the reference frame, from which the motion is counted.
        let mut flat_at_rest = flat.clone();
        flat_at_rest.reference = 5;
        assert_eq!(flat_at_rest.at(0.0).operation, PlanarOperation::None);
        // At the reference frame itself nothing moves, whatever the path has there.
        assert_eq!(flat_at_rest.at(5.0).matrix, IDENTITY);
    }

    #[test]
    fn the_motion_is_counted_from_the_homography_at_the_reference_frame() {
        let mut moving = planar(
            PlanarOperation::Steady,
            &[
                (0, STILL),
                (10, shifted(0.25, 0.0)),
                (20, shifted(0.5, 0.125)),
            ],
        );
        // From frame 0, where the path has no motion, the motion is the path.
        assert_eq!(
            moving.motion_at(20.0),
            Some(matrix_of(&shifted(0.5, 0.125)))
        );
        // From frame 10, the plane is at rest there: before it, it has moved left.
        moving.reference = 10;
        assert_eq!(moving.motion_at(10.0), Some(Mat3::IDENTITY));
        let back = moving.motion_at(0.0).unwrap();
        assert!(near(back.apply([0.5, 0.5]), [0.25, 0.5]));
        let on = moving.motion_at(20.0).unwrap();
        assert!(near(on.apply([0.5, 0.5]), [0.75, 0.625]));
        // Held still, the picture of frame 0 is put back a quarter of the width to the right.
        let held = moving.at(0.0).matrix;
        assert!(near(put(&held, [0.25, 0.5]), [0.5, 0.5]), "{held:?}");
        // A reference beyond the path is where the path is held: before its first frame, at
        // the first.
        moving.reference = -40;
        assert_eq!(moving.motion_at(0.0), Some(Mat3::IDENTITY));
        assert_eq!(moving.motion_at(-100.0), Some(Mat3::IDENTITY));
        // With no path there is no motion at all.
        let still = planar(PlanarOperation::Steady, &[]);
        assert_eq!(still.motion_at(12.0), Some(Mat3::IDENTITY));
    }

    #[test]
    fn a_corner_pin_puts_the_corners_of_the_foreground_on_the_corners_moved_by_the_path() {
        let mut pinned = planar(
            PlanarOperation::CornerPin,
            &[(0, STILL), (10, shifted(0.125, 0.25))],
        );
        pinned.corners = Corners {
            tl: Place { x: 0.2, y: 0.8 },
            tr: Place { x: 0.7, y: 0.75 },
            br: Place { x: 0.8, y: 0.3 },
            bl: Place { x: 0.1, y: 0.2 },
        };
        let pin = pinned.corner_pin().unwrap();
        for (corner, place) in PICTURE.iter().zip(pinned.corners.places()) {
            assert!(near(pin.apply(*corner), place), "{corner:?}");
        }
        // From the top left: the upper left corner of the picture is at its top left.
        for frame in [0.0, 10.0] {
            let drawn = pinned.at(frame);
            assert_eq!(drawn.operation, PlanarOperation::CornerPin);
            let shift = if frame > 0.0 {
                [0.125, 0.25]
            } else {
                [0.0, 0.0]
            };
            for (corner, place) in PICTURE.iter().zip(pinned.corners.places()) {
                let from_top = [corner[0], 1.0 - corner[1]];
                let wanted = [place[0] + shift[0], 1.0 - (place[1] + shift[1])];
                assert!(
                    near(put(&drawn.matrix, from_top), wanted),
                    "{frame}: {corner:?}"
                );
            }
        }
        // Corners that make no quadrilateral pin nothing: the node is its input.
        let one = Place { x: 0.5, y: 0.5 };
        pinned.corners = Corners {
            tl: one,
            tr: one,
            br: one,
            bl: one,
        };
        assert_eq!(pinned.corner_pin(), None);
        assert_eq!(pinned.at(10.0).operation, PlanarOperation::None);
    }

    #[test]
    fn a_homography_is_a_matrix_whose_corner_is_1_and_back() {
        assert_eq!(matrix_of(&STILL), Mat3::IDENTITY);
        assert_eq!(homography_of(&matrix_of(&TILTED)), Some(TILTED));
        assert_eq!(homography_of(&matrix_of(&TILTED).scaled(2.0)), Some(TILTED));
        assert_eq!(
            homography_of(&Mat3([[1.0; 3], [1.0; 3], [1.0, 1.0, 0.0]])),
            None
        );
        assert_eq!(top_left(&Mat3::IDENTITY), Some(IDENTITY));
        assert_eq!(top_left(&Mat3::IDENTITY.scaled(2.0)), Some(IDENTITY));
    }

    #[test]
    fn a_corner_below_1e_12_is_taken_for_0_and_one_of_1e_12_is_divided_by() {
        let tiny = Mat3::IDENTITY.scaled(1e-13);
        assert_eq!(homography_of(&tiny), None);
        assert_eq!(top_left(&tiny), None);
        let least = Mat3::IDENTITY.scaled(1e-12);
        let read = homography_of(&least).unwrap();
        assert!(
            read.iter().zip(STILL).all(|(n, s)| (n - s).abs() < 1e-9),
            "{read:?}"
        );
        let seen = top_left(&least).unwrap();
        assert!(
            seen.iter()
                .flatten()
                .zip(IDENTITY.iter().flatten())
                .all(|(n, s)| (n - s).abs() < 1e-9),
            "{seen:?}"
        );
    }

    #[test]
    fn a_region_no_more_than_a_millionth_across_or_down_is_bounded_by_the_picture() {
        let spanning = |[left, right]: [f64; 2], [bottom, top]: [f64; 2]| PlanarTracker {
            region: vec![
                Place { x: left, y: bottom },
                Place {
                    x: right,
                    y: bottom,
                },
                Place { x: right, y: top },
                Place { x: left, y: top },
            ],
            ..PlanarTracker::default()
        };
        // A line up and down, one across, and a box exactly as small as the least.
        assert_eq!(spanning([0.5, 0.5], [0.25, 0.75]).bounding_box(), PICTURE);
        assert_eq!(
            spanning([0.0, SMALLEST], [0.25, 0.75]).bounding_box(),
            PICTURE
        );
        assert_eq!(
            spanning([0.25, 0.75], [0.0, SMALLEST]).bounding_box(),
            PICTURE
        );
        // Any more across and down, and it is the box.
        let wider = 2.0 * SMALLEST;
        assert_eq!(
            spanning([0.0, wider], [0.0, wider]).bounding_box(),
            [[0.0, 0.0], [wider, 0.0], [wider, wider], [0.0, wider]]
        );
    }

    #[test]
    fn the_motions_name_the_parts_of_a_motion_that_are_kept() {
        let kept = |motion: PlanarMotion| {
            let parts = motion.components();
            (
                motion.perspective(),
                [
                    parts.pan,
                    parts.tilt,
                    parts.zoom,
                    parts.rotate,
                    parts.perspective,
                ],
            )
        };
        assert_eq!(
            kept(PlanarMotion::Translation),
            (false, [true, true, false, false, false])
        );
        assert_eq!(
            kept(PlanarMotion::TranslationRotation),
            (false, [true, true, false, true, false])
        );
        assert_eq!(
            kept(PlanarMotion::TranslationRotationScale),
            (false, [true, true, true, true, false])
        );
        assert_eq!(kept(PlanarMotion::Perspective), (true, [true; 5]));
    }

    #[test]
    fn the_numbers_of_a_planar_tracker_are_within_their_bounds() {
        let whole = || planar(PlanarOperation::Steady, &[(0, STILL)]);
        let refusals: [(&str, Change); 10] = [
            ("planar_tracker.region", |p| p.region.truncate(2)),
            ("planar_tracker.region", |p| {
                p.region = vec![Place { x: 0.5, y: 0.5 }; MOST_REGION_POINTS + 1];
            }),
            ("planar_tracker.region", |p| p.region[1].x = f64::NAN),
            ("planar_tracker.region", |p| p.region[2].y = 17.0),
            ("planar_tracker.corners", |p| p.corners.tl.x = f64::INFINITY),
            ("planar_tracker.corners", |p| p.corners.br.y = -15.6),
            ("planar_tracker.path", |p| {
                p.path
                    .insert(3, [f64::NAN, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0]);
            }),
            ("planar_tracker.path", |p| {
                p.path.insert(LARGEST + 1, STILL);
            }),
            ("planar_tracker.path", |p| {
                p.path.insert(-LARGEST - 1, STILL);
            }),
            ("planar_tracker.reference", |p| p.reference = LARGEST + 1),
        ];
        for (what, change) in refusals {
            let mut changed = whole();
            change(&mut changed);
            assert_eq!(changed.check(), Err(OpError::OutOfRange { what }), "{what}");
        }
        assert_eq!(whole().check(), Ok(()));
        // The ends of each range are within it.
        let mut ends = whole();
        ends.region = vec![Place { x: -15.5, y: 16.5 }; LEAST_REGION_POINTS];
        ends.corners.tl = Place { x: 16.5, y: -15.5 };
        ends.path = [(-LARGEST, STILL), (LARGEST, [1e12; 8])]
            .into_iter()
            .collect();
        ends.reference = -LARGEST;
        assert_eq!(ends.check(), Ok(()));
        ends.region = vec![Place { x: 0.5, y: 0.5 }; MOST_REGION_POINTS];
        assert_eq!(ends.check(), Ok(()));
    }

    #[test]
    fn the_frames_of_the_path_and_the_reference_are_moved_with_the_keys() {
        let mut moving = planar(
            PlanarOperation::Steady,
            &[
                (10, STILL),
                (11, shifted(0.1, 0.0)),
                (12, shifted(0.2, 0.0)),
            ],
        );
        moving.reference = 11;
        let frames = |p: &PlanarTracker| p.path.keys().copied().collect::<Vec<_>>();
        let later = moving.with_key_frames(&|frame| frame + 100);
        assert_eq!(frames(&later), [110, 111, 112]);
        assert_eq!(later.reference, 111);
        assert_eq!(
            (later.region.clone(), later.corners),
            (moving.region.clone(), moving.corners)
        );
        // It draws later what it drew.
        assert_eq!(later.at(111.5), moving.at(11.5));
        assert_eq!(later.check(), Ok(()));
        // Homographies that come to one frame are one, the last; and a frame goes no further
        // than the largest.
        let halved = moving.with_key_frames(&|frame| frame / 2);
        assert_eq!(frames(&halved), [5, 6]);
        assert_eq!(halved.path[&5], shifted(0.1, 0.0));
        assert_eq!(halved.reference, 5);
        let far = moving.with_key_frames(&|frame| frame.saturating_mul(i64::MAX / 11));
        assert_eq!(frames(&far), [LARGEST]);
        assert_eq!(far.reference, LARGEST);
        // Played backward, each homography stays on its picture.
        let backward = moving.with_key_frames(&|frame| 30 - frame);
        assert_eq!(frames(&backward), [18, 19, 20]);
        assert_eq!(backward.path[&18], shifted(0.2, 0.0));
        assert_eq!(backward.reference, 19);
    }
}
