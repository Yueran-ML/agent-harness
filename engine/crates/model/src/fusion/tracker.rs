//! The Tracker of a composition (Reference Manual 20, Chapter 82, pp. 1724–1759, and the node,
//! pp. 2773–2784): its patterns and their paths, the motion that the paths tell of, what the
//! node draws with it (Match Move), and what it publishes for the centres of other nodes to
//! follow. What it is in a composition is told with the composition (The Tracker and
//! Following a Tracker, in the module above); the mathematics is that of
//! `columbina_track::matchmove`, which is not written a second time.
//!
//! Finding a pattern in the pictures of a clip, and so writing its path, is not done here: a
//! path is a setting, and is set as one.
//!
//! # Units
//!
//! The settings are Fusion's: a place is a share of the width and of the height of the
//! picture from its lower left corner, `y` up. The mathematics counts in pixels of a frame
//! that reaches from 0 to its width and from 0 to its height, `y` down: a place `(x, y)` of a
//! picture `W` wide and `H` high is at `(x · W, (1 − y) · H)` there. A pixel `p` of a picture
//! reaches from `p` to `p + 1`, as the masks and the pictures of the compositor have it, so
//! that the middle of the pixel in column `i` and row `j`, counted from the top, is the place
//! `((i + 0.5) / W, 1 − (j + 0.5) / H)`.

use std::collections::BTreeMap;

use columbina_track::matchmove::{self, Operation, Pivot, Settings, Solution, Way};
use serde::{Deserialize, Serialize};

use super::bounds;
use crate::grade::{Bounds, LONGEST_LABEL};
use crate::ops::{OpError, LARGEST};
use crate::project::Id;

/// The most patterns of a [`Tracker`]. The manual gives none: this is ours.
pub const MOST_TRACKERS: usize = 16;

/// What a [`Tracker`] does with its pictures (the Operation menu, p. 2778). It is written
/// `"none"` or `"match_move"`, and takes no keys. Corner Positioning and Perspective
/// Positioning are not kept.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackerOperation {
    /// Nothing: the node is the picture that comes into its `input`. What it publishes is
    /// published all the same. Every Tracker has it at first.
    #[default]
    None,
    /// The motion of the patterns is taken out of the picture that was tracked, or given to
    /// the foreground, as [`TrackerMerge`] says.
    MatchMove,
}

/// Which pictures a [`Tracker`] that match moves draws, and which way the motion goes (the
/// Merge menu, pp. 1742, 1744, 2779). It is written `"bg_only"`, `"fg_only"`, `"fg_over_bg"`
/// or `"bg_over_fg"`, and takes no keys.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackerMerge {
    /// The picture that was tracked, held still: each frame is brought back to where it was
    /// at the reference frame. Every Tracker has it at first.
    #[default]
    BgOnly,
    /// The foreground alone, moved as the patterns move, over nothing.
    FgOnly,
    /// The foreground, moved as the patterns move, laid over the picture that was tracked as
    /// it is.
    FgOverBg,
    /// The picture that was tracked, as it is, laid over the foreground moved as the patterns
    /// move.
    BgOverFg,
}

/// The point that the turn and the size of a [`Tracker`] are about (the Pivot Type menu,
/// pp. 1743, 2783). It is written `"tracker_average"`, `"selected_tracker"` or `"manual"`, and
/// takes no keys.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackerPivot {
    /// The mean of where the patterns are at the reference frame. Every Tracker has it at
    /// first.
    #[default]
    TrackerAverage,
    /// Where one pattern is at the reference frame: the one at `pivot_tracker` of the list.
    /// Where there is none there, or it is off or has no path, the pivot is the tracker
    /// average.
    SelectedTracker,
    /// The place `pivot_point`.
    Manual,
}

/// A place of a [`Tracker`] that takes no keys: `x` the share of the width of the picture from
/// its left edge and `y` the share of its height from its lower edge, as a
/// [`Center`](super::Center) counts them; and, of an offset, how far right and how far up, in
/// the same shares.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Place {
    /// How far right, as a share of the width.
    pub x: f64,
    /// How far up, as a share of the height.
    pub y: f64,
}

/// Where a pattern of a [`Tracker`] was put, and how large it is (the Pattern Width and
/// Height, p. 2776): what the tracking looks for. They are numbers, and take no keys.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatternArea {
    /// Where its middle was put: how far right, as a share of the width of the picture.
    pub x: f64,
    /// Where its middle was put: how far up, as a share of the height of the picture.
    pub y: f64,
    /// How wide it is, in widths of the picture.
    pub width: f64,
    /// How high it is, in heights of the picture.
    pub height: f64,
}

/// How large the area is that the tracking searches for a pattern, about where the pattern
/// was in the frame before (the Search Width and Height, p. 2776). They are numbers, and take
/// no keys.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchArea {
    /// How wide it is, in widths of the picture.
    pub width: f64,
    /// How high it is, in heights of the picture.
    pub height: f64,
}

/// One pattern of a [`Tracker`]: what the tracking looks for, and the path that it found.
///
/// The path is where the middle of the pattern was found, at frames of the keys of the clip
/// (those that the keys of the other settings count): `[x, y]`, as a
/// [`Center`](super::Center) counts them. It is written as an object of the frames:
/// `{"12": [0.5, 0.25], "13": [0.51, 0.25]}`. A frame that it has not was not tracked: the
/// pattern is then taken to be on the line between the frames about it that were, and where
/// it was at the first of them before that one, and at the last after it ([`Pattern::place_at`]).
/// A pattern with no path is not tracked at all.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(
    clippy::struct_field_names,
    reason = "`pattern` is the field as it is written: what Fusion calls the pattern of a tracker"
)]
pub struct Pattern {
    /// The name that it was given, or empty: at most
    /// [`LONGEST_LABEL`](crate::grade::LONGEST_LABEL) characters.
    pub name: String,
    /// Whether it counts: one that is off is left out of the motion, as if it had no path.
    /// It publishes its own place all the same.
    pub enabled: bool,
    /// Where it was put, and how large it is.
    pub pattern: PatternArea,
    /// How large the area is that it is searched for in.
    pub search: SearchArea,
    /// The X and Y Offset (pp. 1741, 2777): how far from the middle of the pattern the thing
    /// lies that the path is wanted for, right and up. It shifts what the pattern publishes,
    /// and has no say in the motion.
    pub offset: Place,
    /// Its path: where its middle was found, by the frame.
    pub path: BTreeMap<i64, [f64; 2]>,
}

impl Default for Pattern {
    /// A pattern in the middle of the picture, a twentieth of it wide and high, searched for
    /// in an eighth of it, with no offset and no path, on, with no name. The sizes are ours:
    /// the manual gives none.
    fn default() -> Self {
        Self::new(bounds::TRACKER_PLACE.default, bounds::TRACKER_PLACE.default)
    }
}

