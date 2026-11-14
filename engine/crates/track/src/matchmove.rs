//! The mathematics of the Operation tab of Fusion's Tracker node (Reference Manual of Resolve
//! 20, pp. 1726, 1741–1748 and 2777–2784): from the paths of its tracked patterns, the motion of
//! the picture since a reference frame, the transform that takes that motion out (Stabilize) or
//! gives it to another picture (Match Move), and the values that the node publishes for other
//! parameters to be connected to.
//!
//! # What it is given, and what it gives
//!
//! Each pattern of the node is a [`Tracker`]: where its centre was found in each frame (not in
//! every frame: a pattern may be hidden or leave the frame, p. 1740), and its offset (p. 1741).
//! [`solve`] gives, for each frame, the motion of the patterns since the reference frame as a
//! shift, a turn and a size about a pivot ([`Solution`]). From a solution come the matrix that a
//! picture is drawn through ([`Solution::steady`], [`Solution::unsteady`],
//! [`Solution::transform`]) and the published values in Fusion's units
//! ([`Solution::published`]); each pattern publishes its own ([`offset_position`],
//! [`tracker_positions`]). Following the patterns in the frames, and drawing a picture through a
//! matrix, are not done here.
//!
//! # The controls
//!
//! - **Operation** ([`Operation`], p. 2778): None or Match Move. Corner Positioning and
//!   Perspective Positioning are yet to be made.
//! - **Merge** (pp. 1742, 1744, 2779), here only as which way the transform goes ([`Way`]): BG
//!   Only stabilizes the picture that was tracked, which is drawn through the inverse of the
//!   motion; FG Only, FG Over BG and BG Over FG give the motion to the foreground.
//! - **Position, Rotation, Scaling** (pp. 1743, 2782): the parts of the motion that are
//!   corrected. A part that is off is left as no motion ([`Settings::components`]).
//! - **Pivot Type** ([`Pivot`], pp. 1743, 2783): the point that the turn and the size are about.
//! - **Reference** (pp. 1743, 2783): the frame that the picture is brought back to, here a frame
//!   chosen by its place in the paths (Select Time; Start and End are the first and the last
//!   frame of the paths, which the caller finds). Start and End, which smooths the motion in
//!   place of removing it, is yet to be made.
//!
//! # How many patterns
//!
//! "Stabilizing and match move require a minimum of one tracking pattern to determine position,
//! and two or more to determine scaling and rotation" (p. 2778; likewise pp. 1726, 1742, 1746).
//! So a frame with one pattern gives a shift alone, its turn none and its size 1; two or more
//! give the similarity that fits them best by least squares. This is not the rule of the point
//! tracker of the Color page ([`crate::Model::for_points`]), whose manual wants three points for
//! a zoom.
//!
//! # Coordinates and units
//!
//! The paths, the offsets, the pivot and the matrices are the tracker's: pixels of the frame,
//! `[x, y]` with y down, the frame reaching from 0 to its width and from 0 to its height (as
//! [`crate::stabilize`] has it, whose centre is half the size). A turn of a [`Motion`] is in
//! radians from x towards y, which is clockwise on the screen.
//!
//! The published values are Fusion's:
//!
//! - A position is a share of the frame, x of its width and y of its height, from the bottom
//!   left: "the bottom-left corner is always 0/0, the top-right corner is always 1/1" (p. 1549),
//!   and 0.5, 0.5 is the centre (p. 2810). So y points up ([`to_fusion`], [`from_fusion`]).
//! - An angle is in degrees and counter-clockwise: "increasing the Angle rotates the image in a
//!   counterclockwise direction" (p. 2811). It is the opposite of [`Motion::rotate`] in degrees.
//! - A size is a factor, 1 for the size as it was (p. 1746).

use std::f64::consts::TAU;

use crate::fit::{least_squares, Model};
use crate::linalg::{Mat3, Point};
use crate::motion::{Components, Motion};

/// The least size that a fitted motion may have. A frame whose patterns have come together into
/// (nearly) one place would give a size of nothing, which cannot be undone: it has no solution.
/// Ours: the manual says nothing of it.
pub const MIN_SIZE: f64 = 1e-6;

/// What the node does with the picture (the Operation menu, p. 2778).
///
/// Corner Positioning and Perspective Positioning (pp. 1745, 2778), which map four patterns to
/// the corners of a picture by a homography, are yet to be made: they are to be added here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Operation {
    /// Nothing "beyond merely locating and tracking the chosen pattern": the picture is left as
    /// it is. The values are published all the same (pp. 1742, 2783).
    None,
    /// The motion of the patterns is taken out of the picture, or given to another.
    MatchMove,
}

/// Which way the transform goes: what the Merge menu comes to for the mathematics (pp. 1742,
/// 1744, 2779).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Way {
    /// The picture that was tracked is held still (BG Only): it is drawn through the inverse of
    /// the motion, which brings each frame back to the reference frame.
    Steady,
    /// Another picture follows the one that was tracked (FG Only, FG Over BG, BG Over FG): it is
    /// drawn through the motion.
    Unsteady,
}

/// The point that the turn and the size are about (the Pivot Type menu, pp. 1743, 2783), where
/// it is at the reference frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pivot {
    /// "Averages the location based on the tracking points": the mean of the centres of the
    /// patterns that are tracked at the reference frame. All of them, whichever are tracked at
    /// the frame that is solved, so that a pattern lost for a while does not make the pivot
    /// jump (ours). The offsets have no say.
    TrackerAverage,
    /// The centre of one pattern at the reference frame, by its place among the trackers. Where
    /// that pattern is not tracked at the reference frame, or there is none at that place, the
    /// pivot is the tracker average (ours).
    SelectedTracker(usize),
    /// A point given, in pixels. Where it is not finite, the pivot is the tracker average (ours).
    Manual(Point),
}

/// The controls of the Operation tab (see the module).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// What the node does with the picture.
    pub operation: Operation,
    /// Which way the transform goes.
    pub way: Way,
    /// Whether the shift is corrected (the Position box).
    pub position: bool,
    /// Whether the turn is corrected (the Rotation box).
    pub rotation: bool,
    /// Whether the size is corrected (the Scaling box).
    pub scaling: bool,
    /// The point that the turn and the size are about.
    pub pivot: Pivot,
    /// The reference frame, by its place in the paths: the frame that stabilizing brings the
    /// picture back to, where nothing moves.
    pub reference: usize,
}

impl Default for Settings {
    /// The controls as the manual has them at first, where it says: an Operation of None
    /// ("the default mode", p. 2778), a pivot at the tracker average ("usually the average of
    /// the combined pattern centers", p. 1743) and the first frame as the reference ("by default
    /// at frame 1", p. 1745). The three boxes on and BG Only are assumptions, to be checked
    /// against `DaVinci`.
    fn default() -> Self {
        Self {
            operation: Operation::None,
            way: Way::Steady,
            position: true,
            rotation: true,
            scaling: true,
            pivot: Pivot::TrackerAverage,
            reference: 0,
        }
    }
}

impl Settings {
    /// The components of a motion that the boxes correct: Position is the pan and the tilt,
    /// Rotation the turn and Scaling the zoom. A similarity has no perspective.
    #[must_use]
    pub fn components(&self) -> Components {
        Components {
            pan: self.position,
            tilt: self.position,
            zoom: self.scaling,
            rotate: self.rotation,
            perspective: false,
        }
    }
}

/// One pattern of the node.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tracker<'a> {
    /// Where the centre of the pattern was found in each frame, in pixels; `None` where it was
    /// not tracked. The paths of the trackers of a node start at the same frame; a path that is
    /// shorter than another is not tracked in the frames that it lacks.
    pub path: &'a [Option<Point>],
    /// The X and Y Offset (pp. 1741, 2777), in pixels, right and down: how far from the centre
    /// of the pattern the thing lies that the path is wanted for. It shifts the Offset Position
    /// alone ([`offset_position`]); the motion is fitted to the centres of the patterns.
    pub offset: Point,
}

impl<'a> Tracker<'a> {
    /// A pattern with no offset.
    #[must_use]
    pub fn new(path: &'a [Option<Point>]) -> Self {
        Self {
            path,
            offset: [0.0, 0.0],
        }
    }

    /// Where the centre of the pattern is at `frame`, or `None` where it is not tracked: where
    /// the path has no point, or one that is not finite, or ends before the frame.
    #[must_use]
    pub fn at(&self, frame: usize) -> Option<Point> {
        self.path.get(frame).copied().flatten().and_then(finite)
    }
}

/// A point whose two numbers are finite.
fn finite(point: Point) -> Option<Point> {
    (point[0].is_finite() && point[1].is_finite()).then_some(point)
}

/// The width and the height of a frame, where both are finite and more than nothing.
fn extent(size: (f64, f64)) -> Option<(f64, f64)> {
    let (width, height) = size;
    (width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0)
        .then_some((width, height))
}

/// A point of a frame of `size`, given in pixels with y down, as Fusion has it: shares of the
/// width and of the height from the bottom left, y up (pp. 1549, 2810):
///
/// ```text
/// X = x / width,   Y = 1 − y / height
/// ```
///
/// `None` where the size is not finite or not more than nothing.
#[must_use]
pub fn to_fusion(point: Point, size: (f64, f64)) -> Option<Point> {
    let (width, height) = extent(size)?;
    Some([point[0] / width, 1.0 - point[1] / height])
}

/// A point as Fusion has it, in pixels of a frame of `size` with y down: the inverse of
/// [`to_fusion`].
#[must_use]
pub fn from_fusion(point: Point, size: (f64, f64)) -> Option<Point> {
    let (width, height) = extent(size)?;
    Some([point[0] * width, (1.0 - point[1]) * height])
}

/// How many frames the paths of `trackers` cover: the length of the longest.
#[must_use]
pub fn frames(trackers: &[Tracker<'_>]) -> usize {
    trackers
        .iter()
        .map(|tracker| tracker.path.len())
        .max()
        .unwrap_or(0)
}

/// The mean of the points that there are.
fn average(points: &[Option<Point>]) -> Option<Point> {
    let (mut sum, mut count) = ([0.0_f64, 0.0_f64], 0_u32);
    for point in points.iter().flatten() {
        sum[0] += point[0];
        sum[1] += point[1];
        count += 1;
    }
    (count > 0).then(|| {
        let count = f64::from(count);
        [sum[0] / count, sum[1] / count]
    })
}

/// The motion of the patterns from the reference frame to one frame, about a pivot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Solution {
    /// The motion since the reference frame, about [`Solution::pivot`], with the parts that the
    /// boxes leave out put back to no motion: `pan` and `tilt` in pixels, right and down; `zoom`
    /// the size; `rotate` in radians, clockwise on the screen, counted on from frame to frame so
    /// that a thing that goes round does not jump back by a whole turn. The rest of a
    /// [`Motion`] is as for no motion.
    pub motion: Motion,
    /// The pivot where it was at the reference frame, in pixels.
    pub pivot: Point,
    /// How many patterns the motion was fitted to: those tracked both at this frame and at the
    /// reference frame.
    pub trackers: usize,
}

impl Solution {
    /// The motion as a matrix in pixels: where a point of the reference frame is at this frame,
    /// by the parts that are on. With the pivot `c`, the shift `t`, the size `s` and the turn `R`,
    ///
    /// ```text
    /// U(p) = c + t + s · R · (p − c)
    /// ```
    ///
    /// A picture drawn through it follows the patterns (Match Move with a foreground, p. 1744).
    #[must_use]
    pub fn unsteady(&self) -> Mat3 {
        self.motion.matrix(self.pivot)
    }

    /// The inverse of [`Solution::unsteady`], in pixels: where a point of this frame was at the
    /// reference frame,
    ///
    /// ```text
    /// S(q) = (c + t) − t + (1 / s) · R⁻¹ · (q − (c + t))
    /// ```
    ///
    /// that is, the opposite shift, turn and size about the pivot where it is at this frame. A
    /// frame drawn through it is "transformed to return the image to a reference position"
    /// (Match Move with BG Only, p. 1742). It is written out and not found by inverting the
    /// matrix, so it is there for every solution that [`solve`] gives.
    #[must_use]
    pub fn steady(&self) -> Mat3 {
        let motion = &self.motion;
        Motion {
            pan: -motion.pan,
            tilt: -motion.tilt,
            zoom: 1.0 / motion.zoom,
            rotate: -motion.rotate,
            ..Motion::IDENTITY
        }
        .matrix([self.pivot[0] + motion.pan, self.pivot[1] + motion.tilt])
    }