impl Pattern {
    /// A pattern put at `x` and `y`, as one is at first but for where it is.
    #[must_use]
    pub fn new(x: f64, y: f64) -> Self {
        Self {
            name: String::new(),
            enabled: true,
            pattern: PatternArea {
                x,
                y,
                width: bounds::TRACKER_PATTERN.default,
                height: bounds::TRACKER_PATTERN.default,
            },
            search: SearchArea {
                width: bounds::TRACKER_SEARCH.default,
                height: bounds::TRACKER_SEARCH.default,
            },
            offset: Place {
                x: bounds::TRACKER_OFFSET.default,
                y: bounds::TRACKER_OFFSET.default,
            },
            path: BTreeMap::new(),
        }
    }

    /// Where the middle of the pattern is at `frame` of its keys, which may lie between
    /// frames: where its path has it; on the line between the two frames of its path about
    /// `frame`, where that is between two; and where the first frame of its path has it
    /// before that one, and the last after it. None of a pattern that has no path, and at a
    /// frame that is no number.
    #[must_use]
    pub fn place_at(&self, frame: f64) -> Option<[f64; 2]> {
        if frame.is_nan() {
            return None;
        }
        let (&first, &start) = self.path.first_key_value()?;
        let (&last, &end) = self.path.last_key_value()?;
        #[allow(clippy::cast_precision_loss, reason = "frames are within 2^53")]
        let at = |key: i64| key as f64;
        if frame <= at(first) {
            return Some(start);
        }
        if frame >= at(last) {
            return Some(end);
        }
        // The frame is after the first of the path and before the last: one of the path is
        // at it or before it, and one after it.
        #[allow(
            clippy::cast_possible_truncation,
            reason = "a frame between two frames of the path"
        )]
        let whole = frame.floor() as i64;
        let (&before, &from) = self.path.range(..=whole).next_back()?;
        let (&after, &to) = self.path.range(whole.saturating_add(1)..).next()?;
        let share = (frame - at(before)) / (at(after) - at(before));
        Some([
            from[0] + (to[0] - from[0]) * share,
            from[1] + (to[1] - from[1]) * share,
        ])
    }

    /// Check it: its name, its numbers each within its bounds, and its path of places within
    /// theirs at frames within [`LARGEST`] of 0.
    fn check(&self) -> Result<(), OpError> {
        if self.name.chars().count() > LONGEST_LABEL {
            return Err(refused("tracker.trackers.name"));
        }
        let PatternArea {
            x,
            y,
            width,
            height,
        } = self.pattern;
        let placed = [x, y].iter().all(|&at| within(bounds::TRACKER_PLACE, at));
        let sized = [width, height]
            .iter()
            .all(|&side| within(bounds::TRACKER_PATTERN, side));
        if !(placed && sized) {
            return Err(refused("tracker.trackers.pattern"));
        }
        let searched = [self.search.width, self.search.height];
        if !searched
            .iter()
            .all(|&side| within(bounds::TRACKER_SEARCH, side))
        {
            return Err(refused("tracker.trackers.search"));
        }
        let offset = [self.offset.x, self.offset.y];
        if !offset.iter().all(|&by| within(bounds::TRACKER_OFFSET, by)) {
            return Err(refused("tracker.trackers.offset"));
        }
        let fits = self.path.iter().all(|(frame, place)| {
            frame.unsigned_abs() <= LARGEST.unsigned_abs()
                && place.iter().all(|&at| within(bounds::TRACKER_PLACE, at))
        });
        if fits {
            Ok(())
        } else {
            Err(refused("tracker.trackers.path"))
        }
    }
}

/// Whether `value` is a number within `bounds`.
fn within(bounds: Bounds, value: f64) -> bool {
    value.is_finite() && (bounds.low..=bounds.high).contains(&value)
}

fn refused(what: &'static str) -> OpError {
    OpError::OutOfRange { what }
}

/// The settings of a Tracker (pp. 2773–2784): its patterns with their paths, and the controls
/// of its Operation tab, those that the mathematics of the match move has.
///
/// None of its numbers takes keys: the paths of its patterns are what moves. The frames of
/// the paths, and `reference`, are frames of the keys of the clip, and are moved with them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(
    clippy::struct_field_names,
    reason = "`pivot_tracker` is the field as it is written: the pattern that the pivot is at"
)]
pub struct Tracker {
    /// Operation (p. 2778).
    pub operation: TrackerOperation,
    /// Merge (p. 2779): which pictures are drawn, and which way the motion goes.
    pub merge: TrackerMerge,
    /// Position (p. 2782): whether the shift is corrected.
    pub position: bool,
    /// Rotation (p. 2782): whether the turn is corrected. It takes two patterns to tell one.
    pub rotation: bool,
    /// Scaling (p. 2782): whether the size is corrected. It takes two patterns to tell one.
    pub scaling: bool,
    /// Pivot Type (p. 2783): the point that the turn and the size are about.
    pub pivot: TrackerPivot,
    /// The pattern whose place is the pivot, by its place in `trackers`, from 0: read where
    /// `pivot` is `selected_tracker`, and kept otherwise. Below [`MOST_TRACKERS`].
    pub pivot_tracker: usize,
    /// The pivot given by hand: read where `pivot` is `manual`, and kept otherwise.
    pub pivot_point: Place,
    /// Reference (p. 2783): the frame, of the keys of the clip, that the picture is brought
    /// back to, at which nothing moves. Each pattern is where its path has it there
    /// ([`Pattern::place_at`]): so a reference before the first frame of the paths is that
    /// frame, and one after the last is the last.
    pub reference: i64,
    /// Its patterns, at most [`MOST_TRACKERS`]: the list is set whole.
    pub trackers: Vec<Pattern>,
}

impl Default for Tracker {
    /// No patterns; an Operation of None, BG Only, the three boxes on, the pivot at the
    /// tracker average, and frame 0 as the reference: the controls as the mathematics has
    /// them at first.
    fn default() -> Self {
        Self {
            operation: TrackerOperation::None,
            merge: TrackerMerge::BgOnly,
            position: true,
            rotation: true,
            scaling: true,
            pivot: TrackerPivot::TrackerAverage,
            pivot_tracker: 0,
            pivot_point: Place {
                x: bounds::TRACKER_PLACE.default,
                y: bounds::TRACKER_PLACE.default,
            },
            reference: 0,
            trackers: Vec::new(),
        }
    }
}

/// The width and the height of a picture of `size` pixels, as the mathematics takes them.
fn extent(size: (u32, u32)) -> (f64, f64) {
    (f64::from(size.0), f64::from(size.1))
}

impl Tracker {
    /// Whether a pattern of it has a path: what it draws and publishes then changes with the
    /// frame.
    #[must_use]
    pub fn has_paths(&self) -> bool {
        self.trackers.iter().any(|pattern| !pattern.path.is_empty())
    }

    /// The motion of its patterns from the reference frame to `frame` of its keys, in a
    /// picture of `size` pixels: what `columbina_track::matchmove::solve` makes of where each
    /// pattern is at the two frames ([`Pattern::place_at`]), by the boxes and the pivot. The
    /// patterns that count are those that are on and have a path. None where none counts, and
    /// where the mathematics has no solution (two patterns that were at one place at the
    /// reference frame, or that have come together).
    ///
    /// The motion is found whatever the Operation is: the node publishes it all the same.
    /// Its turn is within half a turn of none: it is not counted on from frame to frame, which
    /// neither a picture drawn through it nor a place that is published can tell.
    #[must_use]
    pub fn solution(&self, frame: f64, size: (u32, u32)) -> Option<Solution> {
        let extent = extent(size);
        #[allow(clippy::cast_precision_loss, reason = "frames are within 2^53")]
        let reference = self.reference as f64;
        // Each pattern where it was at the reference frame and where it is at the frame, in
        // the pixels of the mathematics: a path of two frames. One that does not count is
        // tracked at neither, and keeps its place in the list, which the pivot may name.
        let paths: Vec<[Option<[f64; 2]>; 2]> = self
            .trackers
            .iter()
            .map(|pattern| {
                if !pattern.enabled {
                    return [None, None];
                }
                [reference, frame].map(|at| {
                    let place = pattern.place_at(at)?;
                    matchmove::from_fusion(place, extent)
                })
            })
            .collect();
        // Where no pattern has moved, the frame is the reference frame, at which nothing
        // moves at all: the second frame of the paths is the first.
        let moved = paths.iter().any(|[from, to]| from != to);
        let trackers: Vec<matchmove::Tracker<'_>> = paths
            .iter()
            .map(|path| matchmove::Tracker::new(&path[..]))
            .collect();
        let manual = [self.pivot_point.x, self.pivot_point.y];
        let settings = Settings {
            operation: Operation::MatchMove,
            way: Way::Steady,
            position: self.position,
            rotation: self.rotation,
            scaling: self.scaling,
            pivot: match self.pivot {
                TrackerPivot::TrackerAverage => Pivot::TrackerAverage,
                TrackerPivot::SelectedTracker => Pivot::SelectedTracker(self.pivot_tracker),
                TrackerPivot::Manual => matchmove::from_fusion(manual, extent)
                    .map_or(Pivot::TrackerAverage, Pivot::Manual),
            },
            reference: 0,
        };
        matchmove::solve(&trackers, &settings)
            .get(usize::from(moved))
            .copied()
            .flatten()
    }

    /// What it draws at `frame` of its keys, in a picture of `size` pixels: its Operation and
    /// its Merge, and how the picture that is moved is put ([`Tracked`]). Where its Operation
    /// is None, and where there is no motion to be had ([`Tracker::solution`]), the picture
    /// lies as it is.
    #[must_use]
    pub fn at(&self, frame: f64, size: (u32, u32)) -> Tracked {
        let still = Tracked {
            operation: self.operation,
            merge: self.merge,
            center: [0.5, 0.5],
            size: 1.0,
            angle: 0.0,
        };
        if self.operation == TrackerOperation::None {
            return still;
        }
        let way = match self.merge {
            TrackerMerge::BgOnly => Way::Steady,
            TrackerMerge::FgOnly | TrackerMerge::FgOverBg | TrackerMerge::BgOverFg => Way::Unsteady,
        };
        self.solution(frame, size)
            .and_then(|solution| placing(&solution, way, size))
            .map_or(still, |(center, size, angle)| Tracked {
                center,
                size,
                angle,
                ..still
            })
    }

    /// What it publishes at `frame` of its keys as `output`, in a picture of `size` pixels: a
    /// place, as a [`Center`](super::Center) counts one.
    ///
    /// - `tracker`: where the pattern at `tracker` of its list is ([`Pattern::place_at`]), and
    ///   its offset: the Offset Position of the mathematics. A pattern that is off publishes
    ///   it all the same; none where there is no such pattern, or it has no path.
    /// - `steady_position` and `unsteady_position`: the middle of the picture less the shift
    ///   of the motion, and the middle and the shift, as the mathematics publishes them; none
    ///   where there is no motion to be had ([`Tracker::solution`]).
    ///
    /// They are published whatever the Operation and the Merge are.
    #[must_use]
    pub fn published(
        &self,
        output: FollowOutput,
        tracker: usize,
        frame: f64,
        size: (u32, u32),
    ) -> Option<[f64; 2]> {
        let place = match output {
            FollowOutput::Tracker => {
                let pattern = self.trackers.get(tracker)?;
                let [x, y] = pattern.place_at(frame)?;
                [x + pattern.offset.x, y + pattern.offset.y]
            }
            FollowOutput::SteadyPosition | FollowOutput::UnsteadyPosition => {
                let published = self.solution(frame, size)?.published(extent(size))?;
                if output == FollowOutput::SteadyPosition {
                    published.steady_position
                } else {
                    published.unsteady_position
                }
            }
        };
        place.iter().all(|at| at.is_finite()).then_some(place)
    }

    /// Check it: no more patterns than it may have, each within its bounds, and its pivot and
    /// its reference within theirs.
    pub(super) fn check(&self) -> Result<(), OpError> {
        if self.trackers.len() > MOST_TRACKERS {
            return Err(refused("tracker.trackers"));
        }
        for pattern in &self.trackers {
            pattern.check()?;
        }
        if self.pivot_tracker >= MOST_TRACKERS {
            return Err(refused("tracker.pivot_tracker"));
        }
        let pivot = [self.pivot_point.x, self.pivot_point.y];
        if !pivot.iter().all(|&at| within(bounds::TRACKER_PLACE, at)) {
            return Err(refused("tracker.pivot_point"));
        }
        if self.reference.unsigned_abs() > LARGEST.unsigned_abs() {
            return Err(refused("tracker.reference"));
        }
        Ok(())
    }

    /// It with each frame of the paths of its patterns, and its reference, at the frame `to`
    /// of its frame, within [`LARGEST`] of 0. Of the places of a path that come to one frame
    /// the last is kept.
    pub(super) fn with_key_frames(&self, to: &impl Fn(i64) -> i64) -> Tracker {
        let at = |frame: i64| to(frame).clamp(-LARGEST, LARGEST);
        let mut moved = self.clone();
        for pattern in &mut moved.trackers {
            pattern.path = pattern
                .path
                .iter()
                .map(|(&frame, &place)| (at(frame), place))
                .collect();
        }
        moved.reference = at(self.reference);
        moved
    }
}