    /// The matrix that the node draws its picture through, in pixels: none for an Operation of
    /// None, and for Match Move the one that `way` asks for.
    #[must_use]
    pub fn transform(&self, operation: Operation, way: Way) -> Mat3 {
        match (operation, way) {
            (Operation::None, _) => Mat3::IDENTITY,
            (Operation::MatchMove, Way::Steady) => self.steady(),
            (Operation::MatchMove, Way::Unsteady) => self.unsteady(),
        }
    }

    /// The values that the node publishes at this frame (pp. 1745–1748, 2784), in Fusion's
    /// units, for a frame of `size` (its width and height in pixels). `None` where the size is
    /// not finite or not more than nothing.
    #[must_use]
    pub fn published(&self, size: (f64, f64)) -> Option<Published> {
        let (width, height) = extent(size)?;
        let motion = &self.motion;
        // The shift as a share of the frame, right and up.
        let shift = [motion.pan / width, -motion.tilt / height];
        // Counter-clockwise, where the turn of a motion is clockwise.
        let degrees = -motion.rotate.to_degrees();
        let moved = [self.pivot[0] + motion.pan, self.pivot[1] + motion.tilt];
        Some(Published {
            steady_position: [0.5 - shift[0], 0.5 - shift[1]],
            steady_angle: -degrees,
            steady_size: 1.0 / motion.zoom,
            steady_axis: to_fusion(moved, size)?,
            unsteady_position: [0.5 + shift[0], 0.5 + shift[1]],
            unsteady_angle: degrees,
            unsteady_size: motion.zoom,
            unsteady_axis: to_fusion(self.pivot, size)?,
        })
    }
}

/// The values that the node publishes at a frame, "calculated using all the patterns in that
/// tracker, as configured by the Match Move Settings" (p. 1746), in Fusion's units (see the
/// module): what the Center, the Pivot, the Angle and the Size of a Transform node are connected
/// to.
///
/// A Transform node is taken to turn and size the picture about its Pivot and then to shift it
/// by its Center less 0.5, 0.5 (p. 2810 says no more than that the Pivot "positions the axis of
/// rotation and scaling"). Read so, a Transform with its four controls connected to the four
/// steady values is [`Solution::steady`], and to the four unsteady values
/// [`Solution::unsteady`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Published {
    /// Steady Position (p. 1745): 0.5, 0.5 less the shift. "The connection inverts the output of
    /// the tracked pattern's motion", and it is "placed at 0.5/0.5 (the center of the screen)"
    /// at the reference frame; a pattern that has gone a tenth of the width to the right gives
    /// 0.4, 0.5 (p. 1748).
    pub steady_position: Point,
    /// Steady Angle (pp. 1745–1746), in degrees, counter-clockwise: the opposite of the turn,
    /// 0 at the reference frame. "If the angle at frame 10 is 15 degrees, the result of the
    /// Steady Angle will be -15 degrees."
    pub steady_angle: f64,
    /// Steady Size (p. 1746): "the inverse of the tracked pattern's scale", 1 at the reference
    /// frame. Here one over the size, which is what holds the picture still. The manual's own
    /// example, 0.85 for a size of 1.15, is 2 less the size: the two part by the square of the
    /// change, and only one over the size undoes the motion.
    pub steady_size: f64,
    /// Steady Axis (named on p. 2784, not described): here the pivot where it is at this frame,
    /// which the steady angle and size are about.
    pub steady_axis: Point,
    /// Unsteady Position (p. 1746): 0.5, 0.5 and the shift, "an offset in the same direction as
    /// the original motion", which gives the motion back to a picture that was held still.
    pub unsteady_position: Point,
    /// Unsteady Angle (named on p. 2784): the turn in degrees, counter-clockwise.
    pub unsteady_angle: f64,
    /// Unsteady Size (named on p. 2784): the size.
    pub unsteady_size: f64,
    /// The pivot where it was at the reference frame, which the unsteady angle and size are
    /// about. Ours: Fusion publishes no such value, and a Transform that follows the patterns
    /// needs its Pivot there.
    pub unsteady_axis: Point,
}