/// How a picture of `size` pixels is put so that it is drawn through `solution` the way
/// `way`: where its middle is put, as a [`Center`](super::Center) counts it, how many times
/// as large it is made about its middle, and how far it is turned there, in degrees
/// counter-clockwise. None where the numbers are not finite.
///
/// The mathematics turns and sizes a picture about a pivot `c` and then shifts it:
/// `M(p) = c + t + s · R · (p − c)`. A Transform and a Merge turn and size a picture about
/// its middle `m`, and put the middle at their Center. The two are one map where the Center
/// is where `M` puts the middle, `M(m) = m + t + (s · R − 1) · (m − c)`, with the same size
/// and the same turn: a turn of the mathematics is clockwise on the screen, in radians, and
/// that of a Center counter-clockwise, in degrees. Held still, the map is the inverse one:
/// the opposite shift, turn and size about the pivot where it is at the frame.
fn placing(solution: &Solution, way: Way, size: (u32, u32)) -> Option<([f64; 2], f64, f64)> {
    let extent = extent(size);
    let motion = &solution.motion;
    let (shift, scale, turn, pivot) = match way {
        Way::Unsteady => (
            [motion.pan, motion.tilt],
            motion.zoom,
            motion.rotate,
            solution.pivot,
        ),
        Way::Steady => (
            [-motion.pan, -motion.tilt],
            1.0 / motion.zoom,
            -motion.rotate,
            [
                solution.pivot[0] + motion.pan,
                solution.pivot[1] + motion.tilt,
            ],
        ),
    };
    let middle = [extent.0 / 2.0, extent.1 / 2.0];
    // From the pivot to the middle, and that turned and sized: with no turn and a size of 1
    // the two are one, and the middle is shifted and no more, to the bit.
    let from = [middle[0] - pivot[0], middle[1] - pivot[1]];
    let (sin, cos) = turn.sin_cos();
    let turned = [
        scale * (from[0] * cos - from[1] * sin),
        scale * (from[0] * sin + from[1] * cos),
    ];
    let put = [
        middle[0] + shift[0] + (turned[0] - from[0]),
        middle[1] + shift[1] + (turned[1] - from[1]),
    ];
    let center = matchmove::to_fusion(put, extent)?;
    // Counter-clockwise, where the turn of the mathematics is clockwise; and no turn is 0,
    // not its negative.
    let angle = 0.0 - turn.to_degrees();
    let finite = center.iter().all(|at| at.is_finite()) && angle.is_finite();
    (finite && scale.is_finite() && scale > 0.0).then_some((center, scale, angle))
}

/// What a [`Tracker`] draws at one moment: its Operation and its Merge, and how the picture
/// that is moved is put, in the numbers by which a Transform puts its picture and a Merge its
/// foreground.
///
/// The picture that is moved is that of `input` with BG Only, which is held still, and that
/// of `foreground` with the others, which follows the patterns. Where the Operation is None,
/// and where there is no motion to be had at the frame, it lies as it is: in the middle, as
/// large as it is, and not turned.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tracked {
    /// Operation: with None the node is its `input`, whatever the rest.
    pub operation: TrackerOperation,
    /// Merge: which pictures are drawn.
    pub merge: TrackerMerge,
    /// Where the middle of the picture that is moved is put: `x` and `y`, as a
    /// [`Center`](super::Center) counts them.
    pub center: [f64; 2],
    /// How many times as large it is made, about its middle: above 0.
    pub size: f64,
    /// How far it is turned about its middle, in degrees counter-clockwise.
    pub angle: f64,
}

/// What a centre follows of a [`Tracker`]: one of the places that the node publishes
/// ([`Tracker::published`]). It is written `"tracker"`, `"steady_position"` or
/// `"unsteady_position"`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FollowOutput {
    /// Where one pattern is, and its offset (the Offset Position, pp. 1741, 1746): so that a
    /// thing follows the pattern.
    #[default]
    Tracker,
    /// Steady Position (p. 1745): the middle of the picture less the shift of the patterns
    /// since the reference frame.
    SteadyPosition,
    /// Unsteady Position (p. 1746): the middle of the picture and that shift.
    UnsteadyPosition,
}

/// That a [`Center`](super::Center) follows what a Tracker of its composition publishes (the
/// Connect To of a Center, pp. 1745–1748): the node, which of its places, and, of the place
/// of a pattern, which pattern.
///
/// It is written `{"node", "output", "tracker"}`. `tracker` is always written, and may be
/// left out where it is read: it is then 0. It is read of the output `tracker` alone, and
/// kept with the others.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Follow {
    /// The node that is followed: a Tracker of the same composition.
    pub node: Id,
    /// Which of the places that it publishes.
    pub output: FollowOutput,
    /// The pattern whose place is followed, by its place in the list of the node, from 0:
    /// below [`MOST_TRACKERS`].
    #[serde(default)]
    pub tracker: usize,
}

impl Follow {
    /// The place of pattern `tracker` of the node `node`.
    #[must_use]
    pub fn tracker(node: impl Into<Id>, tracker: usize) -> Self {
        Self {
            node: node.into(),
            output: FollowOutput::Tracker,
            tracker,
        }
    }

    /// The place `output` of the node `node`.
    #[must_use]
    pub fn of(node: impl Into<Id>, output: FollowOutput) -> Self {
        Self {
            node: node.into(),
            output,
            tracker: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::float_cmp,
        reason = "places of paths and shifts that floats hold whole are those numbers as written"
    )]

    use columbina_track::Mat3;
    use serde_json::json;

    use super::*;

    /// A picture that is not square, so that a share of its width and one of its height are
    /// not the same pixels.
    const SIZE: (u32, u32) = (64, 32);

    /// What is set of a Tracker for a case of a test.
    type Change = fn(&mut Tracker);

    /// A number of a Tracker that is refused: the name that it is refused by, and what sets
    /// it out of its bounds.
    type Refusal = (&'static str, Change);

    /// A pattern whose path has `places`, each at its frame.
    fn pattern(places: &[(i64, [f64; 2])]) -> Pattern {
        Pattern {
            path: places.iter().copied().collect(),
            ..Pattern::default()
        }
    }

    /// A Tracker that match moves, of `trackers`.
    fn tracker(trackers: Vec<Pattern>) -> Tracker {
        Tracker {
            operation: TrackerOperation::MatchMove,
            trackers,
            ..Tracker::default()
        }
    }

    /// The place, in the pixels of the mathematics, of the place `at` of a picture of
    /// [`SIZE`].
    fn pixels(at: [f64; 2]) -> [f64; 2] {
        matchmove::from_fusion(at, extent(SIZE)).unwrap()
    }

    /// Where two patterns are at frame 0.
    const START: [[f64; 2]; 2] = [[0.25, 0.75], [0.625, 0.375]];

    /// Where the place `at` of the picture goes by a turn of 30 degrees counter-clockwise on
    /// the screen and a size of 1.5 about a point of the picture, and a shift.
    fn gone(at: [f64; 2]) -> [f64; 2] {
        let (sin, cos) = (-30.0_f64).to_radians().sin_cos();
        let about = [20.0, 12.0];
        let [x, y] = pixels(at);
        let (x, y) = (x - about[0], y - about[1]);
        let to = [
            about[0] + 1.5 * (x * cos - y * sin) + 3.0,
            about[1] + 1.5 * (x * sin + y * cos) - 2.0,
        ];
        matchmove::to_fusion(to, extent(SIZE)).unwrap()
    }

    /// A Tracker that match moves, of two patterns that are at [`START`] at frame 0 and where
    /// [`gone`] puts them at frame 10, with what `set` sets of it.
    fn both(set: Change) -> Tracker {
        let mut two = tracker(
            START
                .iter()
                .map(|&at| pattern(&[(0, at), (10, gone(at))]))
                .collect(),
        );
        set(&mut two);
        two
    }

    /// The map of a picture of [`SIZE`] that is put as a Transform puts one: its middle at
    /// `center`, made `size` times as large about its middle and turned `angle` degrees
    /// counter-clockwise there; in the pixels of the mathematics.
    fn put(center: [f64; 2], size: f64, angle: f64) -> impl Fn([f64; 2]) -> [f64; 2] {
        let (width, height) = extent(SIZE);
        let middle = [width / 2.0, height / 2.0];
        let to = pixels(center);
        let (sin, cos) = angle.to_radians().sin_cos();
        move |[x, y]: [f64; 2]| {
            let (x, y) = (x - middle[0], y - middle[1]);
            // With y down, a turn counter-clockwise on the screen.
            [
                to[0] + size * (x * cos + y * sin),
                to[1] + size * (-x * sin + y * cos),
            ]
        }
    }

    /// How far apart what `map` and `matrix` make of some points of the picture are, at the
    /// most.
    fn apart(map: &impl Fn([f64; 2]) -> [f64; 2], matrix: &Mat3) -> f64 {
        [
            [0.0, 0.0],
            [64.0, 0.0],
            [10.5, 20.25],
            [32.0, 16.0],
            [64.0, 32.0],
        ]
        .into_iter()
        .map(|point| {
            let (one, other) = (map(point), matrix.apply(point));
            (one[0] - other[0]).hypot(one[1] - other[1])
        })
        .fold(0.0, f64::max)
    }

    #[test]
    fn a_tracker_is_written_with_every_setting_and_read_back() {
        let first = Tracker::default();
        assert_eq!(
            serde_json::to_value(&first).unwrap(),
            json!({
                "operation": "none", "merge": "bg_only", "position": true, "rotation": true,
                "scaling": true, "pivot": "tracker_average", "pivot_tracker": 0,
                "pivot_point": {"x": 0.5, "y": 0.5}, "reference": 0, "trackers": [],
            })
        );
        assert_eq!(first.check(), Ok(()));
        assert!(!first.has_paths());
        let mut whole = tracker(vec![
            pattern(&[(12, [0.5, 0.25]), (13, [0.5625, 0.25])]),
            Pattern::new(0.25, 0.75),
        ]);
        whole.trackers[0].name = "Eye".into();
        whole.trackers[0].offset = Place { x: 0.125, y: -0.25 };
        whole.trackers[1].enabled = false;
        whole.merge = TrackerMerge::FgOverBg;
        whole.pivot = TrackerPivot::Manual;
        whole.pivot_point = Place { x: 0.25, y: 0.75 };
        whole.pivot_tracker = 1;
        whole.reference = 12;
        let written = serde_json::to_value(&whole).unwrap();
        assert_eq!(
            written,
            json!({
                "operation": "match_move", "merge": "fg_over_bg", "position": true,
                "rotation": true, "scaling": true, "pivot": "manual", "pivot_tracker": 1,
                "pivot_point": {"x": 0.25, "y": 0.75}, "reference": 12,
                "trackers": [
                    {"name": "Eye", "enabled": true,
                     "pattern": {"x": 0.5, "y": 0.5, "width": 0.05, "height": 0.05},
                     "search": {"width": 0.125, "height": 0.125},
                     "offset": {"x": 0.125, "y": -0.25},
                     "path": {"12": [0.5, 0.25], "13": [0.5625, 0.25]}},
                    {"name": "", "enabled": false,
                     "pattern": {"x": 0.25, "y": 0.75, "width": 0.05, "height": 0.05},
                     "search": {"width": 0.125, "height": 0.125},
                     "offset": {"x": 0.0, "y": 0.0}, "path": {}},
                ],
            })
        );
        assert_eq!(serde_json::from_value::<Tracker>(written).unwrap(), whole);
        assert_eq!(whole.check(), Ok(()));
        assert!(whole.has_paths());
        // The frames of a path are written in their order, below 0 too.
        let early = pattern(&[(-2, [0.5, 0.5]), (10, [0.5, 0.5]), (9, [0.25, 0.5])]);
        assert_eq!(
            serde_json::to_string(&early.path).unwrap(),
            r#"{"-2":[0.5,0.5],"9":[0.25,0.5],"10":[0.5,0.5]}"#
        );
        // A frame that is no whole number, a place of one number, and a setting that a
        // Tracker has not cannot be read.
        for wrong in [
            json!({"1.5": [0.5, 0.5]}),
            json!({"first": [0.5, 0.5]}),
            json!({"1": [0.5]}),
            json!({"1": {"x": 0.5, "y": 0.5}}),
            json!([[0.5, 0.5]]),
        ] {
            let mut written = serde_json::to_value(pattern(&[])).unwrap();
            written["path"] = wrong.clone();
            assert!(
                serde_json::from_value::<Pattern>(written).is_err(),
                "{wrong}"
            );
        }
        let mut more = serde_json::to_value(Tracker::default()).unwrap();
        more["adaptive_mode"] = json!("none");
        assert!(serde_json::from_value::<Tracker>(more).is_err());
        for pivot in ["tracker_center", "selected_tracker(0)"] {
            let mut other = serde_json::to_value(Tracker::default()).unwrap();
            other["pivot"] = json!(pivot);
            assert!(serde_json::from_value::<Tracker>(other).is_err(), "{pivot}");
        }
    }

    #[test]
    fn a_pattern_is_on_the_line_between_the_frames_of_its_path_and_held_beyond_them() {
        let moving = pattern(&[(10, [0.25, 0.5]), (20, [0.5, 0.25]), (22, [0.75, 0.25])]);
        // Where the path has it.
        assert_eq!(moving.place_at(10.0), Some([0.25, 0.5]));
        assert_eq!(moving.place_at(20.0), Some([0.5, 0.25]));
        assert_eq!(moving.place_at(22.0), Some([0.75, 0.25]));
        // Between two frames of it, whole and not.
        assert_eq!(moving.place_at(15.0), Some([0.375, 0.375]));
        assert_eq!(moving.place_at(21.0), Some([0.625, 0.25]));
        assert_eq!(moving.place_at(12.5), Some([0.3125, 0.4375]));
        // Before its first frame and after its last.
        for frame in [9.0, 0.0, -1000.0, f64::NEG_INFINITY] {
            assert_eq!(moving.place_at(frame), Some([0.25, 0.5]), "{frame}");
        }
        for frame in [22.5, 23.0, 1e9, f64::INFINITY] {
            assert_eq!(moving.place_at(frame), Some([0.75, 0.25]), "{frame}");
        }
        // One frame is every frame; no path is no place; and a frame that is no number.
        let still = pattern(&[(7, [0.125, 0.875])]);
        for frame in [-3.0, 7.0, 7.5, 400.0] {
            assert_eq!(still.place_at(frame), Some([0.125, 0.875]), "{frame}");
        }
        assert_eq!(pattern(&[]).place_at(7.0), None);
        assert_eq!(moving.place_at(f64::NAN), None);
    }

    #[test]
    fn one_pattern_gives_a_shift_that_holds_the_picture_still_or_makes_another_follow() {
        // Four pixels to the right and two down by frame 4, and as far again by frame 8.
        let path = [(0, [0.25, 0.5]), (4, [0.3125, 0.4375]), (8, [0.375, 0.375])];
        let mut one = tracker(vec![pattern(&path)]);
        let moved = one.solution(4.0, SIZE).unwrap();
        assert_eq!((moved.motion.pan, moved.motion.tilt), (4.0, 2.0));
        assert_eq!((moved.motion.zoom, moved.motion.rotate), (1.0, 0.0));
        assert_eq!(moved.trackers, 1);
        // Held still, the picture is put as far the other way; `center.y` counts upwards.
        assert_eq!(
            one.at(4.0, SIZE),
            Tracked {
                operation: TrackerOperation::MatchMove,
                merge: TrackerMerge::BgOnly,
                center: [0.5 - 4.0 / 64.0, 0.5 + 2.0 / 32.0],
                size: 1.0,
                angle: 0.0,
            }
        );
        // The foreground follows, with each of the other three.
        for merge in [
            TrackerMerge::FgOnly,
            TrackerMerge::FgOverBg,
            TrackerMerge::BgOverFg,
        ] {
            one.merge = merge;
            let drawn = one.at(4.0, SIZE);
            assert_eq!(drawn.merge, merge);
            assert_eq!(drawn.center, [0.5 + 4.0 / 64.0, 0.5 - 2.0 / 32.0]);
            assert_eq!((drawn.size, drawn.angle), (1.0, 0.0));
        }
        // Between two frames of the path, on the line between them.
        assert_eq!(
            one.at(6.0, SIZE).center,
            [0.5 + 6.0 / 64.0, 0.5 - 3.0 / 32.0]
        );
        assert_eq!(
            one.at(2.5, SIZE).center,
            [0.5 + 2.5 / 64.0, 0.5 - 1.25 / 32.0]
        );
        // Before the first frame of the path the pattern is where it was first, and after the
        // last where it was last.
        assert_eq!(one.at(-5.0, SIZE).center, [0.5, 0.5]);
        assert_eq!(
            one.at(100.0, SIZE).center,
            [0.5 + 8.0 / 64.0, 0.5 - 4.0 / 32.0]
        );
        // At the reference frame nothing moves, wherever it is: on a frame of the path,
        // between two, and beyond them.
        for (reference, at) in [
            (4, [4.0, 2.0]),
            (6, [6.0, 3.0]),
            (50, [8.0, 4.0]),
            (-9, [0.0, 0.0]),
        ] {
            one.reference = reference;
            #[allow(clippy::cast_precision_loss, reason = "a small frame")]
            let still = one.at(reference as f64, SIZE);
            assert_eq!(still.center, [0.5, 0.5], "{reference}");
            // And the others are counted from it.
            let last = one.at(8.0, SIZE);
            assert_eq!(
                last.center,
                [0.5 + (8.0 - at[0]) / 64.0, 0.5 - (4.0 - at[1]) / 32.0],
                "{reference}"
            );
        }
        // With an Operation of None the picture lies as it is, and the motion is found all
        // the same.
        one.reference = 0;
        one.operation = TrackerOperation::None;
        let idle = one.at(4.0, SIZE);
        assert_eq!(idle.operation, TrackerOperation::None);
        assert_eq!((idle.center, idle.size, idle.angle), ([0.5, 0.5], 1.0, 0.0));
        assert_eq!(one.solution(4.0, SIZE), Some(moved));
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "the motion of two patterns, each way and with each control"
    )]
    fn two_patterns_give_a_shift_a_turn_and_a_size_as_the_mathematics_has_them() {
        let start = START;
        let two = both(|_| {});
        let solution = two.solution(10.0, SIZE).unwrap();
        assert_eq!(solution.trackers, 2);
        assert!((solution.motion.zoom - 1.5).abs() < 1e-9);
        assert!((solution.motion.rotate + 30.0_f64.to_radians()).abs() < 1e-9);
        // The pivot is the mean of where the patterns were at the reference frame.
        let (first, second) = (pixels(start[0]), pixels(start[1]));
        let mean = [first[0].midpoint(second[0]), first[1].midpoint(second[1])];
        assert!((solution.pivot[0] - mean[0]).abs() < 1e-9);
        assert!((solution.pivot[1] - mean[1]).abs() < 1e-9);
        // Held still: a picture put as the node says is drawn through the steady matrix of
        // the mathematics, and one that follows through the unsteady one.
        let held = two.at(10.0, SIZE);
        assert!((held.size - 1.0 / 1.5).abs() < 1e-9 && (held.angle + 30.0).abs() < 1e-9);
        let steady = solution.transform(Operation::MatchMove, Way::Steady);
        assert!(apart(&put(held.center, held.size, held.angle), &steady) < 1e-9);
        let follows = both(|two| two.merge = TrackerMerge::FgOverBg).at(10.0, SIZE);
        assert!((follows.size - 1.5).abs() < 1e-9 && (follows.angle - 30.0).abs() < 1e-9);
        let unsteady = solution.transform(Operation::MatchMove, Way::Unsteady);
        assert!(apart(&put(follows.center, follows.size, follows.angle), &unsteady) < 1e-9);
        // The pattern that was tracked is brought back to where it was at the reference
        // frame.
        let back = put(held.center, held.size, held.angle)(pixels(gone(start[0])));
        assert!((back[0] - first[0]).abs() < 1e-9 && (back[1] - first[1]).abs() < 1e-9);
        // Half way, on the lines between the places of the paths.
        let half = two.solution(5.0, SIZE).unwrap();
        let through = half.transform(Operation::MatchMove, Way::Unsteady);
        let between = two.trackers[0].place_at(5.0).unwrap();
        let there = through.apply(first);
        assert!((there[0] - pixels(between)[0]).abs() < 1e-9);
        assert!((there[1] - pixels(between)[1]).abs() < 1e-9);
        // A box that is off leaves its part as no motion; whatever the pivot, the picture is
        // put through the matrix that the mathematics gives for the same controls.
        let unturned = both(|two| two.rotation = false).at(10.0, SIZE);
        assert_eq!(unturned.angle, 0.0);
        assert!((unturned.size - 1.0 / 1.5).abs() < 1e-9);
        let unscaled = both(|two| two.scaling = false).at(10.0, SIZE);
        assert_eq!(unscaled.size, 1.0);
        assert!((unscaled.angle + 30.0).abs() < 1e-9);
        let unshifted = both(|two| two.position = false);
        let about_mean = unshifted.solution(10.0, SIZE).unwrap();
        assert_eq!((about_mean.motion.pan, about_mean.motion.tilt), (0.0, 0.0));
        let controls: [Change; 5] = [
            |two| two.pivot = TrackerPivot::SelectedTracker,
            |two| {
                two.pivot = TrackerPivot::SelectedTracker;
                two.pivot_tracker = 1;
            },
            |two| {
                two.pivot = TrackerPivot::Manual;
                two.pivot_point = Place { x: 0.75, y: 0.25 };
            },
            |two| two.position = false,
            |two| {
                two.position = false;
                two.pivot = TrackerPivot::Manual;
                two.pivot_point = Place { x: 0.0, y: 1.0 };
            },
        ];
        for (at, set) in controls.into_iter().enumerate() {
            let other = both(set);
            let found = other.solution(10.0, SIZE).unwrap();
            let held = other.at(10.0, SIZE);
            let steady = found.transform(Operation::MatchMove, Way::Steady);
            assert!(
                apart(&put(held.center, held.size, held.angle), &steady) < 1e-9,
                "{at}"
            );
        }
        let selected = both(|two| {
            two.pivot = TrackerPivot::SelectedTracker;
            two.pivot_tracker = 1;
        });
        assert_eq!(selected.solution(10.0, SIZE).unwrap().pivot, second);
        let manual = both(|two| {
            two.pivot = TrackerPivot::Manual;
            two.pivot_point = Place { x: 0.75, y: 0.25 };
        });
        assert_eq!(manual.solution(10.0, SIZE).unwrap().pivot, [48.0, 24.0]);
        // A pattern that the pivot names and that is not there is the tracker average.
        let beyond = both(|two| {
            two.pivot = TrackerPivot::SelectedTracker;
            two.pivot_tracker = 9;
        });
        assert_eq!(beyond.solution(10.0, SIZE), Some(solution));
    }