/// The motion of the patterns from where they were at the reference frame (`start`) to `frame`,
/// about `pivot`, with every part, its turn within half a turn of none; and how many patterns it
/// was fitted to.
fn fitted(
    trackers: &[Tracker<'_>],
    start: &[Option<Point>],
    frame: usize,
    pivot: Point,
) -> Option<(Motion, usize)> {
    let (mut src, mut dst) = (Vec::new(), Vec::new());
    for (tracker, from) in trackers.iter().zip(start) {
        if let (Some(from), Some(to)) = (*from, tracker.at(frame)) {
            src.push(from);
            dst.push(to);
        }
    }
    let motion = match src.len() {
        0 => return None,
        // One pattern cannot tell a turn or a size (p. 2778).
        1 => Motion {
            pan: dst[0][0] - src[0][0],
            tilt: dst[0][1] - src[0][1],
            ..Motion::IDENTITY
        },
        count => {
            // x' = a·x − b·y + tx, y' = b·x + a·y + ty.
            let fit = least_squares(Model::Similarity, &src, &dst, &vec![1.0; count])?.0;
            let (a, b) = (fit[0][0], fit[1][0]);
            let zoom = a.hypot(b);
            if !(zoom.is_finite() && zoom >= MIN_SIZE) {
                return None;
            }
            Motion {
                // Where the pivot goes, less the pivot.
                pan: (a - 1.0) * pivot[0] - b * pivot[1] + fit[0][2],
                tilt: b * pivot[0] + (a - 1.0) * pivot[1] + fit[1][2],
                zoom,
                rotate: b.atan2(a),
                ..Motion::IDENTITY
            }
        }
    };
    (motion.pan.is_finite() && motion.tilt.is_finite()).then_some((motion, src.len()))
}

/// For each frame of the paths ([`frames`]), the motion of the patterns since the reference
/// frame, by the controls (see the module); `None` for a frame that has no solution.
///
/// At a frame, the patterns that count are those tracked both there and at the reference frame.
/// With `pᵢ` where they were at the reference frame and `qᵢ` where they are:
///
/// - None of them: no solution.
/// - One: the shift `q − p`, no turn and a size of 1 (p. 2778).
/// - Two or more: the similarity `q ≈ M(p)` that fits them best by least squares, in closed form
///   (the fit of the crate's `Model::Similarity`; it is Umeyama's without the mirror, which a
///   similarity written with two numbers cannot be). With `(x, y) = pᵢ − p̄` and
///   `(u, v) = qᵢ − q̄`, the bars being the means,
///
///   ```text
///   a = Σ(x·u + y·v) / Σ(x² + y²),   b = Σ(x·v − y·u) / Σ(x² + y²)
///   size = √(a² + b²),   turn = atan2(b, a),   M(p̄) = q̄
///   ```
///
///   and the shift is where the pivot goes, less the pivot: `M(c) − c`.
///
/// The parts that the boxes leave out are then put back to no motion ([`Motion::masked`]), as
/// [`crate::stabilize`] does with a method's. The turn and the size are fitted even where their
/// boxes are off, so the shift is that of the pivot and not of a pattern far from it.
///
/// At the reference frame the motion is none. The turn is counted on from the reference frame
/// outwards, each frame within half a turn of the last one that was fitted to two patterns or
/// more.
///
/// What is refused, each with no solution and no error:
///
/// - A frame at which no pattern is tracked that is also tracked at the reference frame. So a
///   reference frame at which nothing is tracked, or one beyond the paths, gives no solution at
///   any frame.
/// - A point that is not finite counts as not tracked.
/// - Two or more patterns that were all at one place at the reference frame (no turn or size
///   can be told from them), at every frame where they are all that is tracked, the reference
///   frame among them. There is no falling back to a shift.
/// - Patterns that have come together at a frame, to a size below [`MIN_SIZE`].
#[must_use]
pub fn solve(trackers: &[Tracker<'_>], settings: &Settings) -> Vec<Option<Solution>> {
    let count = frames(trackers);
    let mut out = vec![None; count];
    let reference = settings.reference;
    let start: Vec<Option<Point>> = trackers
        .iter()
        .map(|tracker| tracker.at(reference))
        .collect();
    // With nothing tracked at the reference frame (or the frame beyond the paths) there is
    // nothing to measure a motion from.
    let Some(mean) = average(&start) else {
        return out;
    };
    let pivot = match settings.pivot {
        Pivot::TrackerAverage => None,
        Pivot::SelectedTracker(index) => start.get(index).copied().flatten(),
        Pivot::Manual(at) => finite(at),
    }
    .unwrap_or(mean);
    let enabled = settings.components();
    let mut turn = 0.0_f64;
    // Forwards from the reference frame, then backwards from it.
    for frame in (reference..count).chain((0..reference).rev()) {
        if frame + 1 == reference {
            turn = 0.0;
        }
        let Some((mut motion, used)) = fitted(trackers, &start, frame, pivot) else {
            continue;
        };
        if frame == reference {
            motion = Motion::IDENTITY;
        }
        if used > 1 {
            // The angle of a fit is within half a turn of none: it is brought to within half a
            // turn of the one before.
            let turns = ((turn - motion.rotate) / TAU).round();
            motion.rotate += turns * TAU;
            turn = motion.rotate;
        }
        out[frame] = Some(Solution {
            motion: motion.masked(enabled),
            pivot,
            trackers: used,
        });
    }
    out
}

/// The Offset Position of a pattern at `frame` (pp. 1741, 1746), in Fusion's units for a frame
/// of `size`: "essentially the path generated by the tracker", the centre of the pattern and
/// its offset. It is what a Center is connected to so that a thing follows the pattern.
///
/// `None` where the pattern is not tracked at the frame, or the offset or the size is not
/// finite.
#[must_use]
pub fn offset_position(tracker: &Tracker<'_>, frame: usize, size: (f64, f64)) -> Option<Point> {
    let centre = tracker.at(frame)?;
    let place = finite([centre[0] + tracker.offset[0], centre[1] + tracker.offset[1]])?;
    to_fusion(place, size)
}

/// The Steady Position and the Unsteady Position of one pattern (`SteadyPosition1` and
/// `UnsteadyPosition1` of p. 2784), in Fusion's units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Positions {
    /// 0.5, 0.5 less how far the pattern has gone since the reference frame.
    pub steady: Point,
    /// 0.5, 0.5 and how far the pattern has gone since the reference frame.
    pub unsteady: Point,
}

/// The steady and the unsteady position of one pattern at `frame`, from its own path alone: how
/// far its centre has gone since the frame `reference`, as a share of a frame of `size`, taken
/// from 0.5, 0.5 and added to it (the pattern of p. 1748, which goes from 0.5, 0.5 to 0.6, 0.5
/// and whose Steady Position is then 0.4, 0.5). An offset that stays as it is has no say in
/// how far the pattern has gone.
///
/// `None` where the pattern is not tracked at the frame or at the reference frame, or the size
/// is not finite or not more than nothing.
#[must_use]
pub fn tracker_positions(
    tracker: &Tracker<'_>,
    reference: usize,
    frame: usize,
    size: (f64, f64),
) -> Option<Positions> {
    let (width, height) = extent(size)?;
    let (from, to) = (tracker.at(reference)?, tracker.at(frame)?);
    let shift = [(to[0] - from[0]) / width, -(to[1] - from[1]) / height];
    Some(Positions {
        steady: [0.5 - shift[0], 0.5 - shift[1]],
        unsteady: [0.5 + shift[0], 0.5 + shift[1]],
    })
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::float_cmp,
        reason = "no motion, and a shift of numbers that floats hold whole, are those numbers as written"
    )]

    use super::*;
    use proptest::prelude::*;

    const SIZE: (f64, f64) = (1920.0, 1080.0);

    /// Five patterns, no three of them in a line.
    const FIVE: [Point; 5] = [
        [300.0, 200.0],
        [1500.0, 260.0],
        [960.0, 540.0],
        [420.0, 900.0],
        [1700.0, 820.0],
    ];

    /// A turn (clockwise on the screen, in radians) and a size about a point, and a shift.
    fn similarity(angle: f64, scale: f64, about: Point, shift: Point) -> Mat3 {
        Motion {
            pan: shift[0],
            tilt: shift[1],
            zoom: scale,
            rotate: angle,
            ..Motion::IDENTITY
        }
        .matrix(about)
    }

    /// The paths of patterns that start at `start` and are, frame after frame, where each of
    /// `moves` puts them.
    fn paths_of(start: &[Point], moves: &[Mat3]) -> Vec<Vec<Option<Point>>> {
        start
            .iter()
            .map(|&at| {
                let mut path = vec![Some(at)];
                path.extend(moves.iter().map(|moved| Some(moved.apply(at))));
                path
            })
            .collect()
    }

    fn trackers_of(paths: &[Vec<Option<Point>>]) -> Vec<Tracker<'_>> {
        paths
            .iter()
            .map(|path| Tracker::new(path.as_slice()))
            .collect()
    }

    /// Match Move, with every box on.
    fn settings() -> Settings {
        Settings {
            operation: Operation::MatchMove,
            ..Settings::default()
        }
    }

    fn close(a: Point, b: Point, within: f64) -> bool {
        (a[0] - b[0]).abs() <= within && (a[1] - b[1]).abs() <= within
    }

    /// A Transform node as the module reads it: a turn and a size about the Pivot, then a shift
    /// by the Center less 0.5, 0.5; all in Fusion's units, the matrix in pixels.
    fn transform_node(center: Point, pivot: Point, angle: f64, size: f64) -> Mat3 {
        Motion {
            pan: (center[0] - 0.5) * SIZE.0,
            tilt: -(center[1] - 0.5) * SIZE.1,
            zoom: size,
            rotate: -angle.to_radians(),
            ..Motion::IDENTITY
        }
        .matrix(from_fusion(pivot, SIZE).unwrap())
    }

    #[test]
    fn at_the_reference_frame_nothing_moves() {
        let moves = [
            similarity(0.2, 1.1, [900.0, 500.0], [40.0, -25.0]),
            similarity(-0.4, 0.8, [900.0, 500.0], [-70.0, 12.0]),
            similarity(0.9, 1.6, [900.0, 500.0], [5.0, 90.0]),
        ];
        let paths = paths_of(&FIVE, &moves);
        let chosen = Settings {
            reference: 2,
            ..settings()
        };
        let found = solve(&trackers_of(&paths), &chosen);
        assert_eq!(found.len(), 4);
        let still = found[2].unwrap();
        assert_eq!(still.motion, Motion::IDENTITY);
        assert_eq!(still.trackers, 5);
        assert_eq!(still.unsteady(), Mat3::IDENTITY);
        assert!(still.steady().distance_on(&Mat3::IDENTITY, &FIVE) < 1e-12);
        for way in [Way::Steady, Way::Unsteady] {
            let through = still.transform(Operation::MatchMove, way);
            assert!(through.distance_on(&Mat3::IDENTITY, &FIVE) < 1e-12);
        }
        let published = still.published(SIZE).unwrap();
        assert_eq!(published.steady_position, [0.5, 0.5]);
        assert_eq!(published.unsteady_position, [0.5, 0.5]);
        assert_eq!(
            (published.steady_angle, published.unsteady_angle),
            (0.0, 0.0)
        );
        assert_eq!((published.steady_size, published.unsteady_size), (1.0, 1.0));
        assert!(close(published.steady_axis, published.unsteady_axis, 1e-15));
        // The frames before it are measured from it too: the first is the second move undone.
        let back = moves[1].inverse().unwrap();
        assert!(found[0].unwrap().unsteady().distance_on(&back, &FIVE) < 1e-9);
    }

    #[test]
    fn one_tracker_gives_its_shift_and_neither_a_turn_nor_a_size() {
        let path = [Some([100.0, 200.0]), Some([112.5, 193.0])];
        let found = solve(&[Tracker::new(&path)], &settings());
        let moved = found[1].unwrap();
        assert_eq!((moved.motion.pan, moved.motion.tilt), (12.5, -7.0));
        assert_eq!((moved.motion.zoom, moved.motion.rotate), (1.0, 0.0));
        assert_eq!(moved.trackers, 1);
        assert_eq!(moved.pivot, [100.0, 200.0]);
        assert!(close(
            moved.unsteady().apply([640.0, 360.0]),
            [652.5, 353.0],
            1e-12
        ));
        let published = moved.published(SIZE).unwrap();
        // Right is more of x, and up (less of the tracker's y) is more of Fusion's y.
        assert!(close(
            published.unsteady_position,
            [0.5 + 12.5 / 1920.0, 0.5 + 7.0 / 1080.0],
            1e-15
        ));
        assert!(close(
            published.steady_position,
            [0.5 - 12.5 / 1920.0, 0.5 - 7.0 / 1080.0],
            1e-15
        ));
        assert_eq!(
            (published.steady_angle, published.unsteady_angle),
            (0.0, 0.0)
        );
        assert_eq!((published.steady_size, published.unsteady_size), (1.0, 1.0));
        // The pattern of the manual (p. 1748): from the centre, a tenth of the width to the
        // right.
        let manual = [Some([960.0, 540.0]), Some([1152.0, 540.0])];
        let found = solve(&[Tracker::new(&manual)], &settings());
        let published = found[1].unwrap().published(SIZE).unwrap();
        assert!(close(published.steady_position, [0.4, 0.5], 1e-12));
        assert!(close(published.unsteady_position, [0.6, 0.5], 1e-12));
        let own = tracker_positions(&Tracker::new(&manual), 0, 1, SIZE).unwrap();
        assert!(close(own.steady, [0.4, 0.5], 1e-12));
        assert!(close(own.unsteady, [0.6, 0.5], 1e-12));
    }

    #[test]
    fn two_trackers_turned_about_their_middle_give_the_turn_and_no_size() {
        let start = [[800.0, 500.0], [1100.0, 600.0]];
        let middle = [950.0, 550.0];
        // Thirty degrees counter-clockwise on the screen, which for a motion is less.
        let turned = similarity(-30.0_f64.to_radians(), 1.0, middle, [0.0, 0.0]);
        let paths = paths_of(&start, &[turned]);
        let found = solve(&trackers_of(&paths), &settings())[1].unwrap();
        assert!((found.motion.rotate + 30.0_f64.to_radians()).abs() < 1e-12);
        assert!((found.motion.zoom - 1.0).abs() < 1e-12);
        assert!(found.motion.pan.abs() < 1e-9 && found.motion.tilt.abs() < 1e-9);
        assert!(close(found.pivot, middle, 1e-12));
        let published = found.published(SIZE).unwrap();
        assert!((published.unsteady_angle - 30.0).abs() < 1e-9);
        assert!((published.steady_angle + 30.0).abs() < 1e-9);
        assert!((published.unsteady_size - 1.0).abs() < 1e-12);
        assert!(close(published.unsteady_position, [0.5, 0.5], 1e-12));
    }

    #[test]
    fn two_trackers_twice_as_far_apart_give_a_size_of_two() {
        let start = [[800.0, 500.0], [1100.0, 600.0]];
        let larger = similarity(0.0, 2.0, [950.0, 550.0], [0.0, 0.0]);
        let paths = paths_of(&start, &[larger]);
        let found = solve(&trackers_of(&paths), &settings())[1].unwrap();
        assert!((found.motion.zoom - 2.0).abs() < 1e-12);
        assert!(found.motion.rotate.abs() < 1e-12);
        assert!(found.motion.pan.abs() < 1e-9 && found.motion.tilt.abs() < 1e-9);
        let published = found.published(SIZE).unwrap();
        assert!((published.unsteady_size - 2.0).abs() < 1e-12);
        assert!((published.steady_size - 0.5).abs() < 1e-12);
        assert!(published.unsteady_angle.abs() < 1e-9);
    }

    #[test]
    fn a_box_that_is_off_leaves_its_part_as_no_motion() {
        let moved = similarity(0.3, 1.4, [960.0, 540.0], [80.0, -45.0]);
        let paths = paths_of(&FIVE, &[moved]);
        let trackers = trackers_of(&paths);
        let with = |position: bool, rotation: bool, scaling: bool| {
            let boxes = Settings {
                position,
                rotation,
                scaling,
                ..settings()
            };
            solve(&trackers, &boxes)[1].unwrap()
        };
        let whole = with(true, true, true);
        assert!(whole.unsteady().distance_on(&moved, &FIVE) < 1e-9);

        let unshifted = with(false, true, true);
        assert_eq!((unshifted.motion.pan, unshifted.motion.tilt), (0.0, 0.0));
        assert_eq!(unshifted.motion.rotate, whole.motion.rotate);
        assert_eq!(unshifted.motion.zoom, whole.motion.zoom);
        let published = unshifted.published(SIZE).unwrap();
        assert_eq!(published.steady_position, [0.5, 0.5]);
        assert_eq!(published.unsteady_position, [0.5, 0.5]);
        // The pivot then stays where it is.
        assert!(close(
            unshifted.unsteady().apply(unshifted.pivot),
            unshifted.pivot,
            1e-9
        ));

        let unturned = with(true, false, true);
        assert_eq!(unturned.motion.rotate, 0.0);
        assert_eq!(unturned.motion.pan, whole.motion.pan);
        assert_eq!(unturned.motion.zoom, whole.motion.zoom);
        let published = unturned.published(SIZE).unwrap();
        assert_eq!(
            (published.steady_angle, published.unsteady_angle),
            (0.0, 0.0)
        );

        let unscaled = with(true, true, false);
        assert_eq!(unscaled.motion.zoom, 1.0);
        assert_eq!(unscaled.motion.rotate, whole.motion.rotate);
        let published = unscaled.published(SIZE).unwrap();
        assert_eq!((published.steady_size, published.unsteady_size), (1.0, 1.0));

        let none = with(false, false, false);
        assert_eq!(none.motion, Motion::IDENTITY);
        assert_eq!(none.unsteady(), Mat3::IDENTITY);

        // The boxes as components of a motion.
        let boxes = Settings {
            rotation: false,
            ..settings()
        }
        .components();
        assert!(boxes.pan && boxes.tilt && boxes.zoom && !boxes.rotate && !boxes.perspective);
    }

    #[test]
    fn an_operation_of_none_leaves_the_picture_and_publishes_all_the_same() {
        let moved = similarity(0.3, 1.4, [960.0, 540.0], [80.0, -45.0]);
        let paths = paths_of(&FIVE, &[moved]);
        let whole = solve(&trackers_of(&paths), &Settings::default())[1].unwrap();
        assert_eq!(Settings::default().operation, Operation::None);
        assert_eq!(
            whole.transform(Operation::None, Way::Steady),
            Mat3::IDENTITY
        );
        assert_eq!(
            whole.transform(Operation::None, Way::Unsteady),
            Mat3::IDENTITY
        );
        assert_eq!(
            whole.transform(Operation::MatchMove, Way::Unsteady),
            whole.unsteady()
        );
        assert_eq!(
            whole.transform(Operation::MatchMove, Way::Steady),
            whole.steady()
        );
        let published = whole.published(SIZE).unwrap();
        assert!((published.unsteady_size - 1.4).abs() < 1e-9);
        assert!((published.steady_size - 1.0 / 1.4).abs() < 1e-9);
    }

    #[test]
    fn a_tracker_missing_at_a_frame_is_left_out_and_the_rest_still_fit() {
        let first = similarity(0.25, 1.2, [960.0, 540.0], [30.0, 60.0]);
        let second = similarity(-0.5, 0.7, [960.0, 540.0], [-90.0, 15.0]);
        let mut paths = paths_of(&FIVE, &[first, second]);
        // One is hidden at the second frame, and one is found at a place that is no place.
        paths[1][1] = None;
        paths[3][1] = Some([f64::NAN, 400.0]);
        // One was never tracked at the reference frame: it is left out of every frame.
        paths[4][0] = None;
        let found = solve(&trackers_of(&paths), &settings());
        let (at_first, at_second) = (found[1].unwrap(), found[2].unwrap());
        assert_eq!((at_first.trackers, at_second.trackers), (2, 4));
        assert!(at_first.unsteady().distance_on(&first, &FIVE) < 1e-9);
        assert!(at_second.unsteady().distance_on(&second, &FIVE) < 1e-9);
        // The pivot is the mean of those tracked at the reference frame, whichever are tracked
        // at a frame.
        let mean = [
            (300.0 + 1500.0 + 960.0 + 420.0) / 4.0,
            (200.0 + 260.0 + 540.0 + 900.0) / 4.0,
        ];
        assert!(close(at_first.pivot, mean, 1e-12));
        assert_eq!(at_first.pivot, at_second.pivot);
        // With all but one hidden, what is left is a shift.
        paths[2][1] = None;
        let found = solve(&trackers_of(&paths), &settings());
        let alone = found[1].unwrap();
        assert_eq!(alone.trackers, 1);
        assert_eq!((alone.motion.zoom, alone.motion.rotate), (1.0, 0.0));
        let gone = first.apply(FIVE[0]);
        assert!((alone.motion.pan - (gone[0] - FIVE[0][0])).abs() < 1e-12);
        assert!((alone.motion.tilt - (gone[1] - FIVE[0][1])).abs() < 1e-12);
    }

    #[test]
    fn a_frame_with_no_tracker_has_no_solution() {
        let moved = similarity(0.1, 1.0, [960.0, 540.0], [10.0, 0.0]);
        let mut paths = paths_of(&FIVE[..2], &[moved, moved]);
        paths[0][1] = None;
        paths[1][1] = None;
        let found = solve(&trackers_of(&paths), &settings());
        assert!(found[0].is_some() && found[1].is_none() && found[2].is_some());
        // A reference frame with nothing tracked, or beyond the paths, measures nothing.
        for reference in [1, 3, 99] {
            let chosen = Settings {
                reference,
                ..settings()
            };
            assert_eq!(solve(&trackers_of(&paths), &chosen), vec![None; 3]);
        }
        // A path that ends early is not tracked after its end.
        let short = [Some([10.0, 10.0])];
        let long = [None, Some([20.0, 20.0]), Some([25.0, 20.0])];
        let both = [Tracker::new(&short), Tracker::new(&long)];
        assert_eq!(frames(&both), 3);
        assert_eq!(both[0].at(1), None);
        let found = solve(&both, &settings());
        assert_eq!(found.len(), 3);
        assert!(found[0].is_some() && found[1].is_none() && found[2].is_none());
        // No tracker, no frame.
        assert_eq!(frames(&[]), 0);
        assert_eq!(solve(&[], &settings()), Vec::new());
    }

    #[test]
    fn an_offset_shifts_the_offset_position_and_nothing_else() {
        let moved = similarity(0.2, 1.3, [960.0, 540.0], [25.0, -40.0]);
        let paths = paths_of(&FIVE[..3], &[moved]);
        let plain = trackers_of(&paths);
        let mut shifted = plain.clone();
        shifted[0].offset = [30.0, -20.0];
        // Thirty pixels to the right and twenty up, as shares of the frame.
        for frame in 0..2 {
            let at = offset_position(&plain[0], frame, SIZE).unwrap();
            let with = offset_position(&shifted[0], frame, SIZE).unwrap();
            assert!(close(
                with,
                [at[0] + 30.0 / 1920.0, at[1] + 20.0 / 1080.0],
                1e-12
            ));
        }
        // With none, it is the path: from the bottom left, y up.
        let first = offset_position(&plain[0], 0, SIZE).unwrap();
        assert!(close(first, [300.0 / 1920.0, 1.0 - 200.0 / 1080.0], 1e-15));
        assert!(close(from_fusion(first, SIZE).unwrap(), FIVE[0], 1e-9));
        assert_eq!(to_fusion([0.0, 1080.0], SIZE), Some([0.0, 0.0]));
        assert_eq!(to_fusion([1920.0, 0.0], SIZE), Some([1.0, 1.0]));
        assert_eq!(to_fusion([960.0, 540.0], SIZE), Some([0.5, 0.5]));
        // The motion, and the pattern's own steady position, are those of the pattern's centre.
        assert_eq!(solve(&shifted, &settings()), solve(&plain, &settings()));
        assert_eq!(
            tracker_positions(&shifted[0], 0, 1, SIZE),
            tracker_positions(&plain[0], 0, 1, SIZE)
        );
        // Not tracked, or an offset or a frame that is no number: no position.
        assert_eq!(offset_position(&plain[0], 7, SIZE), None);
        shifted[0].offset = [f64::NAN, 0.0];
        assert_eq!(offset_position(&shifted[0], 0, SIZE), None);
        assert_eq!(offset_position(&plain[0], 0, (0.0, 1080.0)), None);
        assert_eq!(tracker_positions(&plain[0], 0, 7, SIZE), None);
        assert_eq!(tracker_positions(&plain[0], 0, 1, (1920.0, f64::NAN)), None);
    }

    #[test]
    fn trackers_at_one_place_and_numbers_that_are_none_are_refused() {
        // Two patterns at one place at the reference frame tell no turn and no size: no frame
        // has a solution, the reference frame among them.
        let same = [Some([500.0, 500.0]), Some([520.0, 480.0])];
        let found = solve(&[Tracker::new(&same), Tracker::new(&same)], &settings());
        assert_eq!(found, vec![None; 2]);
        // Two patterns that come together afterwards: no solution there, one before and after.
        let one = [
            Some([400.0, 300.0]),
            Some([700.0, 300.0]),
            Some([410.0, 300.0]),
        ];
        let other = [
            Some([900.0, 300.0]),
            Some([700.0, 300.0]),
            Some([910.0, 300.0]),
        ];
        let found = solve(&[Tracker::new(&one), Tracker::new(&other)], &settings());
        assert!(found[0].is_some() && found[1].is_none() && found[2].is_some());
        // A point that is not finite counts as not tracked.
        let broken = [Some([400.0, 300.0]), Some([f64::INFINITY, 300.0])];
        assert_eq!(Tracker::new(&broken).at(1), None);
        let found = solve(&[Tracker::new(&broken)], &settings());
        assert!(found[0].is_some() && found[1].is_none());
        let never = [Some([f64::NAN, f64::NAN]), Some([400.0, 300.0])];
        assert_eq!(solve(&[Tracker::new(&never)], &settings()), vec![None; 2]);
        // A frame of no size publishes nothing.
        let moved = solve(&[Tracker::new(&one), Tracker::new(&other)], &settings())[2].unwrap();
        assert!(moved.published(SIZE).is_some());
        for size in [(0.0, 1080.0), (1920.0, -1.0), (f64::NAN, 1080.0)] {
            assert_eq!(moved.published(size), None);
            assert_eq!(to_fusion([1.0, 1.0], size), None);
            assert_eq!(from_fusion([0.5, 0.5], size), None);
        }
    }

    #[test]
    fn the_pivot_is_what_the_turn_and_the_size_are_about() {
        let start = [[600.0, 400.0], [900.0, 500.0], [700.0, 800.0]];
        // A turn about the first pattern, which stays where it is.
        let turned = similarity(0.35, 1.0, start[0], [0.0, 0.0]);
        let paths = paths_of(&start, &[turned]);
        let trackers = trackers_of(&paths);
        let with = |pivot: Pivot, position: bool| {
            let chosen = Settings {
                position,
                pivot,
                ..settings()
            };
            solve(&trackers, &chosen)[1].unwrap()
        };
        let mean = [2200.0 / 3.0, 1700.0 / 3.0];
        let average = with(Pivot::TrackerAverage, true);
        let selected = with(Pivot::SelectedTracker(0), true);
        let manual = with(Pivot::Manual([0.0, 0.0]), true);
        assert!(close(average.pivot, mean, 1e-12));
        assert_eq!(selected.pivot, start[0]);
        assert_eq!(manual.pivot, [0.0, 0.0]);
        // With every box on, the picture goes the same way whatever the pivot.
        for found in [average, selected, manual] {
            assert!(found.unsteady().distance_on(&turned, &start) < 1e-9);
            assert!((found.motion.rotate - 0.35).abs() < 1e-12);
        }
        // About the pattern that stays there is no shift; about another point there is.
        assert!(selected.motion.pan.abs() < 1e-9 && selected.motion.tilt.abs() < 1e-9);
        let gone = turned.apply(mean);
        assert!((average.motion.pan - (gone[0] - mean[0])).abs() < 1e-9);
        assert!(average.motion.pan.abs() > 1.0);
        // Without Position the turn is about the pivot, which stays.
        let about_first = with(Pivot::SelectedTracker(0), false);
        assert!(about_first.unsteady().distance_on(&turned, &start) < 1e-9);
        let about_mean = with(Pivot::TrackerAverage, false);
        assert!(close(about_mean.unsteady().apply(mean), mean, 1e-9));
        // A pivot that cannot be had is the tracker average.
        assert_eq!(with(Pivot::SelectedTracker(9), true), average);
        assert_eq!(with(Pivot::Manual([f64::NAN, 0.0]), true), average);
    }

    #[test]
    fn a_thing_that_goes_round_does_not_jump_back_by_a_turn() {
        let start = [[800.0, 500.0], [1100.0, 600.0]];
        let middle = [950.0, 550.0];
        // Fifty degrees with each frame, for fifteen frames after the first.
        let moves: Vec<Mat3> = (1..16_u16)
            .map(|frame| {
                similarity(
                    (f64::from(frame) * 50.0).to_radians(),
                    1.0,
                    middle,
                    [0.0, 0.0],
                )
            })
            .collect();
        let paths = paths_of(&start, &moves);
        let chosen = Settings {
            reference: 8,
            ..settings()
        };
        let found = solve(&trackers_of(&paths), &chosen);
        for frame in 0..16_u16 {
            let since = (f64::from(frame) - 8.0) * 50.0;
            let solved = found[usize::from(frame)].unwrap();
            assert!(
                (solved.motion.rotate - since.to_radians()).abs() < 1e-9,
                "frame {frame}"
            );
            let published = solved.published(SIZE).unwrap();
            assert!((published.unsteady_angle + since).abs() < 1e-6);
            assert!((published.steady_angle - since).abs() < 1e-6);
        }
    }

    #[test]
    fn a_transform_node_connected_to_the_published_values_does_what_the_matrices_do() {
        let moved = similarity(-0.6, 1.7, [700.0, 300.0], [-120.0, 85.0]);
        let paths = paths_of(&FIVE, &[moved]);
        let trackers = trackers_of(&paths);
        for pivot in [
            Pivot::TrackerAverage,
            Pivot::SelectedTracker(3),
            Pivot::Manual([200.0, 1000.0]),
        ] {
            for (position, rotation, scaling) in [
                (true, true, true),
                (false, true, true),
                (true, false, true),
                (true, true, false),
            ] {
                let chosen = Settings {
                    position,
                    rotation,
                    scaling,
                    pivot,
                    ..settings()
                };
                let found = solve(&trackers, &chosen)[1].unwrap();
                let published = found.published(SIZE).unwrap();
                let follow = transform_node(
                    published.unsteady_position,
                    published.unsteady_axis,
                    published.unsteady_angle,
                    published.unsteady_size,
                );
                assert!(follow.distance_on(&found.unsteady(), &FIVE) < 1e-8);
                let hold = transform_node(
                    published.steady_position,
                    published.steady_axis,
                    published.steady_angle,
                    published.steady_size,
                );
                assert!(hold.distance_on(&found.steady(), &FIVE) < 1e-8);
                // Held still and then let go, a picture is where it was.
                let whole = hold.mul(&follow);
                assert!(whole.distance_on(&Mat3::IDENTITY, &FIVE) < 1e-8);
            }
        }
    }

    proptest! {
        #[test]
        fn a_similarity_of_five_points_is_found_again_and_steady_undoes_unsteady(
            angle in -3.0_f64..3.0,
            scale in 0.3_f64..3.0,
            dx in -300.0_f64..300.0,
            dy in -300.0_f64..300.0,
            px in 0.0_f64..1920.0,
            py in 0.0_f64..1080.0,
        ) {
            let moved = similarity(angle, scale, [px, py], [dx, dy]);
            let paths = paths_of(&FIVE, &[moved]);
            let found = solve(&trackers_of(&paths), &settings())[1].unwrap();
            prop_assert_eq!(found.trackers, 5);
            prop_assert!(found.unsteady().distance_on(&moved, &FIVE) < 1e-9);
            prop_assert!((found.motion.rotate - angle).abs() < 1e-9);
            prop_assert!((found.motion.zoom - scale).abs() < 1e-9);
            // The shift is where the pivot goes.
            let gone = moved.apply(found.pivot);
            prop_assert!((found.motion.pan - (gone[0] - found.pivot[0])).abs() < 1e-9);
            prop_assert!((found.motion.tilt - (gone[1] - found.pivot[1])).abs() < 1e-9);
            // Steady after unsteady, and unsteady after steady, leave every point.
            let there = found.steady().mul(&found.unsteady());
            let back = found.unsteady().mul(&found.steady());
            prop_assert!(there.distance_on(&Mat3::IDENTITY, &FIVE) < 1e-9);
            prop_assert!(back.distance_on(&Mat3::IDENTITY, &FIVE) < 1e-9);
            // A frame held still has its patterns where they were at the reference frame.
            for (path, start) in paths.iter().zip(&FIVE) {
                let held = found.steady().apply(path[1].unwrap());
                prop_assert!(close(held, *start, 1e-9));
            }
            let published = found.published(SIZE).unwrap();
            prop_assert!((published.steady_angle + published.unsteady_angle).abs() < 1e-12);
            prop_assert!((published.steady_size * published.unsteady_size - 1.0).abs() < 1e-12);
            prop_assert!((published.unsteady_angle + angle.to_degrees()).abs() < 1e-6);
            prop_assert!((published.steady_position[0] + published.unsteady_position[0] - 1.0).abs() < 1e-12);
            prop_assert!((published.steady_position[1] + published.unsteady_position[1] - 1.0).abs() < 1e-12);
        }
    }
}