    #[test]
    fn a_pattern_that_is_off_or_has_no_path_is_left_out_of_the_motion() {
        let moving = pattern(&[(0, [0.25, 0.5]), (4, [0.3125, 0.5])]);
        let other = pattern(&[(0, [0.75, 0.5]), (4, [0.75, 0.25])]);
        let mut two = tracker(vec![moving.clone(), other]);
        assert_eq!(two.solution(4.0, SIZE).unwrap().trackers, 2);
        // The second off: the shift of the first alone.
        two.trackers[1].enabled = false;
        let alone = two.solution(4.0, SIZE).unwrap();
        assert_eq!(alone.trackers, 1);
        assert_eq!((alone.motion.pan, alone.motion.tilt), (4.0, 0.0));
        assert_eq!(
            Some(alone),
            tracker(vec![moving.clone()]).solution(4.0, SIZE)
        );
        // It keeps its place in the list: the pivot that names it is the tracker average.
        two.pivot = TrackerPivot::SelectedTracker;
        two.pivot_tracker = 1;
        assert_eq!(two.solution(4.0, SIZE), Some(alone));
        // One with no path is left out as one that is off is.
        let pathless = tracker(vec![moving, Pattern::new(0.75, 0.5)]);
        assert_eq!(pathless.solution(4.0, SIZE), Some(alone));
        // Both off, no patterns, and none with a path: no motion, and the picture as it is.
        two.trackers[0].enabled = false;
        for none in [two, tracker(vec![]), tracker(vec![Pattern::default()])] {
            assert_eq!(none.solution(4.0, SIZE), None);
            let drawn = none.at(4.0, SIZE);
            assert_eq!(
                (drawn.center, drawn.size, drawn.angle),
                ([0.5, 0.5], 1.0, 0.0)
            );
            for output in [FollowOutput::SteadyPosition, FollowOutput::UnsteadyPosition] {
                assert_eq!(none.published(output, 0, 4.0, SIZE), None);
            }
        }
        // Two patterns that were at one place at the reference frame tell no turn and no
        // size: the mathematics has no solution.
        let same = pattern(&[(0, [0.5, 0.5]), (4, [0.75, 0.5])]);
        let stacked = tracker(vec![same.clone(), same]);
        assert_eq!(stacked.solution(4.0, SIZE), None);
        assert_eq!(stacked.at(4.0, SIZE).center, [0.5, 0.5]);
    }

    #[test]
    fn what_a_tracker_publishes_is_what_the_mathematics_publishes() {
        let start = [[0.25, 0.75], [0.625, 0.375]];
        let ends = [[0.375, 0.5], [0.5, 0.25]];
        let mut two = tracker(vec![
            pattern(&[(0, start[0]), (8, ends[0])]),
            pattern(&[(0, start[1]), (8, ends[1])]),
        ]);
        two.trackers[1].offset = Place {
            x: 0.125,
            y: -0.0625,
        };
        let size = extent(SIZE);
        for frame in [0.0, 3.0, 8.0, 20.0] {
            let solution = two.solution(frame, SIZE).unwrap();
            let published = solution.published(size).unwrap();
            assert_eq!(
                two.published(FollowOutput::SteadyPosition, 0, frame, SIZE),
                Some(published.steady_position)
            );
            assert_eq!(
                two.published(FollowOutput::UnsteadyPosition, 7, frame, SIZE),
                Some(published.unsteady_position)
            );
            // The place of a pattern is its Offset Position: the offset is right and up here,
            // and right and down in the pixels of the mathematics.
            for (at, pattern) in two.trackers.iter().enumerate() {
                let centre = pixels(pattern.place_at(frame).unwrap());
                let path = [Some(centre)];
                let offset = [pattern.offset.x * size.0, -pattern.offset.y * size.1];
                let wanted = matchmove::offset_position(
                    &matchmove::Tracker {
                        path: &path,
                        offset,
                    },
                    0,
                    size,
                )
                .unwrap();
                let place = two
                    .published(FollowOutput::Tracker, at, frame, SIZE)
                    .unwrap();
                assert!((place[0] - wanted[0]).abs() < 1e-12, "{frame}: {place:?}");
                assert!((place[1] - wanted[1]).abs() < 1e-12, "{frame}: {place:?}");
            }
        }
        // At the reference frame the two positions are the middle, and a pattern is where
        // its path has it, to the bit.
        assert_eq!(
            two.published(FollowOutput::SteadyPosition, 0, 0.0, SIZE),
            Some([0.5, 0.5])
        );
        assert_eq!(
            two.published(FollowOutput::Tracker, 0, 8.0, SIZE),
            Some(ends[0])
        );
        assert_eq!(
            two.published(FollowOutput::Tracker, 1, 0.0, SIZE),
            Some([0.75, 0.3125])
        );
        // One pattern that goes a sixteenth of the width to the right: the manual's own case.
        let one = tracker(vec![pattern(&[(0, [0.5, 0.5]), (1, [0.5625, 0.5])])]);
        assert_eq!(
            one.published(FollowOutput::SteadyPosition, 0, 1.0, SIZE),
            Some([0.4375, 0.5])
        );
        assert_eq!(
            one.published(FollowOutput::UnsteadyPosition, 0, 1.0, SIZE),
            Some([0.5625, 0.5])
        );
        // They are published whatever the Operation, and by a pattern that is off; a pattern
        // that is not there, or has no path, publishes nothing.
        two.operation = TrackerOperation::None;
        two.trackers[0].enabled = false;
        assert_eq!(
            two.published(FollowOutput::Tracker, 0, 8.0, SIZE),
            Some(ends[0])
        );
        assert!(two
            .published(FollowOutput::UnsteadyPosition, 0, 8.0, SIZE)
            .is_some());
        assert_eq!(two.published(FollowOutput::Tracker, 2, 8.0, SIZE), None);
        two.trackers[0].path.clear();
        assert_eq!(two.published(FollowOutput::Tracker, 0, 8.0, SIZE), None);
    }

    #[test]
    fn the_numbers_of_a_tracker_are_within_their_bounds() {
        let whole = || tracker(vec![pattern(&[(0, [0.5, 0.5])]), Pattern::default()]);
        let refusals: [Refusal; 15] = [
            ("tracker.trackers", |tracker| {
                tracker.trackers = vec![Pattern::default(); MOST_TRACKERS + 1];
            }),
            ("tracker.trackers.name", |tracker| {
                tracker.trackers[1].name = "x".repeat(LONGEST_LABEL + 1);
            }),
            ("tracker.trackers.pattern", |tracker| {
                tracker.trackers[1].pattern.x = f64::NAN;
            }),
            ("tracker.trackers.pattern", |tracker| {
                tracker.trackers[1].pattern.y = 17.0;
            }),
            ("tracker.trackers.pattern", |tracker| {
                tracker.trackers[1].pattern.width = -0.1;
            }),
            ("tracker.trackers.pattern", |tracker| {
                tracker.trackers[1].pattern.height = 1.5;
            }),
            ("tracker.trackers.search", |tracker| {
                tracker.trackers[1].search.width = 1.25;
            }),
            ("tracker.trackers.search", |tracker| {
                tracker.trackers[1].search.height = f64::INFINITY;
            }),
            ("tracker.trackers.offset", |tracker| {
                tracker.trackers[1].offset.x = -16.5;
            }),
            ("tracker.trackers.offset", |tracker| {
                tracker.trackers[1].offset.y = f64::NAN;
            }),
            ("tracker.trackers.path", |tracker| {
                tracker.trackers[0].path.insert(3, [0.5, f64::NAN]);
            }),
            ("tracker.trackers.path", |tracker| {
                tracker.trackers[0].path.insert(LARGEST + 1, [0.5, 0.5]);
            }),
            ("tracker.pivot_tracker", |tracker| {
                tracker.pivot_tracker = MOST_TRACKERS;
            }),
            ("tracker.pivot_point", |tracker| {
                tracker.pivot_point.y = -16.0;
            }),
            ("tracker.reference", |tracker| {
                tracker.reference = -LARGEST - 1;
            }),
        ];
        for (what, change) in refusals {
            let mut changed = whole();
            change(&mut changed);
            assert_eq!(changed.check(), Err(OpError::OutOfRange { what }), "{what}");
        }
        assert_eq!(whole().check(), Ok(()));
        // The ends of each range are within it, and as many patterns as there may be.
        let mut ends = whole();
        ends.trackers = vec![Pattern::default(); MOST_TRACKERS];
        ends.trackers[0].name = "x".repeat(LONGEST_LABEL);
        ends.trackers[0].pattern = PatternArea {
            x: -15.5,
            y: 16.5,
            width: 0.0,
            height: 1.0,
        };
        ends.trackers[0].search = SearchArea {
            width: 1.0,
            height: 0.0,
        };
        ends.trackers[0].offset = Place { x: -16.0, y: 16.0 };
        ends.trackers[0].path = [(-LARGEST, [-15.5, 16.5]), (LARGEST, [16.5, -15.5])]
            .into_iter()
            .collect();
        ends.pivot_tracker = MOST_TRACKERS - 1;
        ends.pivot_point = Place { x: 16.5, y: -15.5 };
        ends.reference = LARGEST;
        assert_eq!(ends.check(), Ok(()));
    }

    #[test]
    fn the_frames_of_the_paths_and_the_reference_are_moved_with_the_keys() {
        let mut two = tracker(vec![
            pattern(&[(10, [0.25, 0.5]), (11, [0.375, 0.5]), (12, [0.5, 0.5])]),
            Pattern::default(),
        ]);
        two.reference = 11;
        let later = two.with_key_frames(&|frame| frame + 100);
        let frames =
            |tracker: &Tracker| tracker.trackers[0].path.keys().copied().collect::<Vec<_>>();
        assert_eq!(frames(&later), [110, 111, 112]);
        assert_eq!(later.reference, 111);
        assert_eq!(later.trackers[1], two.trackers[1]);
        // It draws later what it drew.
        assert_eq!(later.at(112.0, SIZE), two.at(12.0, SIZE));
        assert_eq!(later.check(), Ok(()));
        // Places that come to one frame are one, the last; and a frame goes no further than
        // the largest.
        let halved = two.with_key_frames(&|frame| frame / 2);
        assert_eq!(frames(&halved), [5, 6]);
        assert_eq!(halved.trackers[0].path[&5], [0.375, 0.5]);
        assert_eq!(halved.reference, 5);
        let far = two.with_key_frames(&|frame| frame.saturating_mul(i64::MAX / 11));
        assert_eq!(frames(&far), [LARGEST]);
        assert_eq!(far.reference, LARGEST);
        // Played backward, each place stays on its picture.
        let backward = two.with_key_frames(&|frame| 30 - frame);
        assert_eq!(frames(&backward), [18, 19, 20]);
        assert_eq!(backward.trackers[0].path[&18], [0.5, 0.5]);
        assert_eq!(backward.reference, 19);
    }

    #[test]
    fn a_follow_is_written_with_its_pattern_and_read_without_it() {
        let follow = Follow::tracker("n5", 2);
        assert_eq!(
            serde_json::to_value(&follow).unwrap(),
            json!({"node": "n5", "output": "tracker", "tracker": 2})
        );
        let steady = Follow::of("n5", FollowOutput::SteadyPosition);
        assert_eq!(
            serde_json::to_string(&steady).unwrap(),
            r#"{"node":"n5","output":"steady_position","tracker":0}"#
        );
        let short = json!({"node": "n5", "output": "unsteady_position"});
        assert_eq!(
            serde_json::from_value::<Follow>(short).unwrap(),
            Follow::of("n5", FollowOutput::UnsteadyPosition)
        );
        for wrong in [
            json!({"node": "n5"}),
            json!({"output": "tracker"}),
            json!({"node": "n5", "output": "steady_angle"}),
            json!({"node": "n5", "output": "tracker", "tracker": -1}),
            json!({"node": "n5", "output": "tracker", "input": "mask"}),
        ] {
            assert!(
                serde_json::from_value::<Follow>(wrong.clone()).is_err(),
                "{wrong}"
            );
        }
    }
}
