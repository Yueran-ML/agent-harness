//! Compositions: the graphs of nodes that make the picture of a clip, as the Fusion page of
//! `DaVinci Resolve` builds them (Reference Manual 20, pp. 1211–1274; the Merge node,
//! pp. 2162–2170).
//!
//! A clip of a track of picture may have a composition ([`crate::project::Clip::fusion`]): of
//! media, a title, a colour or a still. A clip without one shows its source as it always did,
//! and is written as it always was. A clip of sound has none.
//!
//! # Where it is in the picture
//!
//! The composition makes the picture of its clip: what comes out of its `MediaOut` is taken
//! for the source of the clip, before the stabilizer of the clip ([`crate::stabilize`],
//! [`Fused::stabilized`]), the grade of the clip and the transform, the crop and the composite
//! mode of the Edit page, which are done to it as they are to a source ("`MediaOut`'s alpha
//! returns to the Edit page", p. 1266). Its pictures are of the size of the source of the clip
//! (p. 1265): of media, its picture as it is shown upright; of a title or a colour, the
//! picture of the timeline ([`size_of`]).
//!
//! # Nodes and links
//!
//! A composition is a set of nodes and the links between them. A node has one output, but for
//! `MediaOut`, which has none; and the inputs of its kind, each known by its name
//! ([`Kind::inputs`]). A link goes from the output of a node into an input of another; an
//! output may feed any number of inputs, and an input takes at most one link. The links never
//! make a loop. The nodes are kept in the order in which they were added, and the links in the
//! order of the inputs that they go into: by the id of the node, and then by the name of the
//! input.
//!
//! The kinds of node ([`Kind`]):
//!
//! - `media_in`, no inputs: the picture of the clip's own source at the frame, as the engine
//!   reads it for a clip without a composition (along its speed points, and mixed of two
//!   pictures where its Retime Process says so). It is not stabilized: the stabilizer of the
//!   clip is done to what comes out of the composition. A composition may have several: they
//!   are the same picture.
//! - `media_out`, one input, `input`: the picture that goes back to the timeline. A
//!   composition has exactly one. Where its input is not linked the clip shows nothing: it is
//!   transparent.
//! - `merge`, the inputs `background` and `foreground`, and its effect mask, `mask`: the
//!   foreground laid over the background ([`Merge`]).
//! - `transform`, one input, `input`, and its effect mask, `mask`: its picture moved, made
//!   larger or smaller, turned and flipped ([`Transform`]).
//! - `background`, no input but its effect mask, `mask`: one colour over the whole picture
//!   ([`Background`]).
//! - `blur`, one input, `input`, and its effect mask, `mask`: its picture blurred ([`Blur`];
//!   see Blur and Mosaic below).
//! - `mosaic`, one input, `input`, and its effect mask, `mask`: its picture laid out in cells
//!   of one colour each ([`Mosaic`]).
//! - `rectangle`, `ellipse` and `polygon`, one input each, `mask`: the masks ([`Rectangle`],
//!   [`Ellipse`], [`Polygon`]; see Masks below).
//! - `delta_keyer`, the inputs `input`, `garbage_matte` and `solid_matte`, and its effect
//!   mask, `mask`: its picture keyed, the screen that it was shot against made transparent
//!   ([`DeltaKeyer`]; see The Delta Keyer below).
//! - `tracker`, the inputs `input` and `foreground`, and its effect mask, `mask`: the picture
//!   that was tracked held still, or another made to follow it ([`Tracker`]; see The Tracker
//!   below).
//!
//! An input that is not linked gives no picture: a transparent one of the size of the
//! composition; but a `mask` that is not linked is no mask at all (see Masks), and a
//! `garbage_matte` or a `solid_matte` that is not linked is no matte. A node that is
//! passed through (Fusion's Pass Through, Ctrl-P) hands on the picture of its first input as
//! it is ([`Kind::through`]): a Merge its background, a Transform, a Blur, a Mosaic, a Delta
//! Keyer and a Tracker their picture, and a mask node the mask that comes into it. A node
//! that has no such input, a `media_in` or a `background`, then gives none: the effect mask
//! of a node is not a picture that it hands on, and neither are the mattes of a Delta Keyer
//! nor the foreground of a Tracker.
//!
//! # Masks
//!
//! A Rectangle, an Ellipse and a Polygon each make a mask (Reference Manual 20, Chapter 108,
//! pp. 2386–2426): 1 where the shape is and 0 where it is not, with the controls that every
//! mask node has: Level, Soft Edge, Border Width, Invert, Solid and the Paint Mode. Their
//! settings are those of the mathematics of the masks of the compositor, by the same names,
//! in the same units, within the same ranges and with the same values at first; what each
//! makes of a pixel is told there.
//!
//! The links have no types: any output may go into any input. So that this holds of masks
//! (ours: Fusion keeps a mask as a picture of one channel):
//!
//! - What comes out of a mask node is a picture: white premultiplied by the value of the
//!   mask, so that its red, its green, its blue and its alpha are all that value. It may be
//!   the foreground of a Merge as any picture may.
//! - A `mask` input reads the alpha of the picture that is linked to it, whatever made it:
//!   a mask node, or the picture of a clip with its own alpha.
//! - A `mask` input that is not linked, or whose link brings no picture (it comes from a
//!   node that is passed through and has nothing to hand on), is **no mask**: the node is
//!   not limited. In [`Fused`] it is none.
//!
//! What a mask does to the node whose `mask` it comes into, which the compositor is held to:
//!
//! - A node that has a picture to hand on (a `transform`, a `blur`, a `mosaic`, a
//!   `delta_keyer` and a `tracker`, their `input`; a `merge`, its `background`): where the
//!   mask is 1 its result is its own, where
//!   it is 0 it is the picture of that first input as it came in (transparent where that is
//!   not linked), and between them the two are mixed by the value of the mask, channel by
//!   channel: `first + (own − first) · mask`. (A `mosaic` with Whole Cells Only reads its
//!   mask cell by cell: see Blur and Mosaic.)
//! - A `background`, which makes its picture of nothing: its picture multiplied by the
//!   mask, all four channels.
//! - A mask node: the mask that comes in is combined with its own by its Paint Mode
//!   (Merge, Add or Subtract). With nothing coming in there is no Paint Mode, and the mask
//!   is its own.
//!
//! A node that is read through a `mask` alone is still one that the picture depends on: it
//! is among the nodes of [`Fused`], and a loop through a `mask` is a loop.
//!
//! # Blur and Mosaic
//!
//! A Blur is Fusion's Blur node (Reference Manual 20, Chapter 93, pp. 2059–2061), with its
//! daily controls: the Filter, Lock X/Y, the Blur Size and Blend. Of its filters the Gaussian
//! and the Box are kept. It has no Clipping Mode: a picture is blurred within its own size,
//! and the pixels beyond its edges are those at its edges. The pictures are premultiplied, so
//! that the alpha of a picture is blurred with its colour.
//!
//! The Blur Size is counted as the filters of the Color page are (ours: the manual gives the
//! slider no unit), so that a number is the same blur on either page: of a Gaussian it is a
//! standard deviation and of a Box a radius, in pixels of a picture 1080 pixels high. In a
//! picture of another height it is as many pixels as that height is of 1080: a Blur Size of 5
//! is 5 pixels of a picture 1080 high and 10 of one 2160 high, the same share of either. With
//! Lock X/Y the blur across and the blur down are both `size.x`; `size.y` is kept as it is,
//! and is not read. What comes out is the picture as it came in mixed with the blurred one by
//! Blend, and by the effect mask where there is one: `input + (blurred − input) · blend ·
//! mask`.
//!
//! Fusion has no node of its own that makes a mosaic: it uses the Mosaic Blur of Resolve FX
//! (p. 3465). Ours is a node, `mosaic`, whose settings are those of the mathematics of the
//! mosaic of the compositor, by the same names, in the same units, within the same ranges and
//! with the same values at first; what each makes of a pixel is told there. Pixel Frequency
//! is how many cells lie across the width of the picture, Aspect how many times as wide as
//! high a cell is, and the offsets how many pixels of the picture the grid is moved, to the
//! right and down. The cells are squares, or rectangles by the Aspect: the other Cell Shapes
//! are not kept. The key of that mathematics is the effect mask of the node: each pixel is
//! `input + (cell − input) · mask`, `cell` the colour of its cell; and with Whole Cells Only
//! a cell is shown only where the mask is 1 over all of it, and every other cell is the
//! picture as it came in. Without a mask every cell is shown, and Whole Cells Only changes
//! nothing.
//!
//! # The Delta Keyer
//!
//! A Delta Keyer is Fusion's keyer of green and blue screens (Reference Manual 20,
//! pp. 2440–2448, and its use pp. 1664–1668), with the controls of its Key, Matte and Fringe
//! tabs that the mathematics of the keyer of the compositor has. Its settings are those of
//! that mathematics, by the same names, in the same units, within the same ranges and with
//! the same values at first; what each makes of a pixel is told there. The tabs of the
//! advanced tier (Pre Matte, Tuning, the shape and the colour of the fringe) are not kept.
//!
//! What is done to the picture that comes into `input`, in this order:
//!
//! 1. **The key.** The picture, blurred by Pre-Blur where that is above 0, is compared with
//!    the screen colour (Background Color), by Gain and Balance: the pre-matte, 1 where the
//!    picture is kept and 0 where it is the screen.
//! 2. **The matte.** The pre-matte goes through the thresholds (`low` and `high`), then
//!    Erode/Dilate, then the Matte Blur, then Clean Foreground and Clean Background.
//! 3. **The mattes that come in.** Where the solid matte is 1 the matte is made 1, and then
//!    where the garbage matte is 1 the matte is made 0: the garbage matte wins where both are
//!    1 (ours). With `garbage_invert` the garbage matte is read the other way round.
//! 4. **The result.** The screen is taken from the picture by the pre-matte, the spill is
//!    suppressed by Spill Method and Spill Suppression, and what the matte gave back is
//!    filled as Replace Mode says. The alpha is the matte.
//! 5. **The view.** `view` says what comes out: the pre-matte, the matte (as it is before the
//!    solid and garbage mattes), the status or the final result. It is a setting of the node,
//!    kept with the project, as Fusion keeps it.
//!
//! A `garbage_matte` and a `solid_matte` read the alpha of the picture that is linked to
//! them, as a `mask` does, whatever made it; one that is not linked, or whose link brings no
//! picture, is not there, and `garbage_invert` of no garbage matte is nothing. The effect
//! mask limits the node as it limits every node that hands a picture on.
//!
//! The lengths are counted as those of a Blur are: Pre-Blur and Matte Blur are the standard
//! deviations of Gaussians, and Erode/Dilate the radius of a box, in pixels of a picture 1080
//! pixels high. Erode/Dilate below 0 takes from the matte (erodes) and above 0 adds to it
//! (dilates).
//!
//! Two things are checked of numbers that have no keys, since keys cannot be checked at
//! every frame. The screen colour is to have one channel above its other two by
//! [`LEAST_DOMINANCE`]: with keys, at a frame where it has none the keyer keys nothing, and
//! every pixel is kept. And `low` is to be at most `high`: with keys, at a frame where they
//! have crossed the matte is a step at `low`, solid above it and transparent at it and below.
//!
//! # The Tracker
//!
//! A Tracker is Fusion's Tracker node (Reference Manual 20, Chapter 82, pp. 1724–1759, and
//! pp. 2773–2784), with its patterns and the Match Move of its Operation tab; Corner
//! Positioning and Perspective Positioning are not kept. The mathematics is that of
//! `columbina_track::matchmove`, which says what a motion is and is not written a second
//! time.
//!
//! **Its patterns.** A Tracker has a list of patterns ([`Pattern`]), at most
//! [`MOST_TRACKERS`], each with where it was put and how large it and its search area are,
//! its offset, whether it counts, and its **path**: where the middle of the pattern was found
//! at frames of the keys of the clip, each a place as a [`Center`] counts one. Following a
//! pattern through the pictures of a clip, which writes a path, is not done by the model: a
//! path is a setting like another, and is set with the list, whole. A frame that a path has
//! not was not tracked; the pattern is taken to be on the line between the frames about it
//! that were, and before the first of them where it was at that one, and after the last
//! where it was at that one ([`Pattern::place_at`]). A pattern that has no path, and one
//! that is off, is left out of the motion.
//!
//! **The motion.** At a frame the node has the motion of its patterns since the reference
//! frame ([`Tracker::solution`]): a shift alone of one pattern, and a shift, a turn and a size
//! of two or more, by the boxes Position, Rotation and Scaling, about its pivot. The
//! reference is a frame of the keys of the clip; each pattern is where its path has it
//! there, as at any frame, so that a reference before the first frame of the paths is that
//! frame. Where no pattern counts, and where the mathematics has no solution, there is no
//! motion.
//!
//! **What it draws** ([`Tracked`]), by its Operation and its Merge:
//!
//! - Operation `none`: the picture of its `input`, as it is, whatever else is set.
//! - `match_move` and `bg_only`: its `input` held still, drawn through the inverse of the
//!   motion, which brings each frame back to where it was at the reference frame.
//! - `fg_only`: its `foreground` drawn through the motion, alone, over nothing.
//! - `fg_over_bg`: that, laid over its `input` as it is (Normal, Over).
//! - `bg_over_fg`: its `input` as it is, laid over its `foreground` drawn through the motion.
//!
//! Where there is no motion at the frame the picture that would be moved lies as it is. The
//! effect mask limits the node as it limits every node that hands a picture on, the picture
//! of its `input` being the first: `first + (own − first) · mask`. The motion is a turn and
//! a size about a pivot and then a shift; the node hands it to the compositor as a Transform
//! and a Merge have theirs, a turn and a size about the middle of the picture and where the
//! middle is put, which is the same map.
//!
//! The picture depends on the size of the pictures of the composition ([`size_of`]), since
//! a turn is one of pixels and the settings are shares of the width and of the height: so
//! the composition at a frame is made for a size ([`Composition::at_size`]).
//!
//! # Following a Tracker
//!
//! A [`Center`] (of a Merge, of a Transform and of the masks) may follow what a Tracker of
//! its composition publishes ([`Follow`]), as a Center of Fusion is connected to one
//! (pp. 1745–1748): the place of one pattern with its offset, the Steady Position or the
//! Unsteady Position ([`Tracker::published`]). At a frame the centre is then that place,
//! held within the bounds of the centre, and its own `x` and `y` are kept and not read.
//! Where nothing is published at the frame (the pattern has no path, there is no motion to
//! be had, or the Tracker is passed through) the centre is its own `x` and `y`.
//!
//! What is published is made of the settings of the Tracker alone, and of no picture: so
//! following needs no link, the Tracker need not be a node that the `MediaOut` reads, and
//! the node that follows does not depend on it in [`Fused`], where its centre is the numbers
//! that it is at the frame, as that of any node. A Tracker has no centre, so that no
//! following goes round. The node that is followed is to be a Tracker of the same
//! composition, and a Tracker that is followed is not taken away: the centres that follow it
//! are to let go of it first.
//!
//! # Coordinates
//!
//! The settings are kept in Fusion's own units, so that a number is what `DaVinci` shows for
//! it and what a tracker gives ([`Center`]): a place is a share of the width and of the height
//! of the picture, from its lower left corner, `x` to the right and `y` up, so that the middle
//! is 0.5 and 0.5; a size is a factor, 1 as it is; an angle is in degrees, counter-clockwise.
//! (The transform of the Edit page counts from the middle, `y` down and clockwise: the two are
//! not the same.) The lengths of a mask are shares of the width of the picture, but for the
//! Height of a Rectangle, which is a share of its height (see [`Rectangle`], [`Ellipse`] and
//! [`Polygon`]).
//!
//! # Keys
//!
//! Each setting that is a number is a [`Param`], a number or keys. The keys count the frames
//! that the other keys of the clip count ([`crate::keyframes::KeyClock`]): those of its media,
//! or its own from its first for a title, a colour and a still. A setting is named by the id
//! of its node, a dot, and its path in the settings of the node: `n4.center.x`. So the id of a
//! node has no dot in it. A point of a Polygon is named by its place in the list of points,
//! from 0: `n7.points.2.x`. Its two numbers take keys each on its own, as those of a point of
//! a Polygon window of the Color page do; the list itself, how many points there are and in
//! which order, is set whole and takes no keys.
//!
//! No number of a Tracker takes keys: the paths of its patterns are what moves it. Their
//! frames, and its reference, are frames of the same count, and are moved with the keys
//! where the keys are moved ([`Composition::with_key_frames`]).

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::grade::{check_label, Bounds, Composite, Param, Pos};
use crate::ops::OpError;
use crate::project::{Clip, Id, Project, Source, Timeline};
use crate::stabilize::Matrix;

mod tracker;

pub use tracker::{
    Follow, FollowOutput, Pattern, PatternArea, Place, SearchArea, Tracked, Tracker, TrackerMerge,
    TrackerOperation, TrackerPivot, MOST_TRACKERS,
};

/// The most nodes of a composition. The manual gives none, and a grade has none: this is
/// ours, as many as a mixer of a grade has inputs at the most.
pub const MOST_NODES: usize = 256;

/// The id of the `MediaIn` of the composition that a clip has at first, as `DaVinci` names it.
pub const MEDIA_IN: &str = "MediaIn1";

/// The id of the `MediaOut` of the composition that a clip has at first, as `DaVinci` names it.
pub const MEDIA_OUT: &str = "MediaOut1";

/// The most points of a [`Polygon`]. The manual gives none: this is ours, as many as a
/// Polygon window of the Color page has at the most.
pub const MOST_POINTS: usize = crate::grade::window::MOST_POLYGON_POINTS;

/// The bounds of the settings of the nodes of a composition, in Fusion's units (see the
/// [module](self)). The manual gives no range of any of them but the Corner Radius of a
/// Rectangle: these are ours. Those of the masks are those of the mathematics of the masks
/// of the compositor, which is held to them.
pub mod bounds {
    use crate::grade::Bounds;

    /// Subtractive/Additive of a Merge (p. 2167): 1, Additive, at first.
    pub const SUBTRACTIVE_ADDITIVE: Bounds = Bounds::new(1.0, 0.0, 1.0);
    /// Alpha Gain of a Merge (p. 2168): 1 changes nothing.
    pub const ALPHA_GAIN: Bounds = Bounds::new(1.0, 0.0, 1.0);
    /// Burn In of a Merge (p. 2168): 0 changes nothing.
    pub const BURN_IN: Bounds = Bounds::new(0.0, 0.0, 1.0);
    /// Blend of a Merge (p. 2168): 1, the merged result, at first.
    pub const BLEND: Bounds = Bounds::new(1.0, 0.0, 1.0);
    /// Center X and Y (p. 2164): 0.5, the middle, at first; as far as 16 pictures either way.
    pub const CENTER: Bounds = Bounds::new(0.5, -15.5, 16.5);
    /// Size (p. 2164): 1 changes nothing, and 0 leaves nothing of the picture.
    pub const SIZE: Bounds = Bounds::new(1.0, 0.0, 16.0);
    /// Angle (p. 2164), in degrees counter-clockwise: 0 changes nothing; ten turns either way.
    pub const ANGLE: Bounds = Bounds::new(0.0, -3600.0, 3600.0);
    /// The red, the green and the blue of a Background: black at first.
    pub const COLOR: Bounds = Bounds::new(0.0, 0.0, 1.0);
    /// The alpha of a Background: opaque at first.
    pub const ALPHA: Bounds = Bounds::new(1.0, 0.0, 1.0);
    /// Level of a mask (p. 2397): 1, the mask as it is, at first.
    pub const MASK_LEVEL: Bounds = Bounds::new(1.0, 0.0, 1.0);
    /// Soft Edge of a mask (p. 2398), in widths of the picture: 0, a crisp edge, at first; up
    /// to a ramp as wide as the picture.
    pub const MASK_SOFT_EDGE: Bounds = Bounds::new(0.0, 0.0, 1.0);
    /// Border Width of a mask (p. 2398), in widths of the picture: 0, no border, at first; up
    /// to the width of the picture, either way.
    pub const MASK_BORDER_WIDTH: Bounds = Bounds::new(0.0, -1.0, 1.0);
    /// Center X and Y of a mask (pp. 2398, 2407, 2417): 0.5, the middle, at first; as far as
    /// ten pictures either way.
    pub const MASK_CENTER: Bounds = Bounds::new(0.5, -10.0, 10.0);
    /// Width and Height of a Rectangle and of an Ellipse (pp. 2398–2399, 2417): 0.5, half the
    /// picture, at first; from 0, a shape that covers nothing, to ten pictures.
    pub const MASK_EXTENT: Bounds = Bounds::new(0.5, 0.0, 10.0);
    /// Corner Radius of a Rectangle (p. 2417): from 0, sharp corners, which it has at first,
    /// to 1, the most rounding there is, both as the manual has them.
    pub const MASK_CORNER_RADIUS: Bounds = Bounds::new(0.0, 0.0, 1.0);
    /// Angle of a mask (pp. 2399, 2417; the Z Rotation of a Polygon, p. 2407), in degrees
    /// counter-clockwise: 0 at first; a hundred turns either way.
    pub const MASK_ANGLE: Bounds = Bounds::new(0.0, -36_000.0, 36_000.0);
    /// Size of a Polygon (p. 2407): 1, its points as they were drawn, at first; from 0 to 10.
    pub const MASK_SIZE: Bounds = Bounds::new(1.0, 0.0, 10.0);
    /// An `x` or a `y` of a point of a Polygon, from its centre, in widths of the picture: as
    /// far as 16 pictures either way.
    pub const MASK_POINT: Bounds = Bounds::new(0.0, -16.0, 16.0);
    /// Blur Size of a Blur, across and down (p. 2060), in pixels of a picture 1080 high: 5 at
    /// first, so that a new Blur is seen; 0 blurs nothing.
    pub const BLUR_SIZE: Bounds = Bounds::new(5.0, 0.0, 500.0);
    /// Blend of a Blur (p. 2061): 1, the blurred picture, at first; 0 is the picture as it
    /// came in.
    pub const BLUR_BLEND: Bounds = Bounds::new(1.0, 0.0, 1.0);
    /// Pixel Frequency of a Mosaic (p. 3465), how many cells lie across the picture: 32 at
    /// first; from 1 to the 16384 pixels of the widest picture. That of the mathematics of
    /// the mosaic of the compositor, which is held to it.
    pub const MOSAIC_FREQUENCY: Bounds = Bounds::new(32.0, 1.0, 16_384.0);
    /// Aspect of a Mosaic (p. 3465), the width of a cell against its height: 1, squares, at
    /// first; from a sixteenth to 16.
    pub const MOSAIC_ASPECT: Bounds = Bounds::new(1.0, 0.0625, 16.0);
    /// X Offset and Y Offset of a Mosaic (p. 3465), in pixels of the picture: none at first;
    /// as far either way as the widest picture is wide.
    pub const MOSAIC_OFFSET: Bounds = Bounds::new(0.0, -16_384.0, 16_384.0);
    /// The red and the blue of the Background Color of a Delta Keyer (p. 2443), the colour of
    /// the screen: none at first; from 0 to the largest value that a picture holds. Those of
    /// the mathematics of the keyer of the compositor, which is held to them, as the others of
    /// a Delta Keyer are.
    pub const KEYER_SCREEN: Bounds = Bounds::new(0.0, 0.0, 65_504.0);
    /// The green of the Background Color of a Delta Keyer: 1 at first, a pure green screen.
    pub const KEYER_SCREEN_GREEN: Bounds = Bounds::new(1.0, 0.0, 65_504.0);
    /// Gain of a Delta Keyer (p. 2443): 1 at first; 0 keys nothing.
    pub const KEYER_GAIN: Bounds = Bounds::new(1.0, 0.0, 10.0);
    /// Balance of a Delta Keyer (p. 2443): a half at first; from 0, the lesser of the other
    /// two channels, to 1, the greater.
    pub const KEYER_BALANCE: Bounds = Bounds::new(0.5, 0.0, 1.0);
    /// Pre-Blur (p. 2443) and Matte Blur (p. 2445) of a Delta Keyer, in pixels of a picture
    /// 1080 high: none at first.
    pub const KEYER_BLUR: Bounds = Bounds::new(0.0, 0.0, 100.0);
    /// The lower threshold of a Delta Keyer (p. 2445): 0 at first, which leaves the matte as
    /// the key made it.
    pub const KEYER_LOW: Bounds = Bounds::new(0.0, 0.0, 1.0);
    /// The upper threshold of a Delta Keyer (p. 2445): 1 at first.
    pub const KEYER_HIGH: Bounds = Bounds::new(1.0, 0.0, 1.0);
    /// Erode/Dilate of a Delta Keyer (p. 2445), in pixels of a picture 1080 high: none at
    /// first; below 0 it erodes the matte and above 0 it dilates it.
    pub const KEYER_ERODE_DILATE: Bounds = Bounds::new(0.0, -100.0, 100.0);
    /// Clean Foreground and Clean Background of a Delta Keyer (p. 2445): none at first.
    pub const KEYER_CLEAN: Bounds = Bounds::new(0.0, 0.0, 1.0);
    /// The red, the green and the blue of the Replace Color of a Delta Keyer (p. 2445): a mid
    /// grey at first.
    pub const KEYER_REPLACE_COLOR: Bounds = Bounds::new(0.5, 0.0, 65_504.0);
    /// Spill Suppression of a Delta Keyer (p. 2446): 1, all that the method takes, at first;
    /// 0 takes none.
    pub const KEYER_SPILL_SUPPRESSION: Bounds = Bounds::new(1.0, 0.0, 1.0);
    /// A place of a Tracker, across and up: where a pattern was put, a place of its path, and
    /// a pivot given by hand. 0.5, the middle, at first; as far as a Center X and Y may be,
    /// which follows one.
    pub const TRACKER_PLACE: Bounds = Bounds::new(0.5, -15.5, 16.5);
    /// The width and the height of a pattern of a Tracker (p. 2776), in widths and in heights
    /// of the picture: a twentieth at first; from 0 to the whole picture.
    pub const TRACKER_PATTERN: Bounds = Bounds::new(0.05, 0.0, 1.0);
    /// The width and the height of the search area of a pattern of a Tracker (p. 2776), in
    /// widths and in heights of the picture: an eighth at first; from 0 to the whole picture.
    pub const TRACKER_SEARCH: Bounds = Bounds::new(0.125, 0.0, 1.0);
    /// X Offset and Y Offset of a pattern of a Tracker (pp. 1741, 2777), in widths and in
    /// heights of the picture, right and up: none at first; as far as 16 pictures either way.
    pub const TRACKER_OFFSET: Bounds = Bounds::new(0.0, -16.0, 16.0);
}

// ---------------------------------------------------------------------------------------------
// Kinds, inputs and links
// ---------------------------------------------------------------------------------------------

/// The kinds of node, as they are named in `kind`. Another is to come: Text+.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Kind {
    /// The picture of the clip's own source.
    MediaIn,
    /// The picture that goes back to the timeline.
    MediaOut,
    /// A foreground laid over a background.
    Merge,
    /// A picture moved, made larger or smaller, turned and flipped.
    Transform,
    /// A colour over the whole picture.
    Background,
    /// The mask of a rectangle.
    Rectangle,
    /// The mask of an ellipse.
    Ellipse,
    /// The mask of a polygon.
    Polygon,
    /// A picture blurred.
    Blur,
    /// A picture laid out in cells of one colour each.
    Mosaic,
    /// A picture keyed: the screen that it was shot against made transparent.
    DeltaKeyer,
    /// The picture that was tracked held still, or another made to follow it.
    Tracker,
}

impl Kind {
    /// Its name, as it is written.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::MediaIn => "media_in",
            Self::MediaOut => "media_out",
            Self::Merge => "merge",
            Self::Transform => "transform",
            Self::Background => "background",
            Self::Rectangle => "rectangle",
            Self::Ellipse => "ellipse",
            Self::Polygon => "polygon",
            Self::Blur => "blur",
            Self::Mosaic => "mosaic",
            Self::DeltaKeyer => "delta_keyer",
            Self::Tracker => "tracker",
        }
    }

    /// The inputs that a node of the kind has: its pictures, and after them its effect mask,
    /// `mask`, which every kind has but `media_in` and `media_out`. A mask node has that one
    /// alone: the mask that comes into it. A Delta Keyer has its two mattes between its
    /// picture and its mask, and a Tracker its foreground.
    #[must_use]
    pub fn inputs(self) -> &'static [Input] {
        match self {
            Self::MediaIn => &[],
            Self::MediaOut => &[Input::Input],
            Self::Transform | Self::Blur | Self::Mosaic => &[Input::Input, Input::Mask],
            Self::Merge => &[Input::Background, Input::Foreground, Input::Mask],
            Self::Background | Self::Rectangle | Self::Ellipse | Self::Polygon => &[Input::Mask],
            Self::DeltaKeyer => &[
                Input::Input,
                Input::GarbageMatte,
                Input::SolidMatte,
                Input::Mask,
            ],
            Self::Tracker => &[Input::Input, Input::Foreground, Input::Mask],
        }
    }

    /// The input whose picture a node of the kind hands on where it is passed through: the
    /// first of its pictures, and of a mask node the mask that comes into it. None of a kind
    /// that makes its picture of nothing, `media_in` and `background`: the effect mask of a
    /// node is not a picture that it hands on.
    #[must_use]
    pub fn through(self) -> Option<Input> {
        match self {
            Self::MediaIn | Self::Background => None,
            Self::MediaOut
            | Self::Transform
            | Self::Blur
            | Self::Mosaic
            | Self::DeltaKeyer
            | Self::Tracker => Some(Input::Input),
            Self::Merge => Some(Input::Background),
            Self::Rectangle | Self::Ellipse | Self::Polygon => Some(Input::Mask),
        }
    }

    /// Whether a node of the kind has an output: every kind but `media_out`.
    #[must_use]
    pub fn has_output(self) -> bool {
        self != Self::MediaOut
    }

    /// Whether a node of the kind makes a mask: `rectangle`, `ellipse` and `polygon`.
    #[must_use]
    pub fn is_mask(self) -> bool {
        matches!(self, Self::Rectangle | Self::Ellipse | Self::Polygon)
    }
}

/// An input of a node, by its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Input {
    /// The one input of a node that has one picture: of `media_out`, of `transform`, of
    /// `blur` and of `mosaic`; the picture that a `delta_keyer` keys; and the picture that a
    /// `tracker` was tracked in (Fusion's Background of a Tracker).
    Input,
    /// The background of a Merge (the orange input of an editor).
    Background,
    /// The foreground of a Merge (the green one); and the picture that a `tracker` makes
    /// follow its patterns.
    Foreground,
    /// The garbage matte of a `delta_keyer`: where it is 1 the matte is made transparent. It
    /// reads the alpha of the picture that is linked to it; not linked, it is not there.
    GarbageMatte,
    /// The solid matte of a `delta_keyer`: where it is 1 the matte is made solid. It reads
    /// the alpha of the picture that is linked to it; not linked, it is not there.
    SolidMatte,
    /// The effect mask of a node (the blue input of an editor): the mask that limits a
    /// `merge`, a `transform`, a `background`, a `blur`, a `mosaic`, a `delta_keyer` or a
    /// `tracker`, and the mask that comes into a mask node. It reads the alpha of the picture
    /// that is linked to it; not linked, it is no mask.
    Mask,
}

impl Input {
    /// Its name, as it is written.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::Background => "background",
            Self::Foreground => "foreground",
            Self::GarbageMatte => "garbage_matte",
            Self::SolidMatte => "solid_matte",
            Self::Mask => "mask",
        }
    }
}

/// A link of a composition: from the output of the node `from` into the input `input` of the
/// node `to`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Link {
    /// The node that it comes from.
    pub from: Id,
    /// The node that it goes into.
    pub to: Id,
    /// The input of `to` that it goes into.
    pub input: Input,
}

impl Link {
    /// A link from `from` into `input` of `to`.
    #[must_use]
    pub fn new(from: impl Into<Id>, to: impl Into<Id>, input: Input) -> Self {
        Self {
            from: from.into(),
            to: to.into(),
            input,
        }
    }

    /// The input that it goes into, by which it is told from the other links of its
    /// composition.
    #[must_use]
    pub fn target(&self) -> (&str, Input) {
        (&self.to, self.input)
    }

    /// Whether it comes from or goes to `node`.
    #[must_use]
    pub fn touches(&self, node: &str) -> bool {
        self.from == node || self.to == node
    }

    /// What the links of a composition are put in the order of: the node that it goes into,
    /// and the name of the input.
    fn place(&self) -> (&str, &'static str) {
        (&self.to, self.input.name())
    }
}

// ---------------------------------------------------------------------------------------------
// The settings of the kinds
// ---------------------------------------------------------------------------------------------

/// A place in a picture, as Fusion counts it: `x` the share of its width from its left edge,
/// and `y` the share of its height from its lower edge, so that the middle is 0.5 and 0.5.
///
/// It may follow what a Tracker of its composition publishes (`follow`; see Following a
/// Tracker of the [module](self)): at a frame it is then that place, and its own `x` and `y`
/// are kept, checked and not read, but where nothing is published. `follow` is written only
/// where there is one, so that a centre that follows nothing is written `{"x", "y"}`, as it
/// always was; and it is read as none where it is left out or `null`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Center {
    /// How far right, as a share of the width.
    pub x: Param,
    /// How far up, as a share of the height.
    pub y: Param,
    /// What it follows of a Tracker, or none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub follow: Option<Follow>,
}

impl Center {
    /// The middle of the picture.
    fn middle() -> Self {
        Self {
            x: bounds::CENTER.param(),
            y: bounds::CENTER.param(),
            follow: None,
        }
    }

    /// Its own `x` and `y` at a frame of its keys, whatever it follows.
    fn at(&self, frame: f64) -> [f64; 2] {
        [self.x.value_at(frame), self.y.value_at(frame)]
    }

    fn check(&self, names: [&'static str; 2]) -> Result<(), OpError> {
        self.check_within(bounds::CENTER, names)
    }

    /// Check that its `x` and its `y` are within `bounds`, each by the name that it is
    /// refused by; and, where it follows a pattern, that the place of the pattern is one
    /// that a Tracker may have.
    fn check_within(&self, bounds: Bounds, names: [&'static str; 2]) -> Result<(), OpError> {
        bounds.check(names[0], &self.x)?;
        bounds.check(names[1], &self.y)?;
        let beyond = |follow: &Follow| follow.tracker >= MOST_TRACKERS;
        if self.follow.as_ref().is_some_and(beyond) {
            return Err(OpError::OutOfRange {
                what: "center.follow.tracker",
            });
        }
        Ok(())
    }
}

/// How the foreground and the background of a Merge are combined (Operator, pp. 2166–2167).
/// Over alone is done, as the mathematics of the Merge has it; the others are of the advanced
/// tier. It is written `"over"`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operator {
    /// The foreground over the background.
    #[default]
    Over,
}

/// The settings of a Merge (pp. 2162–2170): those that the mathematics of the Merge takes, by
/// the same names, and where the foreground lies over the background.
///
/// The Apply Modes are the composite modes of the timeline ([`Composite`]), which a Layer
/// Mixer has too: `normal` at first. The four sliders are each from 0 to 1.
/// Subtractive/Additive is read in the Normal mode alone. The foreground is put with its
/// middle at `center` of the background, made `size` times as large about its middle and
/// turned by `angle` there (p. 2164); the picture that comes out is of the size of the
/// background.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Merge {
    /// Apply Mode (pp. 2164–2166).
    pub apply: Composite,
    /// Operator (pp. 2166–2167).
    pub operator: Operator,
    /// Subtractive/Additive (p. 2167): from 0, Subtractive, to 1, Additive.
    pub subtractive_additive: Param,
    /// Alpha Gain (p. 2168): what the alpha of the foreground is multiplied by.
    pub alpha_gain: Param,
    /// Burn In (p. 2168): how much of the alpha is kept from darkening the background.
    pub burn_in: Param,
    /// Blend (p. 2168): from 0, the background as it is, to 1, the merged result.
    pub blend: Param,
    /// Center X and Y of the foreground (p. 2164), in the background.
    pub center: Center,
    /// Size of the foreground (p. 2164).
    pub size: Param,
    /// Angle of the foreground (p. 2164), in degrees counter-clockwise.
    pub angle: Param,
}

impl Default for Merge {
    /// The over of a premultiplied foreground, laid where it is.
    fn default() -> Self {
        Self {
            apply: Composite::Normal,
            operator: Operator::Over,
            subtractive_additive: bounds::SUBTRACTIVE_ADDITIVE.param(),
            alpha_gain: bounds::ALPHA_GAIN.param(),
            burn_in: bounds::BURN_IN.param(),
            blend: bounds::BLEND.param(),
            center: Center::middle(),
            size: bounds::SIZE.param(),
            angle: bounds::ANGLE.param(),
        }
    }
}

impl Merge {
    /// The four sliders that the mathematics of the Merge takes, each as a refusal names it
    /// and with its bounds, in the order of the Merge tab. The compositor is held to them.
    pub const SLIDERS: [(&'static str, Bounds); 4] = [
        ("merge.subtractive_additive", bounds::SUBTRACTIVE_ADDITIVE),
        ("merge.alpha_gain", bounds::ALPHA_GAIN),
        ("merge.burn_in", bounds::BURN_IN),
        ("merge.blend", bounds::BLEND),
    ];

    /// Its settings at a frame of its keys.
    #[must_use]
    pub fn at(&self, frame: f64) -> Merged {
        Merged {
            apply: self.apply,
            operator: self.operator,
            subtractive_additive: self.subtractive_additive.value_at(frame),
            alpha_gain: self.alpha_gain.value_at(frame),
            burn_in: self.burn_in.value_at(frame),
            blend: self.blend.value_at(frame),
            center: self.center.at(frame),
            size: self.size.value_at(frame),
            angle: self.angle.value_at(frame),
        }
    }

    fn sliders(&self) -> [&Param; 4] {
        [
            &self.subtractive_additive,
            &self.alpha_gain,
            &self.burn_in,
            &self.blend,
        ]
    }

    fn check(&self) -> Result<(), OpError> {
        for ((what, bounds), slider) in Self::SLIDERS.into_iter().zip(self.sliders()) {
            bounds.check(what, slider)?;
        }
        self.center.check(["merge.center.x", "merge.center.y"])?;
        bounds::SIZE.check("merge.size", &self.size)?;
        bounds::ANGLE.check("merge.angle", &self.angle)
    }

    fn settings(&self) -> [(&'static str, &Param); 8] {
        [
            ("subtractive_additive", &self.subtractive_additive),
            ("alpha_gain", &self.alpha_gain),
            ("burn_in", &self.burn_in),
            ("blend", &self.blend),
            ("center.x", &self.center.x),
            ("center.y", &self.center.y),
            ("size", &self.size),
            ("angle", &self.angle),
        ]
    }

    fn settings_mut(&mut self) -> [(&'static str, &mut Param); 8] {
        [
            ("subtractive_additive", &mut self.subtractive_additive),
            ("alpha_gain", &mut self.alpha_gain),
            ("burn_in", &mut self.burn_in),
            ("blend", &mut self.blend),
            ("center.x", &mut self.center.x),
            ("center.y", &mut self.center.y),
            ("size", &mut self.size),
            ("angle", &mut self.angle),
        ]
    }
}

/// The settings of a Transform node, those of the daily tier: its picture is put with its
/// middle at `center`, made `size` times as large about its middle, turned by `angle` there,
/// and flipped; the picture that comes out is of the size that came in.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Transform {
    /// Center X and Y.
    pub center: Center,
    /// Size.
    pub size: Param,
    /// Angle, in degrees counter-clockwise.
    pub angle: Param,
    /// Flip Horizontally: left for right.
    pub flip_horizontal: bool,
    /// Flip Vertically: top for bottom.
    pub flip_vertical: bool,
}

impl Default for Transform {
    /// The picture as it is.
    fn default() -> Self {
        Self {
            center: Center::middle(),
            size: bounds::SIZE.param(),
            angle: bounds::ANGLE.param(),
            flip_horizontal: false,
            flip_vertical: false,
        }
    }
}

impl Transform {
    /// Its settings at a frame of its keys.
    #[must_use]
    pub fn at(&self, frame: f64) -> Transformed {
        Transformed {
            center: self.center.at(frame),
            size: self.size.value_at(frame),
            angle: self.angle.value_at(frame),
            flip_horizontal: self.flip_horizontal,
            flip_vertical: self.flip_vertical,
        }
    }

    fn check(&self) -> Result<(), OpError> {
        self.center
            .check(["transform.center.x", "transform.center.y"])?;
        bounds::SIZE.check("transform.size", &self.size)?;
        bounds::ANGLE.check("transform.angle", &self.angle)
    }

    fn settings(&self) -> [(&'static str, &Param); 4] {
        [
            ("center.x", &self.center.x),
            ("center.y", &self.center.y),
            ("size", &self.size),
            ("angle", &self.angle),
        ]
    }

    fn settings_mut(&mut self) -> [(&'static str, &mut Param); 4] {
        [
            ("center.x", &mut self.center.x),
            ("center.y", &mut self.center.y),
            ("size", &mut self.size),
            ("angle", &mut self.angle),
        ]
    }
}

/// The colour of a Background, as Fusion keeps one: its red, its green, its blue and its
/// alpha, each from 0 to 1 and each a number or keys. The colour is not premultiplied by the
/// alpha: the compositor does that.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Color {
    /// Red.
    pub r: Param,
    /// Green.
    pub g: Param,
    /// Blue.
    pub b: Param,
    /// Alpha: from 0, not seen, to 1.
    pub a: Param,
}

/// The settings of a Background node: one colour over the whole picture (its Solid type; the
/// gradients are not kept). It is opaque black at first, as `DaVinci` makes one.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Background {
    /// The colour.
    pub color: Color,
}

impl Default for Background {
    fn default() -> Self {
        Self {
            color: Color {
                r: bounds::COLOR.param(),
                g: bounds::COLOR.param(),
                b: bounds::COLOR.param(),
                a: bounds::ALPHA.param(),
            },
        }
    }
}

impl Background {
    /// Its colour at a frame of its keys: red, green, blue and alpha, not premultiplied.
    #[must_use]
    pub fn at(&self, frame: f64) -> [f64; 4] {
        let Color { r, g, b, a } = &self.color;
        [r, g, b, a].map(|channel| channel.value_at(frame))
    }

    fn check(&self) -> Result<(), OpError> {
        let Color { r, g, b, a } = &self.color;
        bounds::COLOR.check("background.color.r", r)?;
        bounds::COLOR.check("background.color.g", g)?;
        bounds::COLOR.check("background.color.b", b)?;
        bounds::ALPHA.check("background.color.a", a)
    }

    fn settings(&self) -> [(&'static str, &Param); 4] {
        let Color { r, g, b, a } = &self.color;
        [
            ("color.r", r),
            ("color.g", g),
            ("color.b", b),
            ("color.a", a),
        ]
    }

    fn settings_mut(&mut self) -> [(&'static str, &mut Param); 4] {
        let Color { r, g, b, a } = &mut self.color;
        [
            ("color.r", r),
            ("color.g", g),
            ("color.b", b),
            ("color.a", a),
        ]
    }
}

// ---------------------------------------------------------------------------------------------
// The masks
// ---------------------------------------------------------------------------------------------

/// How the mask of a node is combined with the mask that comes into it (Paint Mode, p. 2398):
/// the three that the mathematics of the masks has. With `m` the value of the mask of the
/// node and `i` that of the incoming one, both from 0 to 1. It is written `"merge"`, `"add"`
/// or `"subtract"`, takes no keys, and is read only where a mask comes in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaintMode {
    /// The mask is laid over the incoming one: `i + m · (1 − i)`. Every mask has it at first.
    #[default]
    Merge,
    /// The values of the mask are added to those of the incoming one: `min(i + m, 1)`.
    Add,
    /// The values of the mask are taken from those of the incoming one: `max(i − m, 0)`.
    Subtract,
}

/// Which of the regions that the sides of a Polygon part are inside it (Fill Method,
/// p. 2407). It is written `"alternate"` or `"non_zero_winding"`, and takes no keys.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FillMethod {
    /// A region is inside where a line from it to far away crosses the sides an odd number of
    /// times, so that where the polygon lies over itself there is a hole.
    #[default]
    Alternate,
    /// A region is inside where the sides go around it at all: no hole where the polygon lies
    /// over itself.
    NonZeroWinding,
}

/// The controls that every mask has, as they are at a frame of their keys: `numbers` its
/// Level, its Soft Edge and its Border Width.
fn masked(
    numbers: [&Param; 3],
    invert: bool,
    solid: bool,
    paint_mode: PaintMode,
    frame: f64,
) -> Masked {
    let [level, soft_edge, border_width] = numbers.map(|setting| setting.value_at(frame));
    Masked {
        level,
        soft_edge,
        border_width,
        invert,
        solid,
        paint_mode,
    }
}

/// Check the Level, the Soft Edge and the Border Width of a mask, in that order, each by the
/// name that it is refused by.
fn check_common(names: [&'static str; 3], numbers: [&Param; 3]) -> Result<(), OpError> {
    let all = [
        bounds::MASK_LEVEL,
        bounds::MASK_SOFT_EDGE,
        bounds::MASK_BORDER_WIDTH,
    ];
    for ((what, range), number) in names.into_iter().zip(all).zip(numbers) {
        range.check(what, number)?;
    }
    Ok(())
}

/// The settings of a Rectangle mask (pp. 2414–2417): the controls that every mask has, and
/// its shape.
///
/// Level is what the mask is multiplied by. Soft Edge is how wide the ramp across the edge
/// is, and Border Width how thick the line along the edge is, both in widths of the picture:
/// solid, the border is added to the shape, or taken from it below 0; not solid, the border
/// is all that is drawn. Invert makes the mask 1 where the shape is not. Its middle is at
/// `center`; its Width is a share of the width of the picture and its Height a share of its
/// height, so that a Width and a Height of 1 are the whole picture; Corner Radius is from 0,
/// sharp corners, to 1, the most rounding there is; and it is turned by `angle` about its
/// middle.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rectangle {
    /// Level (p. 2397): from 0 to 1.
    pub level: Param,
    /// Soft Edge (p. 2398), in widths of the picture.
    pub soft_edge: Param,
    /// Border Width (p. 2398), in widths of the picture, either way.
    pub border_width: Param,
    /// Invert: the mask is 1 where the shape is not.
    pub invert: bool,
    /// Solid: the shape is filled. Without it only its border is drawn.
    pub solid: bool,
    /// Paint Mode: how the mask is combined with the one that comes in.
    pub paint_mode: PaintMode,
    /// Center X and Y: where its middle is.
    pub center: Center,
    /// Width, in widths of the picture.
    pub width: Param,
    /// Height, in heights of the picture.
    pub height: Param,
    /// Corner Radius: from 0, sharp corners, to 1, the short sides half circles.
    pub corner_radius: Param,
    /// Angle, in degrees counter-clockwise.
    pub angle: Param,
}

impl Default for Rectangle {
    /// In the middle of the picture, half of it wide and high, with sharp corners and not
    /// turned: solid and whole, with a crisp edge and no border, not inverted, merged with
    /// the mask that comes in.
    fn default() -> Self {
        Self {
            level: bounds::MASK_LEVEL.param(),
            soft_edge: bounds::MASK_SOFT_EDGE.param(),
            border_width: bounds::MASK_BORDER_WIDTH.param(),
            invert: false,
            solid: true,
            paint_mode: PaintMode::Merge,
            center: Center::middle(),
            width: bounds::MASK_EXTENT.param(),
            height: bounds::MASK_EXTENT.param(),
            corner_radius: bounds::MASK_CORNER_RADIUS.param(),
            angle: bounds::MASK_ANGLE.param(),
        }
    }
}

impl Rectangle {
    /// Its shape at a frame of its keys.
    #[must_use]
    pub fn at(&self, frame: f64) -> RectangleShape {
        RectangleShape {
            center: self.center.at(frame),
            width: self.width.value_at(frame),
            height: self.height.value_at(frame),
            corner_radius: self.corner_radius.value_at(frame),
            angle: self.angle.value_at(frame),
        }
    }

    /// The controls that every mask has, as its own are at a frame of its keys.
    #[must_use]
    pub fn common_at(&self, frame: f64) -> Masked {
        let numbers = [&self.level, &self.soft_edge, &self.border_width];
        masked(numbers, self.invert, self.solid, self.paint_mode, frame)
    }

    fn check(&self) -> Result<(), OpError> {
        check_common(
            [
                "rectangle.level",
                "rectangle.soft_edge",
                "rectangle.border_width",
            ],
            [&self.level, &self.soft_edge, &self.border_width],
        )?;
        let center = ["rectangle.center.x", "rectangle.center.y"];
        self.center.check_within(bounds::MASK_CENTER, center)?;
        bounds::MASK_EXTENT.check("rectangle.width", &self.width)?;
        bounds::MASK_EXTENT.check("rectangle.height", &self.height)?;
        bounds::MASK_CORNER_RADIUS.check("rectangle.corner_radius", &self.corner_radius)?;
        bounds::MASK_ANGLE.check("rectangle.angle", &self.angle)
    }

    fn settings(&self) -> [(&'static str, &Param); 9] {
        [
            ("level", &self.level),
            ("soft_edge", &self.soft_edge),
            ("border_width", &self.border_width),
            ("center.x", &self.center.x),
            ("center.y", &self.center.y),
            ("width", &self.width),
            ("height", &self.height),
            ("corner_radius", &self.corner_radius),
            ("angle", &self.angle),
        ]
    }

    fn settings_mut(&mut self) -> [(&'static str, &mut Param); 9] {
        [
            ("level", &mut self.level),
            ("soft_edge", &mut self.soft_edge),
            ("border_width", &mut self.border_width),
            ("center.x", &mut self.center.x),
            ("center.y", &mut self.center.y),
            ("width", &mut self.width),
            ("height", &mut self.height),
            ("corner_radius", &mut self.corner_radius),
            ("angle", &mut self.angle),
        ]
    }
}

/// The settings of an Ellipse mask (pp. 2396–2399): the controls that every mask has, as a
/// [`Rectangle`] has them, and its shape. Its middle is at `center`; its Width and its Height
/// are both shares of the width of the picture, so that a Width and a Height alike are a
/// circle in a picture of any shape; and it is turned by `angle` about its middle.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ellipse {
    /// Level (p. 2397): from 0 to 1.
    pub level: Param,
    /// Soft Edge (p. 2398), in widths of the picture.
    pub soft_edge: Param,
    /// Border Width (p. 2398), in widths of the picture, either way.
    pub border_width: Param,
    /// Invert: the mask is 1 where the shape is not.
    pub invert: bool,
    /// Solid: the shape is filled. Without it only its border is drawn.
    pub solid: bool,
    /// Paint Mode: how the mask is combined with the one that comes in.
    pub paint_mode: PaintMode,
    /// Center X and Y: where its middle is.
    pub center: Center,
    /// Width, in widths of the picture.
    pub width: Param,
    /// Height, in widths of the picture too.
    pub height: Param,
    /// Angle, in degrees counter-clockwise.
    pub angle: Param,
}

impl Default for Ellipse {
    /// In the middle of the picture, a circle half the picture wide, not turned: solid and
    /// whole, with a crisp edge and no border, not inverted, merged with the mask that comes
    /// in.
    fn default() -> Self {
        Self {
            level: bounds::MASK_LEVEL.param(),
            soft_edge: bounds::MASK_SOFT_EDGE.param(),
            border_width: bounds::MASK_BORDER_WIDTH.param(),
            invert: false,
            solid: true,
            paint_mode: PaintMode::Merge,
            center: Center::middle(),
            width: bounds::MASK_EXTENT.param(),
            height: bounds::MASK_EXTENT.param(),
            angle: bounds::MASK_ANGLE.param(),
        }
    }
}

impl Ellipse {
    /// Its shape at a frame of its keys.
    #[must_use]
    pub fn at(&self, frame: f64) -> EllipseShape {
        EllipseShape {
            center: self.center.at(frame),
            width: self.width.value_at(frame),
            height: self.height.value_at(frame),
            angle: self.angle.value_at(frame),
        }
    }

    /// The controls that every mask has, as its own are at a frame of its keys.
    #[must_use]
    pub fn common_at(&self, frame: f64) -> Masked {
        let numbers = [&self.level, &self.soft_edge, &self.border_width];
        masked(numbers, self.invert, self.solid, self.paint_mode, frame)
    }

    fn check(&self) -> Result<(), OpError> {
        check_common(
            ["ellipse.level", "ellipse.soft_edge", "ellipse.border_width"],
            [&self.level, &self.soft_edge, &self.border_width],
        )?;
        let center = ["ellipse.center.x", "ellipse.center.y"];
        self.center.check_within(bounds::MASK_CENTER, center)?;
        bounds::MASK_EXTENT.check("ellipse.width", &self.width)?;
        bounds::MASK_EXTENT.check("ellipse.height", &self.height)?;
        bounds::MASK_ANGLE.check("ellipse.angle", &self.angle)
    }

    fn settings(&self) -> [(&'static str, &Param); 8] {
        [
            ("level", &self.level),
            ("soft_edge", &self.soft_edge),
            ("border_width", &self.border_width),
            ("center.x", &self.center.x),
            ("center.y", &self.center.y),
            ("width", &self.width),
            ("height", &self.height),
            ("angle", &self.angle),
        ]
    }

    fn settings_mut(&mut self) -> [(&'static str, &mut Param); 8] {
        [
            ("level", &mut self.level),
            ("soft_edge", &mut self.soft_edge),
            ("border_width", &mut self.border_width),
            ("center.x", &mut self.center.x),
            ("center.y", &mut self.center.y),
            ("width", &mut self.width),
            ("height", &mut self.height),
            ("angle", &mut self.angle),
        ]
    }
}

/// A point of a [`Polygon`], from its centre: `x` to the right and `y` up, both in widths of
/// the picture, so that turning the polygon does not shear it. Each is a number or keys.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Point {
    /// How far right of the centre, in widths of the picture.
    pub x: Param,
    /// How far above the centre, in widths of the picture.
    pub y: Param,
}

impl Point {
    /// A point that is at one place on every frame.
    #[must_use]
    pub fn new(x: f64, y: f64) -> Self {
        Self {
            x: x.into(),
            y: y.into(),
        }
    }
}

/// The settings of a Polygon mask (pp. 2404–2408): the controls that every mask has, as a
/// [`Rectangle`] has them, and its shape, a closed outline of straight sides from each point
/// to the next and from the last to the first.
///
/// The points are counted from `center`, multiplied by `size` and turned by `angle` about
/// it. There are at most [`MOST_POINTS`]; fewer than three have no outline, and are a mask
/// of nothing, as a polygon that is being drawn is. Each number of a point takes keys on its
/// own (`points.2.x`); the list is set whole, so that it has as many points at every frame.
/// The Bezier handles of the points of Fusion's polyline are not kept.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Polygon {
    /// Level (p. 2397): from 0 to 1.
    pub level: Param,
    /// Soft Edge (p. 2398), in widths of the picture.
    pub soft_edge: Param,
    /// Border Width (p. 2398), in widths of the picture, either way.
    pub border_width: Param,
    /// Invert: the mask is 1 where the shape is not.
    pub invert: bool,
    /// Solid: the shape is filled. Without it only its border is drawn.
    pub solid: bool,
    /// Paint Mode: how the mask is combined with the one that comes in.
    pub paint_mode: PaintMode,
    /// Center X and Y: the place that its points are counted from.
    pub center: Center,
    /// Size: what its points are multiplied by.
    pub size: Param,
    /// Z Rotation, in degrees counter-clockwise about the centre.
    pub angle: Param,
    /// Its points, in the order of its outline.
    pub points: Vec<Point>,
    /// Fill Method.
    pub fill: FillMethod,
}

impl Default for Polygon {
    /// No points, about the middle of the picture, at the size that they are drawn and not
    /// turned, with the Alternate fill: solid and whole, with a crisp edge and no border, not
    /// inverted, merged with the mask that comes in.
    fn default() -> Self {
        Self {
            level: bounds::MASK_LEVEL.param(),
            soft_edge: bounds::MASK_SOFT_EDGE.param(),
            border_width: bounds::MASK_BORDER_WIDTH.param(),
            invert: false,
            solid: true,
            paint_mode: PaintMode::Merge,
            center: Center::middle(),
            size: bounds::MASK_SIZE.param(),
            angle: bounds::MASK_ANGLE.param(),
            points: Vec::new(),
            fill: FillMethod::Alternate,
        }
    }
}

impl Polygon {
    /// Its shape at a frame of its keys.
    #[must_use]
    pub fn at(&self, frame: f64) -> PolygonShape {
        PolygonShape {
            center: self.center.at(frame),
            size: self.size.value_at(frame),
            angle: self.angle.value_at(frame),
            points: self
                .points
                .iter()
                .map(|point| [point.x.value_at(frame), point.y.value_at(frame)])
                .collect(),
            fill: self.fill,
        }
    }

    /// The controls that every mask has, as its own are at a frame of its keys.
    #[must_use]
    pub fn common_at(&self, frame: f64) -> Masked {
        let numbers = [&self.level, &self.soft_edge, &self.border_width];
        masked(numbers, self.invert, self.solid, self.paint_mode, frame)
    }

    fn check(&self) -> Result<(), OpError> {
        check_common(
            ["polygon.level", "polygon.soft_edge", "polygon.border_width"],
            [&self.level, &self.soft_edge, &self.border_width],
        )?;
        let center = ["polygon.center.x", "polygon.center.y"];
        self.center.check_within(bounds::MASK_CENTER, center)?;
        bounds::MASK_SIZE.check("polygon.size", &self.size)?;
        bounds::MASK_ANGLE.check("polygon.angle", &self.angle)?;
        if self.points.len() > MOST_POINTS {
            return Err(OpError::OutOfRange {
                what: "polygon.points",
            });
        }
        for point in &self.points {
            bounds::MASK_POINT.check("polygon.points.x", &point.x)?;
            bounds::MASK_POINT.check("polygon.points.y", &point.y)?;
        }
        Ok(())
    }

    /// Its settings that take keys, each with its path: those that every polygon has, and
    /// the two numbers of each of its points after them, by the place of the point.
    fn settings(&self) -> Vec<(Cow<'static, str>, &Param)> {
        let mut all = named([
            ("level", &self.level),
            ("soft_edge", &self.soft_edge),
            ("border_width", &self.border_width),
            ("center.x", &self.center.x),
            ("center.y", &self.center.y),
            ("size", &self.size),
            ("angle", &self.angle),
        ]);
        for (at, point) in self.points.iter().enumerate() {
            all.push((Cow::Owned(format!("points.{at}.x")), &point.x));
            all.push((Cow::Owned(format!("points.{at}.y")), &point.y));
        }
        all
    }

    fn settings_mut(&mut self) -> Vec<(Cow<'static, str>, &mut Param)> {
        let mut all = named([
            ("level", &mut self.level),
            ("soft_edge", &mut self.soft_edge),
            ("border_width", &mut self.border_width),
            ("center.x", &mut self.center.x),
            ("center.y", &mut self.center.y),
            ("size", &mut self.size),
            ("angle", &mut self.angle),
        ]);
        for (at, point) in self.points.iter_mut().enumerate() {
            all.push((Cow::Owned(format!("points.{at}.x")), &mut point.x));
            all.push((Cow::Owned(format!("points.{at}.y")), &mut point.y));
        }
        all
    }
}

// ---------------------------------------------------------------------------------------------
// Blur and Mosaic
// ---------------------------------------------------------------------------------------------

/// How a Blur takes the pixels about each one (Filter, p. 2060): of Fusion's five, the two
/// that the filters of the compositor have. It is written `"gaussian"` or `"box"`, and takes
/// no keys.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlurFilter {
    /// A Gaussian, whose standard deviation the Blur Size is. Every Blur has it at first.
    #[default]
    Gaussian,
    /// The mean of the pixels within the Blur Size of each one, its radius.
    Box,
}

/// The Blur Size of a [`Blur`]: how far it reaches across, `x`, and down, `y`, each a number
/// or keys, in pixels of a picture 1080 pixels high (see the [module](self)).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlurSize {
    /// How far the blur reaches across; and down too, with Lock X/Y.
    pub x: Param,
    /// How far the blur reaches down, without Lock X/Y.
    pub y: Param,
}

/// The settings of a Blur (pp. 2059–2061), its daily controls: the picture is blurred by
/// `filter` as far as `size`, across and down, and mixed with the picture as it came in by
/// `blend`. The pixels beyond the edges of the picture are those at its edges, and its alpha
/// is blurred with its colour.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Blur {
    /// Filter (p. 2060).
    pub filter: BlurFilter,
    /// Lock X/Y (p. 2060): the blur across and the blur down are both `size.x`, and `size.y`
    /// is kept and not read. It takes no keys.
    pub lock_xy: bool,
    /// Blur Size (p. 2060).
    pub size: BlurSize,
    /// Blend (p. 2061): from 0, the picture as it came in, to 1, the blurred picture.
    pub blend: Param,
}

impl Default for Blur {
    /// A Gaussian of 5 pixels of a picture 1080 high, across and down alike, shown whole.
    fn default() -> Self {
        Self {
            filter: BlurFilter::Gaussian,
            lock_xy: true,
            size: BlurSize {
                x: bounds::BLUR_SIZE.param(),
                y: bounds::BLUR_SIZE.param(),
            },
            blend: bounds::BLUR_BLEND.param(),
        }
    }
}

impl Blur {
    /// Its settings at a frame of its keys: the Blur Size as it is read, both `size.x` with
    /// Lock X/Y.
    #[must_use]
    pub fn at(&self, frame: f64) -> Blurred {
        let across = self.size.x.value_at(frame);
        let down = if self.lock_xy {
            across
        } else {
            self.size.y.value_at(frame)
        };
        Blurred {
            filter: self.filter,
            size: [across, down],
            blend: self.blend.value_at(frame),
        }
    }

    fn check(&self) -> Result<(), OpError> {
        bounds::BLUR_SIZE.check("blur.size.x", &self.size.x)?;
        bounds::BLUR_SIZE.check("blur.size.y", &self.size.y)?;
        bounds::BLUR_BLEND.check("blur.blend", &self.blend)
    }

    fn settings(&self) -> [(&'static str, &Param); 3] {
        [
            ("size.x", &self.size.x),
            ("size.y", &self.size.y),
            ("blend", &self.blend),
        ]
    }

    fn settings_mut(&mut self) -> [(&'static str, &mut Param); 3] {
        [
            ("size.x", &mut self.size.x),
            ("size.y", &mut self.size.y),
            ("blend", &mut self.blend),
        ]
    }
}

/// How far the grid of a [`Mosaic`] is moved: `x` to the right and `y` down, each a number or
/// keys, in pixels of the picture.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MosaicOffset {
    /// X Offset: how many pixels to the right.
    pub x: Param,
    /// Y Offset: how many pixels down.
    pub y: Param,
}

/// The settings of a Mosaic (the Mosaic Blur of Resolve FX, p. 3465): those of the
/// mathematics of the mosaic of the compositor, by the same names. The picture is laid out
/// in a grid of cells, `frequency` of them across its width, each `aspect` times as wide as
/// high, the grid moved by `offset`; and each cell is shown in one colour, the mean of the
/// picture under it. The cells are squares, or rectangles where `aspect` is not 1.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mosaic {
    /// Pixel Frequency: how many cells lie across the width of the picture.
    pub frequency: Param,
    /// Aspect: how many times as wide as high a cell is.
    pub aspect: Param,
    /// X Offset and Y Offset.
    pub offset: MosaicOffset,
    /// Whole Cells Only: a cell is shown only where the effect mask of the node is 1 over all
    /// of it. Without a mask it changes nothing. It takes no keys.
    pub whole_cells_only: bool,
}

impl Default for Mosaic {
    /// 32 square cells across, the grid where it lies, and every cell shown where the mask
    /// reaches it.
    fn default() -> Self {
        Self {
            frequency: bounds::MOSAIC_FREQUENCY.param(),
            aspect: bounds::MOSAIC_ASPECT.param(),
            offset: MosaicOffset {
                x: bounds::MOSAIC_OFFSET.param(),
                y: bounds::MOSAIC_OFFSET.param(),
            },
            whole_cells_only: false,
        }
    }
}

impl Mosaic {
    /// Its settings at a frame of its keys.
    #[must_use]
    pub fn at(&self, frame: f64) -> Mosaicked {
        Mosaicked {
            frequency: self.frequency.value_at(frame),
            aspect: self.aspect.value_at(frame),
            offset: [self.offset.x.value_at(frame), self.offset.y.value_at(frame)],
            whole_cells_only: self.whole_cells_only,
        }
    }

    fn check(&self) -> Result<(), OpError> {
        bounds::MOSAIC_FREQUENCY.check("mosaic.frequency", &self.frequency)?;
        bounds::MOSAIC_ASPECT.check("mosaic.aspect", &self.aspect)?;
        bounds::MOSAIC_OFFSET.check("mosaic.offset.x", &self.offset.x)?;
        bounds::MOSAIC_OFFSET.check("mosaic.offset.y", &self.offset.y)
    }

    fn settings(&self) -> [(&'static str, &Param); 4] {
        [
            ("frequency", &self.frequency),
            ("aspect", &self.aspect),
            ("offset.x", &self.offset.x),
            ("offset.y", &self.offset.y),
        ]
    }

    fn settings_mut(&mut self) -> [(&'static str, &mut Param); 4] {
        [
            ("frequency", &mut self.frequency),
            ("aspect", &mut self.aspect),
            ("offset.x", &mut self.offset.x),
            ("offset.y", &mut self.offset.y),
        ]
    }
}

// ---------------------------------------------------------------------------------------------
// The Delta Keyer
// ---------------------------------------------------------------------------------------------

/// The least by which one channel of the screen colour of a [`DeltaKeyer`] is above its other
/// two: that of the mathematics of the keyer of the compositor, which is held to it. Ours.
pub const LEAST_DOMINANCE: f32 = 1e-4;

/// A colour of a [`DeltaKeyer`]: its red, its green and its blue, each a number or keys. A
/// channel is from 0, and may be above 1, as a colour of a picture may.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyColor {
    /// Red.
    pub r: Param,
    /// Green.
    pub g: Param,
    /// Blue.
    pub b: Param,
}

impl KeyColor {
    /// A colour that is the same on every frame.
    #[must_use]
    pub fn new(r: f64, g: f64, b: f64) -> Self {
        Self {
            r: r.into(),
            g: g.into(),
            b: b.into(),
        }
    }

    /// Its red, its green and its blue.
    fn channels(&self) -> [&Param; 3] {
        [&self.r, &self.g, &self.b]
    }

    fn at(&self, frame: f64) -> [f64; 3] {
        self.channels().map(|channel| channel.value_at(frame))
    }
}

/// How strongly a [`DeltaKeyer`] suppresses the spill of its screen (Spill Method, p. 2446),
/// from the weakest. It is written `"none"`, `"rare"`, `"medium"`, `"well_done"` or
/// `"burnt"`, and takes no keys.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpillMethod {
    /// No spill suppression.
    None,
    /// The lightest.
    Rare,
    /// The one that the manual gives for green screens. Every Delta Keyer has it at first.
    #[default]
    Medium,
    /// The one that the manual gives for blue screens.
    WellDone,
    /// The strongest, for the shots that give the most trouble.
    Burnt,
}

/// What fills the part of a pixel that the key of a [`DeltaKeyer`] took away and its matte
/// gave back (Replace Mode, p. 2445). It is written `"none"`, `"source"`, `"hard_color"` or
/// `"soft_color"`, and takes no keys.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplaceMode {
    /// Nothing.
    None,
    /// The colour of the picture itself.
    Source,
    /// The Replace Color.
    HardColor,
    /// The Replace Color, by how much of the colour of the screen was taken away. Every Delta
    /// Keyer has it at first.
    #[default]
    SoftColor,
}

/// What comes out of a [`DeltaKeyer`] (View Mode, p. 2442). It is written `"pre_matte"`,
/// `"matte"`, `"status"` or `"final_result"`, and takes no keys.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyerView {
    /// The matte that the key made, before the thresholds: white where the picture is kept.
    PreMatte,
    /// The matte, as it is before the solid and the garbage mattes.
    Matte,
    /// What is solid, what is transparent, what is in between, and what the matte changed.
    Status,
    /// The keyed picture, ready to be laid over another. Every Delta Keyer has it at first.
    #[default]
    FinalResult,
}

/// The settings of a Delta Keyer (pp. 2440–2448): those of the mathematics of the keyer of
/// the compositor, by the same names, in the order of what is done with them (see the
/// [module](self), The Delta Keyer).
///
/// The screen colour is to have one channel above its other two by [`LEAST_DOMINANCE`], and
/// `low` is to be at most `high`: both are checked where the numbers have no keys.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeltaKeyer {
    /// Background Color (p. 2443): the colour of the screen, picked from the picture.
    pub screen: KeyColor,
    /// Gain (p. 2443): more of it makes more of what is of the screen colour transparent.
    pub gain: Param,
    /// Balance (p. 2443): in what shares the two channels that are not the screen's are
    /// compared with it, from 0, the lesser, to 1, the greater.
    pub balance: Param,
    /// Pre-Blur (p. 2443): how far the picture is blurred before it is keyed, the standard
    /// deviation of a Gaussian in pixels of a picture 1080 high. The colours that come out
    /// are not blurred.
    pub pre_blur: Param,
    /// The lower threshold (p. 2445): a matte at or below it is transparent.
    pub low: Param,
    /// The upper threshold (p. 2445): a matte at or above it is solid.
    pub high: Param,
    /// Erode/Dilate (p. 2445): how far the matte is eroded, below 0, or dilated, above it,
    /// the radius of a box in pixels of a picture 1080 high.
    pub erode_dilate: Param,
    /// Matte Blur (p. 2445): how far the matte is blurred, the standard deviation of a
    /// Gaussian in pixels of a picture 1080 high.
    pub matte_blur: Param,
    /// Clean Foreground (p. 2445): how much the holes of the solid part of the matte are
    /// filled, from 0 to 1.
    pub clean_foreground: Param,
    /// Clean Background (p. 2445): how much the specks of the transparent part of the matte
    /// are taken away, from 0 to 1.
    pub clean_background: Param,
    /// Replace Mode (p. 2445).
    pub replace: ReplaceMode,
    /// Replace Color (p. 2445).
    pub replace_color: KeyColor,
    /// Invert of the Garbage Matte: the garbage matte is read the other way round, so that
    /// the matte is made transparent where the picture linked to it is not. It takes no
    /// keys, and is nothing where no garbage matte is linked.
    pub garbage_invert: bool,
    /// Spill Method (p. 2446).
    pub spill_method: SpillMethod,
    /// Spill Suppression (p. 2446): from 0, none, to 1, all that the method takes.
    pub spill_suppression: Param,
    /// View Mode (p. 2442): what comes out of the node.
    pub view: KeyerView,
}

impl Default for DeltaKeyer {
    /// A pure green screen keyed with a Gain of 1 and a Balance of a half; a matte left as
    /// the key made it; the spill suppressed whole by Medium, and Soft Color with a mid grey;
    /// the final result shown.
    fn default() -> Self {
        Self {
            screen: KeyColor {
                r: bounds::KEYER_SCREEN.param(),
                g: bounds::KEYER_SCREEN_GREEN.param(),
                b: bounds::KEYER_SCREEN.param(),
            },
            gain: bounds::KEYER_GAIN.param(),
            balance: bounds::KEYER_BALANCE.param(),
            pre_blur: bounds::KEYER_BLUR.param(),
            low: bounds::KEYER_LOW.param(),
            high: bounds::KEYER_HIGH.param(),
            erode_dilate: bounds::KEYER_ERODE_DILATE.param(),
            matte_blur: bounds::KEYER_BLUR.param(),
            clean_foreground: bounds::KEYER_CLEAN.param(),
            clean_background: bounds::KEYER_CLEAN.param(),
            replace: ReplaceMode::SoftColor,
            replace_color: KeyColor {
                r: bounds::KEYER_REPLACE_COLOR.param(),
                g: bounds::KEYER_REPLACE_COLOR.param(),
                b: bounds::KEYER_REPLACE_COLOR.param(),
            },
            garbage_invert: false,
            spill_method: SpillMethod::Medium,
            spill_suppression: bounds::KEYER_SPILL_SUPPRESSION.param(),
            view: KeyerView::FinalResult,
        }
    }
}

/// How far the greatest channel of a screen colour is above the greater of its other two, as
/// the mathematics of the keyer of the compositor works it out, in floats of 32 bits: green
/// is before blue and blue before red where two are alike.
#[allow(
    clippy::cast_possible_truncation,
    reason = "channels of a colour, as the compositor takes them"
)]
fn dominance(screen: [f64; 3]) -> f32 {
    let [red, green, blue] = screen.map(|channel| channel as f32);
    let (main, first, second) = if green >= red && green >= blue {
        (green, red, blue)
    } else if blue >= red {
        (blue, red, green)
    } else {
        (red, green, blue)
    };
    let (least, most) = (first.min(second), first.max(second));
    main - (least + (most - least))
}

impl DeltaKeyer {
    /// Its settings at a frame of its keys.
    #[must_use]
    pub fn at(&self, frame: f64) -> DeltaKeyed {
        DeltaKeyed {
            screen: self.screen.at(frame),
            gain: self.gain.value_at(frame),
            balance: self.balance.value_at(frame),
            pre_blur: self.pre_blur.value_at(frame),
            low: self.low.value_at(frame),
            high: self.high.value_at(frame),
            erode_dilate: self.erode_dilate.value_at(frame),
            matte_blur: self.matte_blur.value_at(frame),
            clean_foreground: self.clean_foreground.value_at(frame),
            clean_background: self.clean_background.value_at(frame),
            replace: self.replace,
            replace_color: self.replace_color.at(frame),
            garbage_invert: self.garbage_invert,
            spill_method: self.spill_method,
            spill_suppression: self.spill_suppression.value_at(frame),
            view: self.view,
        }
    }

    /// The bounds of its settings that take keys, in the order of its `params`, each with
    /// the name that it is refused by.
    const BOUNDS: [(&'static str, Bounds); 16] = [
        ("delta_keyer.screen.r", bounds::KEYER_SCREEN),
        ("delta_keyer.screen.g", bounds::KEYER_SCREEN_GREEN),
        ("delta_keyer.screen.b", bounds::KEYER_SCREEN),
        ("delta_keyer.gain", bounds::KEYER_GAIN),
        ("delta_keyer.balance", bounds::KEYER_BALANCE),
        ("delta_keyer.pre_blur", bounds::KEYER_BLUR),
        ("delta_keyer.low", bounds::KEYER_LOW),
        ("delta_keyer.high", bounds::KEYER_HIGH),
        ("delta_keyer.erode_dilate", bounds::KEYER_ERODE_DILATE),
        ("delta_keyer.matte_blur", bounds::KEYER_BLUR),
        ("delta_keyer.clean_foreground", bounds::KEYER_CLEAN),
        ("delta_keyer.clean_background", bounds::KEYER_CLEAN),
        ("delta_keyer.replace_color.r", bounds::KEYER_REPLACE_COLOR),
        ("delta_keyer.replace_color.g", bounds::KEYER_REPLACE_COLOR),
        ("delta_keyer.replace_color.b", bounds::KEYER_REPLACE_COLOR),
        (
            "delta_keyer.spill_suppression",
            bounds::KEYER_SPILL_SUPPRESSION,
        ),
    ];

    /// Check that each setting is within its bounds, as a number or at each key; and, of
    /// numbers that have no keys, that the lower threshold is at most the upper and that the
    /// screen colour has one channel above its other two.
    fn check(&self) -> Result<(), OpError> {
        for ((what, range), (_, setting)) in Self::BOUNDS.into_iter().zip(self.settings()) {
            range.check(what, setting)?;
        }
        if let (Param::Fixed(low), Param::Fixed(high)) = (&self.low, &self.high) {
            if low > high {
                return Err(OpError::OutOfRange {
                    what: "delta_keyer.low",
                });
            }
        }
        if let [Param::Fixed(red), Param::Fixed(green), Param::Fixed(blue)] = self.screen.channels()
        {
            if dominance([*red, *green, *blue]) < LEAST_DOMINANCE {
                return Err(OpError::OutOfRange {
                    what: "delta_keyer.screen",
                });
            }
        }
        Ok(())
    }

    fn settings(&self) -> [(&'static str, &Param); 16] {
        [
            ("screen.r", &self.screen.r),
            ("screen.g", &self.screen.g),
            ("screen.b", &self.screen.b),
            ("gain", &self.gain),
            ("balance", &self.balance),
            ("pre_blur", &self.pre_blur),
            ("low", &self.low),
            ("high", &self.high),
            ("erode_dilate", &self.erode_dilate),
            ("matte_blur", &self.matte_blur),
            ("clean_foreground", &self.clean_foreground),
            ("clean_background", &self.clean_background),
            ("replace_color.r", &self.replace_color.r),
            ("replace_color.g", &self.replace_color.g),
            ("replace_color.b", &self.replace_color.b),
            ("spill_suppression", &self.spill_suppression),
        ]
    }

    fn settings_mut(&mut self) -> [(&'static str, &mut Param); 16] {
        [
            ("screen.r", &mut self.screen.r),
            ("screen.g", &mut self.screen.g),
            ("screen.b", &mut self.screen.b),
            ("gain", &mut self.gain),
            ("balance", &mut self.balance),
            ("pre_blur", &mut self.pre_blur),
            ("low", &mut self.low),
            ("high", &mut self.high),
            ("erode_dilate", &mut self.erode_dilate),
            ("matte_blur", &mut self.matte_blur),
            ("clean_foreground", &mut self.clean_foreground),
            ("clean_background", &mut self.clean_background),
            ("replace_color.r", &mut self.replace_color.r),
            ("replace_color.g", &mut self.replace_color.g),
            ("replace_color.b", &mut self.replace_color.b),
            ("spill_suppression", &mut self.spill_suppression),
        ]
    }
}

/// Settings that are named by fixed paths, as a list that settings named otherwise may be
/// added to.
fn named<P>(settings: impl IntoIterator<Item = (&'static str, P)>) -> Vec<(Cow<'static, str>, P)> {
    settings
        .into_iter()
        .map(|(name, setting)| (Cow::Borrowed(name), setting))
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Nodes
// ---------------------------------------------------------------------------------------------

/// What kind of node a node is, and how it is set.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum NodeKind {
    /// The picture of the clip's own source: it has no settings.
    MediaIn,
    /// The picture that goes back to the timeline: it has no settings.
    MediaOut,
    /// A foreground laid over a background.
    Merge(Merge),
    /// A picture moved, made larger or smaller, turned and flipped.
    Transform(Transform),
    /// A colour over the whole picture.
    Background(Background),
    /// The mask of a rectangle.
    Rectangle(Rectangle),
    /// The mask of an ellipse.
    Ellipse(Ellipse),
    /// The mask of a polygon.
    Polygon(Polygon),
    /// A picture blurred.
    Blur(Blur),
    /// A picture laid out in cells of one colour each.
    Mosaic(Mosaic),
    /// A picture keyed: the screen that it was shot against made transparent.
    DeltaKeyer(DeltaKeyer),
    /// The picture that was tracked held still, or another made to follow it.
    Tracker(Tracker),
}

/// The settings of a kind that has none, as they are written: `{}`.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NoSettings {}

impl NodeKind {
    /// Its kind.
    #[must_use]
    pub fn kind(&self) -> Kind {
        match self {
            Self::MediaIn => Kind::MediaIn,
            Self::MediaOut => Kind::MediaOut,
            Self::Merge(_) => Kind::Merge,
            Self::Transform(_) => Kind::Transform,
            Self::Background(_) => Kind::Background,
            Self::Rectangle(_) => Kind::Rectangle,
            Self::Ellipse(_) => Kind::Ellipse,
            Self::Polygon(_) => Kind::Polygon,
            Self::Blur(_) => Kind::Blur,
            Self::Mosaic(_) => Kind::Mosaic,
            Self::DeltaKeyer(_) => Kind::DeltaKeyer,
            Self::Tracker(_) => Kind::Tracker,
        }
    }

    /// A node of `kind` as a new one is set.
    #[must_use]
    pub fn new(kind: Kind) -> Self {
        match kind {
            Kind::MediaIn => Self::MediaIn,
            Kind::MediaOut => Self::MediaOut,
            Kind::Merge => Self::Merge(Merge::default()),
            Kind::Transform => Self::Transform(Transform::default()),
            Kind::Background => Self::Background(Background::default()),
            Kind::Rectangle => Self::Rectangle(Rectangle::default()),
            Kind::Ellipse => Self::Ellipse(Ellipse::default()),
            Kind::Polygon => Self::Polygon(Polygon::default()),
            Kind::Blur => Self::Blur(Blur::default()),
            Kind::Mosaic => Self::Mosaic(Mosaic::default()),
            Kind::DeltaKeyer => Self::DeltaKeyer(DeltaKeyer::default()),
            Kind::Tracker => Self::Tracker(Tracker::default()),
        }
    }

    /// Read the settings of a node of `kind`.
    ///
    /// # Errors
    /// They are not the settings of a node of that kind.
    pub fn from_params(kind: Kind, params: serde_json::Value) -> Result<Self, serde_json::Error> {
        Ok(match kind {
            Kind::MediaIn => {
                serde_json::from_value::<NoSettings>(params)?;
                Self::MediaIn
            }
            Kind::MediaOut => {
                serde_json::from_value::<NoSettings>(params)?;
                Self::MediaOut
            }
            Kind::Merge => Self::Merge(serde_json::from_value(params)?),
            Kind::Transform => Self::Transform(serde_json::from_value(params)?),
            Kind::Background => Self::Background(serde_json::from_value(params)?),
            Kind::Rectangle => Self::Rectangle(serde_json::from_value(params)?),
            Kind::Ellipse => Self::Ellipse(serde_json::from_value(params)?),
            Kind::Polygon => Self::Polygon(serde_json::from_value(params)?),
            Kind::Blur => Self::Blur(serde_json::from_value(params)?),
            Kind::Mosaic => Self::Mosaic(serde_json::from_value(params)?),
            Kind::DeltaKeyer => Self::DeltaKeyer(serde_json::from_value(params)?),
            Kind::Tracker => Self::Tracker(serde_json::from_value(params)?),
        })
    }

    /// Its settings, as they are written in `params`: an object with no fields for a kind
    /// that has none.
    ///
    /// # Panics
    /// Never: the settings of a node are always written as JSON.
    #[must_use]
    pub fn params(&self) -> serde_json::Value {
        let value = match self {
            Self::MediaIn | Self::MediaOut => serde_json::to_value(NoSettings {}),
            Self::Merge(params) => serde_json::to_value(params),
            Self::Transform(params) => serde_json::to_value(params),
            Self::Background(params) => serde_json::to_value(params),
            Self::Rectangle(params) => serde_json::to_value(params),
            Self::Ellipse(params) => serde_json::to_value(params),
            Self::Polygon(params) => serde_json::to_value(params),
            Self::Blur(params) => serde_json::to_value(params),
            Self::Mosaic(params) => serde_json::to_value(params),
            Self::DeltaKeyer(params) => serde_json::to_value(params),
            Self::Tracker(params) => serde_json::to_value(params),
        };
        value.expect("the settings of a node are written as JSON")
    }

    /// Check its settings: each within its bounds, as a number or at each key; of a Polygon,
    /// that it has no more points than it may; of a Delta Keyer, that its thresholds are in
    /// order and its screen colour has a channel above its other two, where they have no
    /// keys; and of a Tracker, that it has no more patterns than it may, each with its
    /// numbers and the places and frames of its path within their bounds. That the node
    /// which a centre follows is a Tracker is checked of the composition, whole.
    pub(crate) fn check(&self) -> Result<(), OpError> {
        match self {
            Self::MediaIn | Self::MediaOut => Ok(()),
            Self::Merge(merge) => merge.check(),
            Self::Transform(transform) => transform.check(),
            Self::Background(background) => background.check(),
            Self::Rectangle(rectangle) => rectangle.check(),
            Self::Ellipse(ellipse) => ellipse.check(),
            Self::Polygon(polygon) => polygon.check(),
            Self::Blur(blur) => blur.check(),
            Self::Mosaic(mosaic) => mosaic.check(),
            Self::DeltaKeyer(keyer) => keyer.check(),
            Self::Tracker(tracker) => tracker.check(),
        }
    }

    /// Its settings that take keys, each with its path in `params`. A Tracker has none.
    fn keyed(&self) -> Vec<(Cow<'static, str>, &Param)> {
        match self {
            Self::MediaIn | Self::MediaOut | Self::Tracker(_) => vec![],
            Self::Merge(merge) => named(merge.settings()),
            Self::Transform(transform) => named(transform.settings()),
            Self::Background(background) => named(background.settings()),
            Self::Rectangle(rectangle) => named(rectangle.settings()),
            Self::Ellipse(ellipse) => named(ellipse.settings()),
            Self::Polygon(polygon) => polygon.settings(),
            Self::Blur(blur) => named(blur.settings()),
            Self::Mosaic(mosaic) => named(mosaic.settings()),
            Self::DeltaKeyer(keyer) => named(keyer.settings()),
        }
    }

    /// Its settings that take keys, each with its path in `params`, to be changed.
    fn settings_mut(&mut self) -> Vec<(Cow<'static, str>, &mut Param)> {
        match self {
            Self::MediaIn | Self::MediaOut | Self::Tracker(_) => vec![],
            Self::Merge(merge) => named(merge.settings_mut()),
            Self::Transform(transform) => named(transform.settings_mut()),
            Self::Background(background) => named(background.settings_mut()),
            Self::Rectangle(rectangle) => named(rectangle.settings_mut()),
            Self::Ellipse(ellipse) => named(ellipse.settings_mut()),
            Self::Polygon(polygon) => polygon.settings_mut(),
            Self::Blur(blur) => named(blur.settings_mut()),
            Self::Mosaic(mosaic) => named(mosaic.settings_mut()),
            Self::DeltaKeyer(keyer) => named(keyer.settings_mut()),
        }
    }

    /// The setting at `path` of its `params`, if it is one that takes keys: `center.x`, or
    /// `points.2.x` of a Polygon that has a third point.
    pub(crate) fn setting_mut(&mut self, path: &str) -> Option<&mut Param> {
        self.settings_mut()
            .into_iter()
            .find_map(|(name, setting)| (name == path).then_some(setting))
    }

    /// The paths of its settings that take keys, in the order of its `params`: of a
    /// Polygon, those of the points that it has, by their places. A Tracker has none.
    #[must_use]
    pub fn settings(&self) -> Vec<String> {
        self.keyed()
            .into_iter()
            .map(|(name, _)| name.into_owned())
            .collect()
    }

    /// Its centre, of a kind that has one: a Merge, a Transform and the masks.
    fn center(&self) -> Option<&Center> {
        match self {
            Self::Merge(merge) => Some(&merge.center),
            Self::Transform(transform) => Some(&transform.center),
            Self::Rectangle(rectangle) => Some(&rectangle.center),
            Self::Ellipse(ellipse) => Some(&ellipse.center),
            Self::Polygon(polygon) => Some(&polygon.center),
            Self::MediaIn
            | Self::MediaOut
            | Self::Background(_)
            | Self::Blur(_)
            | Self::Mosaic(_)
            | Self::DeltaKeyer(_)
            | Self::Tracker(_) => None,
        }
    }

    /// What its centre follows of a Tracker, if it has a centre and that follows one.
    #[must_use]
    pub fn follow(&self) -> Option<&Follow> {
        self.center().and_then(|center| center.follow.as_ref())
    }

    /// What its centre follows of a Tracker, to be changed: for whoever puts the id of a
    /// node in the place of a name.
    #[must_use]
    pub fn follow_mut(&mut self) -> Option<&mut Follow> {
        let center = match self {
            Self::Merge(merge) => &mut merge.center,
            Self::Transform(transform) => &mut transform.center,
            Self::Rectangle(rectangle) => &mut rectangle.center,
            Self::Ellipse(ellipse) => &mut ellipse.center,
            Self::Polygon(polygon) => &mut polygon.center,
            Self::MediaIn
            | Self::MediaOut
            | Self::Background(_)
            | Self::Blur(_)
            | Self::Mosaic(_)
            | Self::DeltaKeyer(_)
            | Self::Tracker(_) => return None,
        };
        center.follow.as_mut()
    }

    /// Whether what it is changes with the frame of its own accord: one of its settings has
    /// keys, or, of a Tracker, one of its patterns has a path. A node whose centre follows
    /// such a Tracker changes with it too, which its composition tells
    /// ([`Composition::node_is_animated`]).
    #[must_use]
    pub fn is_animated(&self) -> bool {
        if let Self::Tracker(tracker) = self {
            return tracker.has_paths();
        }
        self.keyed()
            .into_iter()
            .any(|(_, setting)| setting.is_animated())
    }

    /// It with each key of its settings at the frame `to` of its frame
    /// ([`Param::with_key_frames`]); and, of a Tracker, each frame of the paths of its
    /// patterns and its reference.
    #[must_use]
    pub fn with_key_frames(&self, to: &impl Fn(i64) -> i64) -> NodeKind {
        if let Self::Tracker(tracker) = self {
            return Self::Tracker(tracker.with_key_frames(to));
        }
        let mut moved = self.clone();
        for (_, setting) in moved.settings_mut() {
            *setting = setting.with_key_frames(to);
        }
        moved
    }
}

/// A node of a composition.
///
/// It is written `{"id", "kind", "label", "pass_through", "pos", "params"}`, where `params` is
/// what a node of its kind is set by: a [`Merge`], a [`Transform`], a [`Background`], a
/// [`Rectangle`], an [`Ellipse`], a [`Polygon`], a [`Blur`], a [`Mosaic`], a [`DeltaKeyer`]
/// or a [`Tracker`], and an object with no fields for `media_in` and `media_out`.
/// `pass_through` and `pos` are always written, and may be left out where a node is read:
/// they are then `false` and `null`.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    /// What it is known by in its composition: `n` and a number where the engine made it. It
    /// has no dot in it, since a dot ends it in the path of a setting.
    pub id: Id,
    /// The name that it was given, or empty: at most
    /// [`LONGEST_LABEL`](crate::grade::LONGEST_LABEL) characters.
    pub label: String,
    /// Whether it is passed through (Pass Through, Ctrl-P): it then hands on the picture of
    /// its first input as it is ([`Kind::through`]), and a node with no such input gives none.
    pub pass_through: bool,
    /// Where it lies in the node editor, or none for where the editor lays it out, as a node
    /// of a grade has it ([`Pos`]).
    pub pos: Option<Pos>,
    /// What kind of node it is, and how it is set.
    pub kind: NodeKind,
}

/// A node as it is written, before its `params` are read for its kind.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawNode {
    id: Id,
    kind: Kind,
    label: String,
    #[serde(default)]
    pass_through: bool,
    #[serde(default)]
    pos: Option<Pos>,
    params: serde_json::Value,
}

impl<'de> Deserialize<'de> for Node {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = RawNode::deserialize(deserializer)?;
        let kind = NodeKind::from_params(raw.kind, raw.params).map_err(serde::de::Error::custom)?;
        Ok(Self {
            id: raw.id,
            label: raw.label,
            pass_through: raw.pass_through,
            pos: raw.pos,
            kind,
        })
    }
}

impl Serialize for Node {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut node = serializer.serialize_struct("Node", 6)?;
        node.serialize_field("id", &self.id)?;
        node.serialize_field("kind", &self.kind.kind())?;
        node.serialize_field("label", &self.label)?;
        node.serialize_field("pass_through", &self.pass_through)?;
        node.serialize_field("pos", &self.pos)?;
        match &self.kind {
            NodeKind::MediaIn | NodeKind::MediaOut => {
                node.serialize_field("params", &NoSettings {})?;
            }
            NodeKind::Merge(params) => node.serialize_field("params", params)?,
            NodeKind::Transform(params) => node.serialize_field("params", params)?,
            NodeKind::Background(params) => node.serialize_field("params", params)?,
            NodeKind::Rectangle(params) => node.serialize_field("params", params)?,
            NodeKind::Ellipse(params) => node.serialize_field("params", params)?,
            NodeKind::Polygon(params) => node.serialize_field("params", params)?,
            NodeKind::Blur(params) => node.serialize_field("params", params)?,
            NodeKind::Mosaic(params) => node.serialize_field("params", params)?,
            NodeKind::DeltaKeyer(params) => node.serialize_field("params", params)?,
            NodeKind::Tracker(params) => node.serialize_field("params", params)?,
        }
        node.end()
    }
}

impl Node {
    /// A node of `kind` as a new one is set, not passed through, with no label, where the
    /// editor lays it out.
    #[must_use]
    pub fn new(id: impl Into<Id>, kind: Kind) -> Self {
        Self {
            id: id.into(),
            label: String::new(),
            pass_through: false,
            pos: None,
            kind: NodeKind::new(kind),
        }
    }
}

/// Check the id of a node: it has one, it does not start with `$`, and it has no dot in it.
pub(crate) fn check_node_id(id: &str) -> Result<(), OpError> {
    if id.is_empty() {
        Err(OpError::NoId)
    } else if id.starts_with('$') {
        Err(OpError::Dollar { id: id.into() })
    } else if id.contains('.') {
        Err(OpError::OutOfRange { what: "fusion_id" })
    } else {
        Ok(())
    }
}

/// Check a node but for its id: its label, where it lies and its settings.
pub(crate) fn check_node(node: &Node) -> Result<(), OpError> {
    check_label(&node.label)?;
    if let Some(pos) = node.pos {
        pos.check()?;
    }
    node.kind.check()
}

fn missing_node(id: &str) -> OpError {
    OpError::Missing {
        what: "node",
        id: id.into(),
    }
}

/// The size of the pictures of a composition that is made at a frame with no size told: a
/// square, of which a share of the width and a share of the height are one length.
const SQUARE: (u32, u32) = (1, 1);

/// A moment of a composition: a frame of its keys, and the size of its pictures in pixels.
#[derive(Clone, Copy)]
struct Moment {
    frame: f64,
    size: (u32, u32),
}

// ---------------------------------------------------------------------------------------------
// Compositions
// ---------------------------------------------------------------------------------------------

/// A composition: its nodes, in the order in which they were added, and its links, in the
/// order of the inputs that they go into (by `to`, then by the name of `input`).
///
/// At first it is the one that `DaVinci` gives a clip (p. 1265): a `MediaIn` linked to a
/// `MediaOut`, which shows the clip as it is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Composition {
    /// Its nodes.
    pub nodes: Vec<Node>,
    /// Its links.
    pub links: Vec<Link>,
}

impl Default for Composition {
    /// The composition that a clip has when the Fusion page is first opened on it: a `MediaIn`
    /// ([`MEDIA_IN`]) linked to a `MediaOut` ([`MEDIA_OUT`]), each where the editor lays it out.
    fn default() -> Self {
        Self {
            nodes: vec![
                Node::new(MEDIA_IN, Kind::MediaIn),
                Node::new(MEDIA_OUT, Kind::MediaOut),
            ],
            links: vec![Link::new(MEDIA_IN, MEDIA_OUT, Input::Input)],
        }
    }
}

impl Composition {
    /// The node `id`, if it has one.
    #[must_use]
    pub fn node(&self, id: &str) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }

    /// Where the node `id` is in its list of nodes.
    #[must_use]
    pub fn position(&self, id: &str) -> Option<usize> {
        self.nodes.iter().position(|n| n.id == id)
    }

    /// The link into `input` of `to`, if there is one.
    #[must_use]
    pub fn link_into(&self, to: &str, input: Input) -> Option<&Link> {
        self.links.iter().find(|l| l.target() == (to, input))
    }

    /// Its `MediaOut`: the first, where a composition that was not checked has several.
    #[must_use]
    pub fn media_out(&self) -> Option<&Node> {
        self.nodes
            .iter()
            .find(|node| node.kind.kind() == Kind::MediaOut)
    }

    /// Put its links in the order of the inputs that they go into.
    pub fn sort_links(&mut self) {
        self.links.sort_by(|a, b| a.place().cmp(&b.place()));
    }

    /// Whether a setting of one of its nodes has keys, or a pattern of one of its Trackers a
    /// path: whether it is another composition at another frame. A centre that follows a
    /// Tracker moves where that Tracker has a path, which is a node of the composition.
    #[must_use]
    pub fn is_animated(&self) -> bool {
        self.nodes.iter().any(|node| node.kind.is_animated())
    }

    /// Whether the node `id` changes with the frame: one of its settings has keys, one of its
    /// patterns has a path ([`NodeKind::is_animated`]), or its centre follows a Tracker one
    /// of whose patterns has a path. A node that is not there does not.
    #[must_use]
    pub fn node_is_animated(&self, id: &str) -> bool {
        let Some(node) = self.node(id) else {
            return false;
        };
        let follows_a_path = node
            .kind
            .follow()
            .and_then(|follow| self.node(&follow.node))
            .is_some_and(|named| named.kind.kind() == Kind::Tracker && named.kind.is_animated());
        node.kind.is_animated() || follows_a_path
    }

    /// The place that the Tracker which `follow` names publishes at `frame` of its keys, in
    /// pictures of `size` pixels ([`Tracker::published`]). None where nothing is published:
    /// the composition has no such node, the node is no Tracker or is passed through, the
    /// pattern is not there or has no path, or there is no motion to be had at the frame.
    #[must_use]
    pub fn published(&self, follow: &Follow, frame: f64, size: (u32, u32)) -> Option<[f64; 2]> {
        let node = self.node(&follow.node)?;
        let NodeKind::Tracker(tracker) = &node.kind else {
            return None;
        };
        if node.pass_through {
            return None;
        }
        tracker.published(follow.output, follow.tracker, frame, size)
    }

    /// Where `center` is at `frame` of its keys, in pictures of `size` pixels: what it
    /// follows, where that is published ([`Composition::published`]), held within `within`,
    /// the bounds of the centre; and its own `x` and `y` otherwise.
    fn center_at(&self, center: &Center, within: Bounds, moment: Moment) -> [f64; 2] {
        let Moment { frame, size } = moment;
        let published = center
            .follow
            .as_ref()
            .and_then(|follow| self.published(follow, frame, size));
        match published {
            Some(place) => place.map(|at| at.clamp(within.low, within.high)),
            None => center.at(frame),
        }
    }

    /// The composition with each key of each setting of its nodes moved from frame `k` to
    /// `to(k)`, as [`Param::with_key_frames`] moves the keys of a setting; and so each frame
    /// of the paths of its Trackers that have any, and their references.
    #[must_use]
    pub fn with_key_frames(&self, to: &impl Fn(i64) -> i64) -> Composition {
        let mut moved = self.clone();
        for node in &mut moved.nodes {
            if node.kind.is_animated() {
                node.kind = node.kind.with_key_frames(to);
            }
        }
        moved
    }

    /// Check the composition: how many nodes it has, their ids and their settings, that it has
    /// one `MediaOut`, that what its centres follow are Trackers of it, and its links. The
    /// links are to be in order.
    pub(crate) fn check(&self) -> Result<(), OpError> {
        if self.nodes.len() > MOST_NODES {
            return Err(OpError::OutOfRange {
                what: "fusion_nodes",
            });
        }
        let mut ids = HashSet::new();
        for node in &self.nodes {
            check_node_id(&node.id)?;
            if !ids.insert(node.id.as_str()) {
                return Err(OpError::Taken {
                    what: "node",
                    id: node.id.clone(),
                });
            }
            check_node(node)?;
        }
        let outs = self
            .nodes
            .iter()
            .filter(|node| node.kind.kind() == Kind::MediaOut)
            .count();
        if outs != 1 {
            return Err(OpError::OneMediaOut { count: outs });
        }
        self.check_follows()?;
        self.check_links()
    }

    /// Check what its centres follow: each node that one follows is there, and is a Tracker.
    pub(crate) fn check_follows(&self) -> Result<(), OpError> {
        for follow in self.nodes.iter().filter_map(|node| node.kind.follow()) {
            let named = self
                .node(&follow.node)
                .ok_or_else(|| missing_node(&follow.node))?;
            let kind = named.kind.kind();
            if kind != Kind::Tracker {
                return Err(OpError::WrongNode {
                    node: follow.node.clone(),
                    kind: kind.name(),
                });
            }
        }
        Ok(())
    }

    /// Check its links: each comes from a node that has an output and goes into an input that
    /// its node has, no two go into one input, and they make no loop. The links are to be in
    /// order.
    fn check_links(&self) -> Result<(), OpError> {
        for link in &self.links {
            let from = self
                .node(&link.from)
                .ok_or_else(|| missing_node(&link.from))?;
            if !from.kind.kind().has_output() {
                return Err(OpError::NoOutput {
                    node: link.from.clone(),
                });
            }
            let to = self.node(&link.to).ok_or_else(|| missing_node(&link.to))?;
            if !to.kind.kind().inputs().contains(&link.input) {
                return Err(OpError::NoInput {
                    node: link.to.clone(),
                    input: link.input,
                });
            }
        }
        if let Some(pair) = self
            .links
            .windows(2)
            .find(|pair| pair[0].target() == pair[1].target())
        {
            return Err(OpError::InputLinked {
                node: pair[1].to.clone(),
                input: pair[1].input,
            });
        }
        if let Some(node) = self.in_loop() {
            return Err(OpError::Loop { node });
        }
        Ok(())
    }

    /// A node that its links go round in a loop through, if there is one.
    fn in_loop(&self) -> Option<Id> {
        let mut after: HashMap<&str, Vec<&str>> = HashMap::new();
        for link in &self.links {
            after.entry(&link.from).or_default().push(&link.to);
        }
        // Not in it: not seen; false: on the path; true: done.
        let mut done: HashMap<&str, bool> = HashMap::new();
        for start in self.nodes.iter().map(|n| n.id.as_str()) {
            if done.contains_key(start) {
                continue;
            }
            let mut stack = vec![(start, 0_usize)];
            done.insert(start, false);
            while let Some((node, next)) = stack.pop() {
                let outs = after.get(node).map_or(&[][..], Vec::as_slice);
                if let Some(&to) = outs.get(next) {
                    stack.push((node, next + 1));
                    match done.get(to) {
                        Some(false) => return Some(to.to_owned()),
                        Some(true) => {}
                        None => {
                            done.insert(to, false);
                            stack.push((to, 0));
                        }
                    }
                } else {
                    done.insert(node, true);
                }
            }
        }
        None
    }

    /// The node whose picture comes into `input` of the node `to`: the one that the link
    /// there comes from, or, where that one is passed through, the one whose picture it hands
    /// on, and so on. None where no picture comes in: the input, or the input that a node
    /// that is passed through on the way hands on ([`Kind::through`]), is not linked, or such
    /// a node has none to hand on.
    fn feeding(&self, to: &str, input: Input) -> Option<&Node> {
        let (mut to, mut input) = (to, input);
        // A composition that is checked has no loop: one that is not is not followed for ever.
        for _ in 0..=self.nodes.len() {
            let node = self.node(&self.link_into(to, input)?.from)?;
            if !node.pass_through {
                return Some(node);
            }
            (to, input) = (node.id.as_str(), node.kind.kind().through()?);
        }
        None
    }

    /// The composition at a frame of its keys, as [`Composition::at_size`] has it for pictures
    /// that are square. The size is read by a Tracker alone, and by a centre that follows
    /// one: so this is the composition that has neither, whatever the size of its pictures.
    #[must_use]
    pub fn at(&self, frame: f64) -> Fused {
        self.at_size(frame, SQUARE)
    }

    /// The composition at a frame of its keys, its pictures of `size` pixels ([`size_of`]):
    /// the nodes that the picture of its `MediaOut` is made of, each with its settings as
    /// the numbers that they are there (see [`Fused`]). A centre that follows a Tracker is
    /// where that publishes, and a Tracker has the motion of its patterns there, which is a
    /// motion of pixels: that is what the size is for.
    #[must_use]
    pub fn at_size(&self, frame: f64, size: (u32, u32)) -> Fused {
        let moment = Moment { frame, size };
        let mut nodes = Vec::new();
        let mut placed = HashMap::new();
        let output = self
            .media_out()
            .and_then(|out| self.feeding(&out.id, Input::Input))
            .and_then(|node| self.place(node, moment, &mut nodes, &mut placed));
        Fused { nodes, output }
    }

    /// The composition at a frame of its keys, for the picture of the node `id` in place of
    /// the one that comes into its `MediaOut`: the nodes that the picture of that node is made
    /// of, each with its settings as the numbers that they are there, and `output` the node
    /// itself (see [`Fused`]). It is what a viewer shows of one node: the node and all that
    /// leads to it, and nothing after it.
    ///
    /// The node need not be one that the `MediaOut` depends on: a mask that is not linked yet,
    /// or a branch that is not merged yet, has its picture as any node has. Each node is as
    /// [`Composition::at`] has it, with the same settings, after those that it reads; those
    /// that the node does not depend on are left out. A node that is passed through shows
    /// what it hands on, the picture of its first input ([`Kind::through`]), and none where
    /// it has no such input or that input brings no picture; a node on the way that is passed
    /// through is left out for the one that it hands on, as it is in [`Composition::at`]. Of
    /// the `MediaOut` it is [`Composition::at`] itself. None where the composition has no node
    /// `id`. Its pictures are taken to be square, as [`Composition::at`] takes them.
    #[must_use]
    pub fn at_node(&self, id: &str, frame: f64) -> Option<Fused> {
        self.at_node_size(id, frame, SQUARE)
    }

    /// The composition at a frame of its keys for the picture of the node `id`, as
    /// [`Composition::at_node`] has it, its pictures of `size` pixels, as
    /// [`Composition::at_size`] has them.
    #[must_use]
    pub fn at_node_size(&self, id: &str, frame: f64, size: (u32, u32)) -> Option<Fused> {
        let node = self.node(id)?;
        if node.kind.kind() == Kind::MediaOut {
            return Some(self.at_size(frame, size));
        }
        let shown = if node.pass_through {
            node.kind
                .kind()
                .through()
                .and_then(|input| self.feeding(&node.id, input))
        } else {
            Some(node)
        };
        let moment = Moment { frame, size };
        let mut nodes = Vec::new();
        let mut placed = HashMap::new();
        let output = shown.and_then(|node| self.place(node, moment, &mut nodes, &mut placed));
        Some(Fused { nodes, output })
    }

    /// Put `node` among `nodes`, after the nodes whose pictures it reads, and give its place
    /// there: the one that it has already where it was put before, and none for a node that
    /// has no picture to give.
    #[allow(
        clippy::too_many_lines,
        reason = "what each kind of node is at a moment, one after another"
    )]
    fn place<'a>(
        &'a self,
        node: &'a Node,
        moment: Moment,
        nodes: &mut Vec<FusedNode>,
        placed: &mut HashMap<&'a str, Option<usize>>,
    ) -> Option<usize> {
        if let Some(at) = placed.get(node.id.as_str()) {
            return *at;
        }
        // A node on a loop, which a composition that is checked has none of, reads nothing of
        // itself.
        placed.insert(&node.id, None);
        let frame = moment.frame;
        let mut read = |input: Input| {
            self.feeding(&node.id, input)
                .and_then(|from| self.place(from, moment, nodes, placed))
        };
        // A centre is where it is at the frame: what it follows, where that is published.
        let kind = match &node.kind {
            NodeKind::MediaIn => FusedKind::MediaIn,
            NodeKind::MediaOut => return None,
            NodeKind::Background(background) => FusedKind::Background {
                rgba: background.at(frame),
                mask: read(Input::Mask),
            },
            NodeKind::Merge(merge) => FusedKind::Merge {
                background: read(Input::Background),
                foreground: read(Input::Foreground),
                mask: read(Input::Mask),
                settings: Merged {
                    center: self.center_at(&merge.center, bounds::CENTER, moment),
                    ..merge.at(frame)
                },
            },
            NodeKind::Transform(transform) => FusedKind::Transform {
                input: read(Input::Input),
                mask: read(Input::Mask),
                settings: Transformed {
                    center: self.center_at(&transform.center, bounds::CENTER, moment),
                    ..transform.at(frame)
                },
            },
            NodeKind::Rectangle(rectangle) => FusedKind::Rectangle {
                mask: read(Input::Mask),
                common: rectangle.common_at(frame),
                shape: RectangleShape {
                    center: self.center_at(&rectangle.center, bounds::MASK_CENTER, moment),
                    ..rectangle.at(frame)
                },
            },
            NodeKind::Ellipse(ellipse) => FusedKind::Ellipse {
                mask: read(Input::Mask),
                common: ellipse.common_at(frame),
                shape: EllipseShape {
                    center: self.center_at(&ellipse.center, bounds::MASK_CENTER, moment),
                    ..ellipse.at(frame)
                },
            },
            NodeKind::Polygon(polygon) => FusedKind::Polygon {
                mask: read(Input::Mask),
                common: polygon.common_at(frame),
                shape: PolygonShape {
                    center: self.center_at(&polygon.center, bounds::MASK_CENTER, moment),
                    ..polygon.at(frame)
                },
            },
            NodeKind::Blur(blur) => FusedKind::Blur {
                input: read(Input::Input),
                mask: read(Input::Mask),
                settings: blur.at(frame),
            },
            NodeKind::Mosaic(mosaic) => FusedKind::Mosaic {
                input: read(Input::Input),
                mask: read(Input::Mask),
                settings: mosaic.at(frame),
            },
            NodeKind::DeltaKeyer(keyer) => FusedKind::DeltaKeyer {
                input: read(Input::Input),
                garbage_matte: read(Input::GarbageMatte),
                solid_matte: read(Input::SolidMatte),
                mask: read(Input::Mask),
                settings: keyer.at(frame),
            },
            NodeKind::Tracker(tracker) => FusedKind::Tracker {
                input: read(Input::Input),
                foreground: read(Input::Foreground),
                mask: read(Input::Mask),
                settings: tracker.at(frame, moment.size),
            },
        };
        let at = nodes.len();
        nodes.push(FusedNode {
            id: node.id.clone(),
            kind,
        });
        placed.insert(&node.id, Some(at));
        Some(at)
    }
}

/// The size of the pictures of the composition of a clip of a timeline of a project, in
/// pixels: that of the source of the clip (p. 1265). Of media with a picture, its picture as
/// it is shown upright, so that one that is turned a quarter has its width for its height; of
/// a title, of a colour, and of media that the project has not, the picture of the timeline.
#[must_use]
pub fn size_of(clip: &Clip, project: &Project, timeline: &Timeline) -> (u32, u32) {
    let picture = match &clip.source {
        Source::Media { media, .. } => project
            .media
            .iter()
            .find(|m| m.id == *media)
            .and_then(|media| media.facts.video.as_ref()),
        Source::Title { .. } | Source::Color { .. } => None,
    };
    match picture {
        Some(video) if video.rotation.rem_euclid(180) == 90 => (video.height, video.width),
        Some(video) => (video.width, video.height),
        None => (timeline.width, timeline.height),
    }
}

// ---------------------------------------------------------------------------------------------
// A composition at one moment
// ---------------------------------------------------------------------------------------------

/// A composition at one moment, as the compositor draws it: the nodes that the picture of its
/// `MediaOut` is made of, each with its settings as the numbers that they are at the frame.
///
/// The nodes are in an order in which each comes after those whose pictures it reads, its
/// pictures before its mask (and the mattes of a Delta Keyer between them), and a node names
/// the one that an input reads by its place in the list. Only the nodes that the `MediaOut`
/// depends on are there, each once: a node whose picture is not read is left out, and so is
/// one that is passed through, whose place is taken by the node that it hands on. A node
/// that is read as a mask or as a matte alone is depended on, and is there. The `MediaOut`
/// itself is not in the list: `output` is what it shows.
///
/// A composition made for the picture of one of its nodes ([`Composition::at_node`]) is the
/// same, with that node in the place of what comes into the `MediaOut`: the nodes that the
/// node depends on, and `output` the node itself.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Fused {
    /// The nodes, each after those that it reads.
    pub nodes: Vec<FusedNode>,
    /// The place in `nodes` of the node whose picture goes back to the timeline; none where
    /// no picture comes into the `MediaOut`, and the clip is transparent.
    pub output: Option<usize>,
}

/// A node of a composition at one moment.
#[derive(Clone, Debug, PartialEq)]
pub struct FusedNode {
    /// The node of the composition that it is.
    pub id: Id,
    /// What it draws.
    pub kind: FusedKind,
}

/// What a node of a composition draws at one moment. A picture that an input reads is named
/// by its place in [`Fused::nodes`], which is before that of the node; none is no picture, a
/// transparent one of the size of the composition. But a `mask` that is none is no mask: the
/// node is not limited (see Masks of the [module](self)). One that names a node reads the
/// alpha of its picture. So it is with the two mattes of a Delta Keyer.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum FusedKind {
    /// The picture of the clip's own source at the frame.
    MediaIn,
    /// A colour over the whole picture.
    Background {
        /// Its red, green, blue and alpha, each from 0 to 1, not premultiplied.
        rgba: [f64; 4],
        /// Its effect mask, which its picture is multiplied by; none where it has none.
        mask: Option<usize>,
    },
    /// A foreground laid over a background.
    Merge {
        /// The picture of the background.
        background: Option<usize>,
        /// The picture of the foreground.
        foreground: Option<usize>,
        /// Its effect mask: the merged result where it is 1, the background as it came in
        /// where it is 0. None where it has none.
        mask: Option<usize>,
        /// How it is laid.
        settings: Merged,
    },
    /// A picture moved, made larger or smaller, turned and flipped.
    Transform {
        /// The picture.
        input: Option<usize>,
        /// Its effect mask: the picture as it is put where it is 1, as it came in where it
        /// is 0. None where it has none.
        mask: Option<usize>,
        /// How it is put.
        settings: Transformed,
    },
    /// The mask of a rectangle: a picture that is white premultiplied by the mask.
    Rectangle {
        /// The mask that comes in, which its Paint Mode combines with its own; none where
        /// none comes in, and the mask is its own.
        mask: Option<usize>,
        /// The controls that every mask has.
        common: Masked,
        /// Its shape.
        shape: RectangleShape,
    },
    /// The mask of an ellipse: a picture that is white premultiplied by the mask.
    Ellipse {
        /// The mask that comes in, which its Paint Mode combines with its own; none where
        /// none comes in, and the mask is its own.
        mask: Option<usize>,
        /// The controls that every mask has.
        common: Masked,
        /// Its shape.
        shape: EllipseShape,
    },
    /// The mask of a polygon: a picture that is white premultiplied by the mask.
    Polygon {
        /// The mask that comes in, which its Paint Mode combines with its own; none where
        /// none comes in, and the mask is its own.
        mask: Option<usize>,
        /// The controls that every mask has.
        common: Masked,
        /// Its shape.
        shape: PolygonShape,
    },
    /// A picture blurred.
    Blur {
        /// The picture.
        input: Option<usize>,
        /// Its effect mask: the picture blurred, and mixed by its Blend, where it is 1, and as
        /// it came in where it is 0. None where it has none.
        mask: Option<usize>,
        /// How it is blurred.
        settings: Blurred,
    },
    /// A picture laid out in cells of one colour each.
    Mosaic {
        /// The picture.
        input: Option<usize>,
        /// Its effect mask, the key of the mosaic: the colour of the cell where it is 1, the
        /// picture as it came in where it is 0; and with Whole Cells Only, the colour of the
        /// cell where it is 1 over all of the cell, and the picture as it came in elsewhere.
        /// None where it has none: every cell is shown.
        mask: Option<usize>,
        /// How it is laid out.
        settings: Mosaicked,
    },
    /// A picture keyed: the screen that it was shot against made transparent.
    DeltaKeyer {
        /// The picture that is keyed.
        input: Option<usize>,
        /// The picture whose alpha is the garbage matte: where it is 1 the matte is made
        /// transparent. None where there is none.
        garbage_matte: Option<usize>,
        /// The picture whose alpha is the solid matte: where it is 1 the matte is made
        /// solid. None where there is none.
        solid_matte: Option<usize>,
        /// Its effect mask: what the keyer shows where it is 1, the picture as it came in
        /// where it is 0. None where it has none.
        mask: Option<usize>,
        /// How it is keyed.
        settings: DeltaKeyed,
    },
    /// The picture that was tracked held still, or another made to follow it (see The
    /// Tracker of the [module](self)).
    Tracker {
        /// The picture that was tracked: what the node is with an Operation of None, what is
        /// held still with BG Only, and what lies under or over the foreground.
        input: Option<usize>,
        /// The picture that follows the patterns, with FG Only, FG Over BG and BG Over FG.
        foreground: Option<usize>,
        /// Its effect mask: what the node draws where it is 1, the picture of its `input` as
        /// it came in where it is 0. None where it has none.
        mask: Option<usize>,
        /// What it draws, and how the picture that is moved is put.
        settings: Tracked,
    },
    /// What comes out of the composition drawn through the stabilizer of its clip
    /// ([`crate::stabilize`]): no node of the composition, but what [`Fused::stabilized`] puts
    /// after its last node, between the node that the `MediaOut` read and the `MediaOut`.
    /// Where the picture is not moved over, it is transparent.
    Stabilize {
        /// The picture that comes out of the composition: that of the node that the `MediaOut`
        /// read.
        input: Option<usize>,
        /// Where each point of the picture is put, in shares of the picture, x of its width and
        /// y of its height from its top left corner, y down: a 3×3 matrix acting on `[x, y, 1]`,
        /// row by row ([`crate::stabilize::Stabilize::transform_at`]).
        matrix: Matrix,
    },
}

/// The id of the node that [`Fused::stabilized`] puts before the `MediaOut` of a composition:
/// it has a dot in it, which the id of no node of a composition has, so that it is never that
/// of one.
pub const STABILIZER: &str = "media_out.stabilized";

impl Fused {
    /// It with what comes out of it drawn through the stabilizer of its clip, `matrix`
    /// ([`FusedKind::Stabilize`]): a node that draws it so is put after its last node, reading
    /// the node that `output` named, and `output` is that node. The nodes before it are as they
    /// were, and none of them reads it. A composition out of which no picture comes, and a
    /// matrix that moves no point of the picture ([`crate::stabilize::moves`]), leave it as it
    /// is.
    #[must_use]
    pub fn stabilized(&self, matrix: Matrix) -> Fused {
        let Some(output) = self.output.filter(|_| crate::stabilize::moves(&matrix)) else {
            return self.clone();
        };
        let mut nodes = Vec::with_capacity(self.nodes.len() + 1);
        nodes.extend_from_slice(&self.nodes);
        nodes.push(FusedNode {
            id: STABILIZER.into(),
            kind: FusedKind::Stabilize {
                input: Some(output),
                matrix,
            },
        });
        Fused {
            output: Some(nodes.len() - 1),
            nodes,
        }
    }
}

impl FusedKind {
    /// It with each picture that it reads, by its place in [`Fused::nodes`], at the place
    /// `renumbered` gives for it.
    #[must_use]
    pub fn with_reads(&self, renumbered: &impl Fn(usize) -> usize) -> FusedKind {
        let mut out = self.clone();
        for read in out.inputs_mut() {
            *read = read.map(renumbered);
        }
        out
    }

    /// Each input of a node of the kind that may read a picture, as [`FusedKind::reads`] has
    /// them, to be changed.
    fn inputs_mut(&mut self) -> Vec<&mut Option<usize>> {
        match self {
            Self::MediaIn => vec![],
            Self::Background { mask, .. }
            | Self::Rectangle { mask, .. }
            | Self::Ellipse { mask, .. }
            | Self::Polygon { mask, .. } => vec![mask],
            Self::Merge {
                background,
                foreground,
                mask,
                ..
            } => vec![background, foreground, mask],
            Self::Tracker {
                input,
                foreground,
                mask,
                ..
            } => vec![input, foreground, mask],
            Self::Transform { input, mask, .. }
            | Self::Blur { input, mask, .. }
            | Self::Mosaic { input, mask, .. } => vec![input, mask],
            Self::Stabilize { input, .. } => vec![input],
            Self::DeltaKeyer {
                input,
                garbage_matte,
                solid_matte,
                mask,
                ..
            } => vec![input, garbage_matte, solid_matte, mask],
        }
    }

    /// The picture that the first input of a node of the kind reads, by its place in
    /// [`Fused::nodes`]: of a Merge its background, of a Transform, a Blur, a Mosaic, a Delta
    /// Keyer and a Tracker their picture, of the stabilizer what comes out of the composition,
    /// and of a mask node the mask that comes into it. None of a kind that has no such input,
    /// and where the input reads no picture. It is the picture that a node that is passed
    /// through hands on ([`Kind::through`]), and that what cannot draw a kind may hand on in
    /// its place.
    #[must_use]
    pub fn first_input(&self) -> Option<usize> {
        match self {
            Self::MediaIn | Self::Background { .. } => None,
            Self::Merge { background, .. } => *background,
            Self::Transform { input, .. }
            | Self::Blur { input, .. }
            | Self::Mosaic { input, .. }
            | Self::DeltaKeyer { input, .. }
            | Self::Tracker { input, .. }
            | Self::Stabilize { input, .. } => *input,
            Self::Rectangle { mask, .. }
            | Self::Ellipse { mask, .. }
            | Self::Polygon { mask, .. } => *mask,
        }
    }

    /// The mask that the `mask` input of a node of the kind reads, by its place in
    /// [`Fused::nodes`]: the effect mask of a Merge, of a Transform, of a Background, of a
    /// Blur, of a Mosaic, of a Delta Keyer and of a Tracker, and the mask that comes into a
    /// mask node. None of a `MediaIn` and of the stabilizer, which have no such input, and
    /// where the input is not linked or brings no picture.
    #[must_use]
    pub fn mask(&self) -> Option<usize> {
        match self {
            Self::MediaIn | Self::Stabilize { .. } => None,
            Self::Background { mask, .. }
            | Self::Merge { mask, .. }
            | Self::Transform { mask, .. }
            | Self::Rectangle { mask, .. }
            | Self::Ellipse { mask, .. }
            | Self::Polygon { mask, .. }
            | Self::Blur { mask, .. }
            | Self::Mosaic { mask, .. }
            | Self::DeltaKeyer { mask, .. }
            | Self::Tracker { mask, .. } => *mask,
        }
    }

    /// The pictures that the inputs of a node of the kind read, each by its place in
    /// [`Fused::nodes`], in the order of the inputs of the kind ([`Kind::inputs`]): its
    /// pictures, the two mattes of a Delta Keyer, and its mask last. An input that reads no
    /// picture is left out. They are the nodes that the node depends on, each of which is
    /// before it in the list. A Tracker that a centre follows is not among them: the centre
    /// is its numbers at the frame, and reads no picture.
    #[must_use]
    pub fn reads(&self) -> Vec<usize> {
        let inputs: Vec<Option<usize>> = match self {
            Self::MediaIn => vec![],
            Self::Background { mask, .. }
            | Self::Rectangle { mask, .. }
            | Self::Ellipse { mask, .. }
            | Self::Polygon { mask, .. } => vec![*mask],
            Self::Merge {
                background,
                foreground,
                mask,
                ..
            } => vec![*background, *foreground, *mask],
            Self::Tracker {
                input,
                foreground,
                mask,
                ..
            } => vec![*input, *foreground, *mask],
            Self::Transform { input, mask, .. }
            | Self::Blur { input, mask, .. }
            | Self::Mosaic { input, mask, .. } => vec![*input, *mask],
            Self::Stabilize { input, .. } => vec![*input],
            Self::DeltaKeyer {
                input,
                garbage_matte,
                solid_matte,
                mask,
                ..
            } => vec![*input, *garbage_matte, *solid_matte, *mask],
        };
        inputs.into_iter().flatten().collect()
    }
}

/// The controls that every mask node has, at one moment: each that takes keys as the number
/// that it is there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Masked {
    /// Level: what the mask is multiplied by, from 0 to 1.
    pub level: f64,
    /// Soft Edge: how wide the ramp across the edge is, in widths of the picture.
    pub soft_edge: f64,
    /// Border Width: how thick the line along the edge is, in widths of the picture.
    pub border_width: f64,
    /// Invert: the mask is 1 where the shape is not.
    pub invert: bool,
    /// Solid: the shape is filled. Without it only its border is drawn.
    pub solid: bool,
    /// Paint Mode: how the mask is combined with the one that comes in.
    pub paint_mode: PaintMode,
}

/// The shape of a [`Rectangle`] at one moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RectangleShape {
    /// Where its middle is: `x` and `y`, as [`Center`] counts them.
    pub center: [f64; 2],
    /// Its width, in widths of the picture.
    pub width: f64,
    /// Its height, in heights of the picture.
    pub height: f64,
    /// Corner Radius, from 0 to 1.
    pub corner_radius: f64,
    /// How far it is turned, in degrees counter-clockwise.
    pub angle: f64,
}

/// The shape of an [`Ellipse`] at one moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EllipseShape {
    /// Where its middle is: `x` and `y`, as [`Center`] counts them.
    pub center: [f64; 2],
    /// Its width, in widths of the picture.
    pub width: f64,
    /// Its height, in widths of the picture too.
    pub height: f64,
    /// How far it is turned, in degrees counter-clockwise.
    pub angle: f64,
}

/// The shape of a [`Polygon`] at one moment.
#[derive(Clone, Debug, PartialEq)]
pub struct PolygonShape {
    /// The place that its points are counted from: `x` and `y`, as [`Center`] counts them.
    pub center: [f64; 2],
    /// What its points are multiplied by.
    pub size: f64,
    /// How far it is turned about its centre, in degrees counter-clockwise.
    pub angle: f64,
    /// Its points, each `[x, y]` from the centre, `x` to the right and `y` up, in widths of
    /// the picture.
    pub points: Vec<[f64; 2]>,
    /// Fill Method.
    pub fill: FillMethod,
}

/// The settings of a [`Merge`] at one moment, each that takes keys as the number that it is
/// there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Merged {
    /// Apply Mode.
    pub apply: Composite,
    /// Operator.
    pub operator: Operator,
    /// Subtractive/Additive: from 0, Subtractive, to 1, Additive.
    pub subtractive_additive: f64,
    /// Alpha Gain.
    pub alpha_gain: f64,
    /// Burn In.
    pub burn_in: f64,
    /// Blend.
    pub blend: f64,
    /// Where the middle of the foreground is in the background: `x` and `y`, as [`Center`]
    /// counts them.
    pub center: [f64; 2],
    /// How many times as large the foreground is made.
    pub size: f64,
    /// How far the foreground is turned, in degrees counter-clockwise.
    pub angle: f64,
}

/// The settings of a [`Transform`] at one moment, each that takes keys as the number that it
/// is there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transformed {
    /// Where the middle of the picture is put: `x` and `y`, as [`Center`] counts them.
    pub center: [f64; 2],
    /// How many times as large it is made.
    pub size: f64,
    /// How far it is turned, in degrees counter-clockwise.
    pub angle: f64,
    /// Whether it is flipped left for right.
    pub flip_horizontal: bool,
    /// Whether it is flipped top for bottom.
    pub flip_vertical: bool,
}

/// The settings of a [`Blur`] at one moment, each that takes keys as the number that it is
/// there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Blurred {
    /// Filter.
    pub filter: BlurFilter,
    /// The Blur Size as it is read, across and down, in pixels of a picture 1080 pixels high:
    /// of a Gaussian its standard deviation, of a Box its radius. With Lock X/Y both are the
    /// `size.x` of the node.
    pub size: [f64; 2],
    /// Blend: from 0, the picture as it came in, to 1, the blurred picture.
    pub blend: f64,
}

/// The settings of a [`Mosaic`] at one moment, each that takes keys as the number that it is
/// there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mosaicked {
    /// Pixel Frequency: how many cells lie across the width of the picture.
    pub frequency: f64,
    /// Aspect: how many times as wide as high a cell is.
    pub aspect: f64,
    /// X Offset and Y Offset: how many pixels of the picture the grid is moved, to the right
    /// and down.
    pub offset: [f64; 2],
    /// Whole Cells Only: a cell is shown only where the mask is 1 over all of it.
    pub whole_cells_only: bool,
}

/// The settings of a [`DeltaKeyer`] at one moment, each that takes keys as the number that it
/// is there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeltaKeyed {
    /// Background Color, the colour of the screen: its red, its green and its blue.
    pub screen: [f64; 3],
    /// Gain.
    pub gain: f64,
    /// Balance.
    pub balance: f64,
    /// Pre-Blur, in pixels of a picture 1080 pixels high.
    pub pre_blur: f64,
    /// The lower threshold.
    pub low: f64,
    /// The upper threshold.
    pub high: f64,
    /// Erode/Dilate, in pixels of a picture 1080 pixels high: below 0 it erodes.
    pub erode_dilate: f64,
    /// Matte Blur, in pixels of a picture 1080 pixels high.
    pub matte_blur: f64,
    /// Clean Foreground.
    pub clean_foreground: f64,
    /// Clean Background.
    pub clean_background: f64,
    /// Replace Mode.
    pub replace: ReplaceMode,
    /// Replace Color: its red, its green and its blue.
    pub replace_color: [f64; 3],
    /// Whether the garbage matte is read the other way round.
    pub garbage_invert: bool,
    /// Spill Method.
    pub spill_method: SpillMethod,
    /// Spill Suppression.
    pub spill_suppression: f64,
    /// View Mode: what comes out of the node.
    pub view: KeyerView,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::grade::{Interp, Key};

    fn keys(frames: &[(i64, f64)]) -> Param {
        Param::Keys(
            frames
                .iter()
                .map(|&(frame, value)| Key {
                    frame,
                    value,
                    interp: Interp::Linear,
                })
                .collect(),
        )
    }

    /// A background under the picture of the clip, moved by a Transform: `bg` and `in` into
    /// `over`, whose picture `move` moves into `out`.
    fn merged() -> Composition {
        let mut composition = Composition {
            nodes: vec![
                Node::new("in", Kind::MediaIn),
                Node::new("bg", Kind::Background),
                Node::new("over", Kind::Merge),
                Node::new("move", Kind::Transform),
                Node::new("out", Kind::MediaOut),
            ],
            links: vec![
                Link::new("bg", "over", Input::Background),
                Link::new("in", "over", Input::Foreground),
                Link::new("over", "move", Input::Input),
                Link::new("move", "out", Input::Input),
            ],
        };
        composition.sort_links();
        composition
    }

    #[test]
    fn a_composition_is_at_first_a_media_in_linked_to_a_media_out() {
        let first = Composition::default();
        assert_eq!(first.check(), Ok(()));
        assert_eq!(
            serde_json::to_value(&first).unwrap(),
            json!({
                "nodes": [
                    {"id": "MediaIn1", "kind": "media_in", "label": "", "pass_through": false,
                     "pos": null, "params": {}},
                    {"id": "MediaOut1", "kind": "media_out", "label": "", "pass_through": false,
                     "pos": null, "params": {}},
                ],
                "links": [{"from": "MediaIn1", "to": "MediaOut1", "input": "input"}],
            })
        );
        assert!(!first.is_animated());
        // It shows the clip as it is: the picture of its source goes back to the timeline.
        assert_eq!(
            first.at(0.0),
            Fused {
                nodes: vec![FusedNode {
                    id: "MediaIn1".into(),
                    kind: FusedKind::MediaIn,
                }],
                output: Some(0),
            }
        );
    }

    #[test]
    fn a_node_is_written_with_the_params_of_its_kind_and_read_back() {
        let written = serde_json::to_value(merged()).unwrap();
        assert_eq!(
            written["nodes"][2],
            json!({
                "id": "over", "kind": "merge", "label": "", "pass_through": false, "pos": null,
                "params": {
                    "apply": "normal", "operator": "over", "subtractive_additive": 1.0,
                    "alpha_gain": 1.0, "burn_in": 0.0, "blend": 1.0,
                    "center": {"x": 0.5, "y": 0.5}, "size": 1.0, "angle": 0.0,
                },
            })
        );
        assert_eq!(
            written["nodes"][3]["params"],
            json!({
                "center": {"x": 0.5, "y": 0.5}, "size": 1.0, "angle": 0.0,
                "flip_horizontal": false, "flip_vertical": false,
            })
        );
        assert_eq!(
            written["nodes"][1]["params"],
            json!({"color": {"r": 0.0, "g": 0.0, "b": 0.0, "a": 1.0}})
        );
        assert_eq!(
            written["links"],
            json!([
                {"from": "over", "to": "move", "input": "input"},
                {"from": "move", "to": "out", "input": "input"},
                {"from": "bg", "to": "over", "input": "background"},
                {"from": "in", "to": "over", "input": "foreground"},
            ])
        );
        let read: Composition = serde_json::from_value(written).unwrap();
        assert_eq!(read, merged());
        // Whether it is passed through and where it lies may be left out.
        let short: Node = serde_json::from_value(
            json!({"id": "a", "kind": "media_in", "label": "", "params": {}}),
        )
        .unwrap();
        assert_eq!(short, Node::new("a", Kind::MediaIn));
        // The settings of another kind, a setting that is not known, settings of a kind that
        // has none, and a kind that is not known are refused.
        for wrong in [
            json!({"id": "a", "kind": "media_in", "label": "", "params": {"size": 1.0}}),
            json!({"id": "a", "kind": "transform", "label": "", "params": {}}),
            json!({"id": "a", "kind": "background", "label": "",
                   "params": {"color": {"r": 0, "g": 0, "b": 0, "a": 1}, "type": "solid"}}),
            json!({"id": "a", "kind": "blur", "label": "", "params": {}}),
            json!({"id": "a", "kind": "delta_keyer", "label": "", "params": {}}),
            json!({"id": "a", "kind": "tracker", "label": "", "params": {}}),
            json!({"id": "a", "kind": "text_plus", "label": "", "params": {}}),
            json!({"id": "a", "kind": "media_in", "label": "", "params": {}, "enabled": true}),
        ] {
            assert!(
                serde_json::from_value::<Node>(wrong.clone()).is_err(),
                "{wrong}"
            );
        }
        let link = json!({"from": "a", "to": "b", "input": "effect_mask"});
        assert!(serde_json::from_value::<Link>(link).is_err());
    }

    #[test]
    fn each_kind_has_its_inputs_and_every_kind_but_media_out_an_output() {
        assert_eq!(Kind::MediaIn.inputs(), &[] as &[Input]);
        assert_eq!(Kind::Background.inputs(), &[Input::Mask]);
        assert_eq!(Kind::MediaOut.inputs(), &[Input::Input]);
        assert_eq!(Kind::Transform.inputs(), &[Input::Input, Input::Mask]);
        assert_eq!(
            Kind::Merge.inputs(),
            &[Input::Background, Input::Foreground, Input::Mask]
        );
        // A mask node has the mask that comes into it, and hands it on where it is passed
        // through; the effect mask of another node is not what it hands on.
        for mask in [Kind::Rectangle, Kind::Ellipse, Kind::Polygon] {
            assert_eq!(mask.inputs(), &[Input::Mask]);
            assert_eq!(mask.through(), Some(Input::Mask));
            assert!(mask.is_mask());
        }
        // A Blur and a Mosaic have their picture and their effect mask, as a Transform has,
        // hand on their picture, and make no mask.
        for filter in [Kind::Blur, Kind::Mosaic] {
            assert_eq!(filter.inputs(), &[Input::Input, Input::Mask]);
            assert_eq!(filter.through(), Some(Input::Input));
            assert!(!filter.is_mask());
        }
        // A Delta Keyer has its picture, its two mattes and its effect mask, hands on its
        // picture, and makes no mask.
        assert_eq!(
            Kind::DeltaKeyer.inputs(),
            &[
                Input::Input,
                Input::GarbageMatte,
                Input::SolidMatte,
                Input::Mask
            ]
        );
        assert_eq!(Kind::DeltaKeyer.through(), Some(Input::Input));
        assert!(!Kind::DeltaKeyer.is_mask());
        // A Tracker has the picture that was tracked, the one that follows it and its effect
        // mask, hands on the first, and makes no mask.
        assert_eq!(
            Kind::Tracker.inputs(),
            &[Input::Input, Input::Foreground, Input::Mask]
        );
        assert_eq!(Kind::Tracker.through(), Some(Input::Input));
        assert!(!Kind::Tracker.is_mask());
        assert_eq!(Kind::MediaIn.through(), None);
        assert_eq!(Kind::Background.through(), None);
        assert_eq!(Kind::MediaOut.through(), Some(Input::Input));
        assert_eq!(Kind::Transform.through(), Some(Input::Input));
        assert_eq!(Kind::Merge.through(), Some(Input::Background));
        for kind in [
            Kind::MediaIn,
            Kind::MediaOut,
            Kind::Merge,
            Kind::Transform,
            Kind::Background,
            Kind::Rectangle,
            Kind::Ellipse,
            Kind::Polygon,
            Kind::Blur,
            Kind::Mosaic,
            Kind::DeltaKeyer,
            Kind::Tracker,
        ] {
            // What is handed on is an input of the kind, and its mask comes after its pictures.
            let inputs = kind.inputs();
            if let Some(input) = kind.through() {
                assert!(inputs.contains(&input), "{kind:?}");
            }
            let mask = inputs.iter().position(|input| *input == Input::Mask);
            assert!(mask.is_none() || mask == Some(inputs.len() - 1), "{kind:?}");
            assert_eq!(kind.has_output(), kind != Kind::MediaOut);
            assert_eq!(NodeKind::new(kind).kind(), kind);
            assert_eq!(serde_json::to_value(kind).unwrap(), json!(kind.name()));
            let params = NodeKind::new(kind).params();
            assert_eq!(
                NodeKind::from_params(kind, params).unwrap(),
                NodeKind::new(kind)
            );
            assert_eq!(NodeKind::new(kind).check(), Ok(()));
        }
        for input in [
            Input::Input,
            Input::Background,
            Input::Foreground,
            Input::GarbageMatte,
            Input::SolidMatte,
            Input::Mask,
        ] {
            assert_eq!(serde_json::to_value(input).unwrap(), json!(input.name()));
        }
        assert_eq!(Input::GarbageMatte.name(), "garbage_matte");
        assert_eq!(Input::SolidMatte.name(), "solid_matte");
    }

    #[test]
    fn links_are_kept_in_the_order_of_the_inputs_that_they_go_into() {
        let mut composition = merged();
        composition.links.reverse();
        composition.sort_links();
        let order: Vec<(&str, &str)> = composition
            .links
            .iter()
            .map(|link| (link.to.as_str(), link.input.name()))
            .collect();
        assert_eq!(
            order,
            [
                ("move", "input"),
                ("out", "input"),
                ("over", "background"),
                ("over", "foreground"),
            ]
        );
        assert_eq!(composition, merged());
    }

    #[test]
    fn a_composition_has_one_media_out_and_no_more_nodes_than_the_most() {
        let mut none = merged();
        none.nodes.pop();
        none.links.retain(|link| link.to != "out");
        assert_eq!(none.check(), Err(OpError::OneMediaOut { count: 0 }));
        let mut two = merged();
        two.nodes.push(Node::new("out2", Kind::MediaOut));
        assert_eq!(two.check(), Err(OpError::OneMediaOut { count: 2 }));
        // A MediaOut whose input is not linked is kept: the clip is then transparent.
        let mut unlinked = merged();
        unlinked.links.retain(|link| link.to != "out");
        assert_eq!(unlinked.check(), Ok(()));
        assert_eq!(unlinked.at(0.0), Fused::default());
        // Nor need it have a MediaIn.
        let mut made = merged();
        made.nodes.remove(0);
        made.links.retain(|link| link.from != "in");
        assert_eq!(made.check(), Ok(()));
        let mut many = Composition::default();
        for n in 0..MOST_NODES - 2 {
            many.nodes
                .push(Node::new(format!("b{n}"), Kind::Background));
        }
        assert_eq!(many.check(), Ok(()));
        many.nodes.push(Node::new("one-more", Kind::Background));
        assert_eq!(
            many.check(),
            Err(OpError::OutOfRange {
                what: "fusion_nodes"
            })
        );
    }

    #[test]
    fn the_id_of_a_node_is_its_own_and_has_no_dot_in_it() {
        let with = |id: &str| {
            let mut composition = merged();
            composition.nodes[1].id = id.into();
            composition.links.retain(|link| link.from != "bg");
            composition.check()
        };
        assert_eq!(with("bg2"), Ok(()));
        assert_eq!(with(""), Err(OpError::NoId));
        assert_eq!(with("$bg"), Err(OpError::Dollar { id: "$bg".into() }));
        assert_eq!(with("bg.1"), Err(OpError::OutOfRange { what: "fusion_id" }));
        assert_eq!(
            with("in"),
            Err(OpError::Taken {
                what: "node",
                id: "in".into()
            })
        );
        let mut labelled = merged();
        labelled.nodes[0].label = "x".repeat(crate::grade::LONGEST_LABEL + 1);
        assert_eq!(labelled.check(), Err(OpError::OutOfRange { what: "label" }));
        let mut far = merged();
        far.nodes[0].pos = Some(Pos {
            x: f64::INFINITY,
            y: 0.0,
        });
        assert_eq!(far.check(), Err(OpError::OutOfRange { what: "pos" }));
    }

    #[test]
    fn links_that_go_round_or_into_what_is_not_there_are_refused() {
        let with = |link: Link| {
            let mut composition = merged();
            composition.links.push(link);
            composition.sort_links();
            composition.check()
        };
        let missing = |id: &str| {
            Err(OpError::Missing {
                what: "node",
                id: id.into(),
            })
        };
        assert_eq!(
            with(Link::new("nowhere", "move", Input::Input)),
            missing("nowhere")
        );
        assert_eq!(
            with(Link::new("in", "nowhere", Input::Input)),
            missing("nowhere")
        );
        // A MediaOut has no output, and a kind has the inputs of its kind only.
        assert_eq!(
            with(Link::new("out", "over", Input::Foreground)),
            Err(OpError::NoOutput { node: "out".into() })
        );
        assert_eq!(
            with(Link::new("in", "move", Input::Foreground)),
            Err(OpError::NoInput {
                node: "move".into(),
                input: Input::Foreground
            })
        );
        assert_eq!(
            with(Link::new("in", "bg", Input::Input)),
            Err(OpError::NoInput {
                node: "bg".into(),
                input: Input::Input
            })
        );
        assert_eq!(
            with(Link::new("in", "over", Input::Input)),
            Err(OpError::NoInput {
                node: "over".into(),
                input: Input::Input
            })
        );
        // An input takes one link.
        assert_eq!(
            with(Link::new("in", "over", Input::Background)),
            Err(OpError::InputLinked {
                node: "over".into(),
                input: Input::Background
            })
        );
        // And the links make no loop: not through one node, and not through several.
        let mut round = merged();
        round.links.retain(|link| link.from != "bg");
        round
            .links
            .push(Link::new("move", "over", Input::Background));
        round.sort_links();
        assert!(matches!(round.check(), Err(OpError::Loop { .. })));
        let mut own = merged();
        own.links.retain(|link| link.from != "over");
        own.links.push(Link::new("move", "move", Input::Input));
        own.sort_links();
        assert_eq!(
            own.check(),
            Err(OpError::Loop {
                node: "move".into()
            })
        );
        // One output may feed several inputs.
        let mut twice = merged();
        twice.links.retain(|link| link.from != "bg");
        twice.links.push(Link::new("in", "over", Input::Background));
        twice.sort_links();
        assert_eq!(twice.check(), Ok(()));
    }

    #[test]
    #[allow(clippy::too_many_lines, reason = "a refusal for each setting")]
    fn a_setting_is_within_its_bounds_as_a_number_and_at_each_key() {
        let refused = |change: &dyn Fn(&mut Composition), what: &'static str| {
            let mut composition = merged();
            change(&mut composition);
            assert_eq!(
                composition.check(),
                Err(OpError::OutOfRange { what }),
                "{what}"
            );
        };
        // Each slider of the Merge, by the name that the compositor refuses it by.
        for (at, (what, bounds)) in Merge::SLIDERS.into_iter().enumerate() {
            assert_eq!((bounds.low, bounds.high), (0.0, 1.0), "{what}");
            for value in [-0.001, 1.001, f64::NAN, f64::INFINITY] {
                let set = move |composition: &mut Composition| {
                    let NodeKind::Merge(merge) = &mut composition.nodes[2].kind else {
                        unreachable!("the third node is the merge")
                    };
                    let slider = match at {
                        0 => &mut merge.subtractive_additive,
                        1 => &mut merge.alpha_gain,
                        2 => &mut merge.burn_in,
                        _ => &mut merge.blend,
                    };
                    *slider = Param::Fixed(value);
                };
                refused(&set, what);
            }
        }
        refused(
            &|composition| {
                if let NodeKind::Merge(merge) = &mut composition.nodes[2].kind {
                    merge.center.x = Param::Fixed(17.0);
                }
            },
            "merge.center.x",
        );
        refused(
            &|composition| {
                if let NodeKind::Merge(merge) = &mut composition.nodes[2].kind {
                    merge.center.y = keys(&[(0, 0.5), (10, -16.0)]);
                }
            },
            "merge.center.y",
        );
        refused(
            &|composition| {
                if let NodeKind::Merge(merge) = &mut composition.nodes[2].kind {
                    merge.size = Param::Fixed(-0.5);
                }
            },
            "merge.size",
        );
        refused(
            &|composition| {
                if let NodeKind::Merge(merge) = &mut composition.nodes[2].kind {
                    merge.angle = Param::Fixed(3600.5);
                }
            },
            "merge.angle",
        );
        refused(
            &|composition| {
                if let NodeKind::Transform(transform) = &mut composition.nodes[3].kind {
                    transform.center.x = Param::Fixed(f64::NAN);
                }
            },
            "transform.center.x",
        );
        refused(
            &|composition| {
                if let NodeKind::Transform(transform) = &mut composition.nodes[3].kind {
                    transform.center.y = Param::Fixed(-15.6);
                }
            },
            "transform.center.y",
        );
        refused(
            &|composition| {
                if let NodeKind::Transform(transform) = &mut composition.nodes[3].kind {
                    transform.size = Param::Fixed(16.5);
                }
            },
            "transform.size",
        );
        refused(
            &|composition| {
                if let NodeKind::Transform(transform) = &mut composition.nodes[3].kind {
                    // Keys that are not in the order of their frames.
                    transform.angle = keys(&[(10, 0.0), (10, 90.0)]);
                }
            },
            "transform.angle",
        );
        for (channel, what) in [
            "background.color.r",
            "background.color.g",
            "background.color.b",
            "background.color.a",
        ]
        .into_iter()
        .enumerate()
        {
            refused(
                &move |composition| {
                    if let NodeKind::Background(background) = &mut composition.nodes[1].kind {
                        let (_, setting) = background
                            .settings_mut()
                            .into_iter()
                            .nth(channel)
                            .expect("a colour has four channels");
                        *setting = Param::Fixed(1.5);
                    }
                },
                what,
            );
        }
        // The ends of each range are within it.
        let mut ends = merged();
        if let NodeKind::Merge(merge) = &mut ends.nodes[2].kind {
            merge.blend = Param::Fixed(0.0);
            merge.center.x = Param::Fixed(-15.5);
            merge.center.y = Param::Fixed(16.5);
            merge.size = Param::Fixed(0.0);
            merge.angle = keys(&[(0, -3600.0), (50, 3600.0)]);
        }
        assert_eq!(ends.check(), Ok(()));
    }

    #[test]
    #[allow(clippy::too_many_lines, reason = "the paths of each kind")]
    fn the_settings_that_take_keys_are_named_by_their_paths() {
        let paths = |kind: Kind| NodeKind::new(kind).settings();
        assert_eq!(paths(Kind::MediaIn), [] as [&str; 0]);
        assert_eq!(paths(Kind::MediaOut), [] as [&str; 0]);
        // No number of a Tracker takes keys, with patterns neither: its paths move it.
        assert_eq!(paths(Kind::Tracker), [] as [&str; 0]);
        let mut tracked = NodeKind::Tracker(Tracker {
            trackers: vec![Pattern::new(0.25, 0.75)],
            ..Tracker::default()
        });
        assert_eq!(tracked.settings(), [] as [&str; 0]);
        for path in [
            "reference",
            "pivot_point.x",
            "trackers.0.pattern.x",
            "trackers.0.offset.y",
            "trackers.0.path",
        ] {
            assert!(tracked.setting_mut(path).is_none(), "{path}");
        }
        assert_eq!(
            paths(Kind::Merge),
            [
                "subtractive_additive",
                "alpha_gain",
                "burn_in",
                "blend",
                "center.x",
                "center.y",
                "size",
                "angle"
            ]
        );
        assert_eq!(
            paths(Kind::Transform),
            ["center.x", "center.y", "size", "angle"]
        );
        assert_eq!(
            paths(Kind::Background),
            ["color.r", "color.g", "color.b", "color.a"]
        );
        assert_eq!(
            paths(Kind::Rectangle),
            [
                "level",
                "soft_edge",
                "border_width",
                "center.x",
                "center.y",
                "width",
                "height",
                "corner_radius",
                "angle"
            ]
        );
        assert_eq!(
            paths(Kind::Ellipse),
            [
                "level",
                "soft_edge",
                "border_width",
                "center.x",
                "center.y",
                "width",
                "height",
                "angle"
            ]
        );
        assert_eq!(paths(Kind::Blur), ["size.x", "size.y", "blend"]);
        assert_eq!(
            paths(Kind::Mosaic),
            ["frequency", "aspect", "offset.x", "offset.y"]
        );
        assert_eq!(
            paths(Kind::DeltaKeyer),
            [
                "screen.r",
                "screen.g",
                "screen.b",
                "gain",
                "balance",
                "pre_blur",
                "low",
                "high",
                "erode_dilate",
                "matte_blur",
                "clean_foreground",
                "clean_background",
                "replace_color.r",
                "replace_color.g",
                "replace_color.b",
                "spill_suppression"
            ]
        );
        // A Polygon has the two numbers of each point that it has, after the others.
        let fixed = [
            "level",
            "soft_edge",
            "border_width",
            "center.x",
            "center.y",
            "size",
            "angle",
        ];
        assert_eq!(paths(Kind::Polygon), fixed);
        let mut polygon = NodeKind::Polygon(Polygon {
            points: vec![Point::new(0.0, 0.0), Point::new(0.25, 0.0)],
            ..Polygon::default()
        });
        assert_eq!(polygon.settings()[..7], fixed);
        assert_eq!(
            polygon.settings()[7..],
            ["points.0.x", "points.0.y", "points.1.x", "points.1.y"]
        );
        *polygon.setting_mut("points.1.y").unwrap() = keys(&[(0, 0.0), (10, 0.125)]);
        assert!(polygon.is_animated());
        assert_eq!(
            polygon.params()["points"][1]["y"]["keys"][1]["value"],
            json!(0.125)
        );
        // A point that it has not, a point whole, and a place written another way are none.
        for path in ["points.2.x", "points.1", "points", "points.01.x", "fill"] {
            assert!(polygon.setting_mut(path).is_none(), "{path}");
        }
        let mut merge = NodeKind::new(Kind::Merge);
        *merge.setting_mut("center.y").unwrap() = keys(&[(0, 0.25), (10, 0.75)]);
        assert!(merge.is_animated());
        assert_eq!(
            merge.params()["center"]["y"]["keys"][1],
            json!({"frame": 10, "value": 0.75, "interp": "linear"})
        );
        // What is no number takes no keys, and a kind has its own settings only.
        assert!(merge.setting_mut("apply").is_none());
        assert!(merge.setting_mut("center").is_none());
        assert!(merge.setting_mut("color.r").is_none());
        assert!(NodeKind::new(Kind::Transform)
            .setting_mut("flip_horizontal")
            .is_none());
        assert!(NodeKind::new(Kind::MediaIn).setting_mut("size").is_none());
    }

    #[test]
    fn keys_are_moved_with_the_frames_that_they_count() {
        let mut composition = merged();
        if let NodeKind::Transform(transform) = &mut composition.nodes[3].kind {
            transform.size = keys(&[(10, 1.0), (30, 2.0)]);
        }
        if let NodeKind::Background(background) = &mut composition.nodes[1].kind {
            background.color.a = keys(&[(0, 0.0), (20, 1.0)]);
        }
        assert!(composition.is_animated());
        let later = composition.with_key_frames(&|frame| frame + 5);
        let frames = |param: &Param| match param {
            Param::Keys(keys) => keys.iter().map(|k| k.frame).collect::<Vec<_>>(),
            Param::Fixed(_) => vec![],
        };
        let NodeKind::Transform(transform) = &later.nodes[3].kind else {
            panic!("the fourth node is the transform")
        };
        assert_eq!(frames(&transform.size), [15, 35]);
        assert_eq!(transform.angle, Param::Fixed(0.0));
        let NodeKind::Background(background) = &later.nodes[1].kind else {
            panic!("the second node is the background")
        };
        assert_eq!(frames(&background.color.a), [5, 25]);
        // Nothing else of it is changed, and it shows later what it showed.
        assert_eq!(later.links, composition.links);
        assert_eq!(later.at(25.0), composition.at(20.0));
        assert_eq!(later.check(), Ok(()));
    }

    #[test]
    fn a_composition_at_a_frame_lists_what_its_media_out_reads_each_after_its_inputs() {
        let mut composition = merged();
        // A node that nothing reads, and one that only it reads.
        composition.nodes.push(Node::new("spare", Kind::Transform));
        composition
            .nodes
            .push(Node::new("unseen", Kind::Background));
        composition
            .links
            .push(Link::new("unseen", "spare", Input::Input));
        composition.sort_links();
        if let NodeKind::Merge(merge) = &mut composition.nodes[2].kind {
            merge.blend = keys(&[(0, 0.0), (10, 1.0)]);
            merge.apply = Composite::Screen;
        }
        if let NodeKind::Transform(transform) = &mut composition.nodes[3].kind {
            transform.center.x = keys(&[(0, 0.5), (10, 1.5)]);
            transform.flip_vertical = true;
        }
        assert_eq!(composition.check(), Ok(()));
        let fused = composition.at(5.0);
        let ids: Vec<&str> = fused.nodes.iter().map(|node| node.id.as_str()).collect();
        assert_eq!(ids, ["bg", "in", "over", "move"]);
        assert_eq!(fused.output, Some(3));
        assert_eq!(
            fused.nodes[0].kind,
            FusedKind::Background {
                rgba: [0.0, 0.0, 0.0, 1.0],
                mask: None,
            }
        );
        assert_eq!(fused.nodes[1].kind, FusedKind::MediaIn);
        assert_eq!(
            fused.nodes[2].kind,
            FusedKind::Merge {
                background: Some(0),
                foreground: Some(1),
                mask: None,
                settings: Merged {
                    apply: Composite::Screen,
                    operator: Operator::Over,
                    subtractive_additive: 1.0,
                    alpha_gain: 1.0,
                    burn_in: 0.0,
                    blend: 0.5,
                    center: [0.5, 0.5],
                    size: 1.0,
                    angle: 0.0,
                },
            }
        );
        assert_eq!(
            fused.nodes[3].kind,
            FusedKind::Transform {
                input: Some(2),
                mask: None,
                settings: Transformed {
                    center: [1.0, 0.5],
                    size: 1.0,
                    angle: 0.0,
                    flip_horizontal: false,
                    flip_vertical: true,
                },
            }
        );
        // A node that two inputs read is there once.
        let mut twice = merged();
        twice.links.retain(|link| link.from != "bg");
        twice.links.push(Link::new("in", "over", Input::Background));
        twice.sort_links();
        let fused = twice.at(0.0);
        let ids: Vec<&str> = fused.nodes.iter().map(|node| node.id.as_str()).collect();
        assert_eq!(ids, ["in", "over", "move"]);
        assert!(matches!(
            fused.nodes[1].kind,
            FusedKind::Merge {
                background: Some(0),
                foreground: Some(0),
                ..
            }
        ));
        // An input that is not linked reads no picture.
        let mut bare = merged();
        bare.links.retain(|link| link.from != "in");
        let fused = bare.at(0.0);
        assert!(matches!(
            fused.nodes[1].kind,
            FusedKind::Merge {
                background: Some(0),
                foreground: None,
                ..
            }
        ));
    }

    #[test]
    fn a_node_that_is_passed_through_hands_on_its_first_input() {
        // The merge passed through: its background, and nothing of its foreground.
        let mut composition = merged();
        composition.nodes[2].pass_through = true;
        let fused = composition.at(0.0);
        let ids: Vec<&str> = fused.nodes.iter().map(|node| node.id.as_str()).collect();
        assert_eq!(ids, ["bg", "move"]);
        assert!(matches!(
            fused.nodes[1].kind,
            FusedKind::Transform { input: Some(0), .. }
        ));
        // The transform too: the MediaOut shows the background as it is.
        composition.nodes[3].pass_through = true;
        let fused = composition.at(0.0);
        assert_eq!(fused.nodes.len(), 1);
        assert_eq!((fused.nodes[0].id.as_str(), fused.output), ("bg", Some(0)));
        // A node with no input that is passed through gives no picture.
        composition.nodes[1].pass_through = true;
        assert_eq!(composition.at(0.0), Fused::default());
        // A MediaOut that is passed through shows what it showed.
        let mut out = merged();
        out.nodes[4].pass_through = true;
        assert_eq!(out.at(0.0), merged().at(0.0));
        // And a node passed through whose first input is not linked gives none.
        let mut bare = merged();
        bare.nodes[2].pass_through = true;
        bare.links.retain(|link| link.from != "bg");
        let fused = bare.at(0.0);
        assert_eq!(fused.nodes.len(), 1);
        assert!(matches!(
            fused.nodes[0].kind,
            FusedKind::Transform { input: None, .. }
        ));
    }

    #[test]
    fn a_composition_for_a_node_lists_what_that_node_reads_and_nothing_after_it() {
        let mut composition = merged();
        if let NodeKind::Merge(merge) = &mut composition.nodes[2].kind {
            merge.blend = keys(&[(0, 0.0), (10, 1.0)]);
        }
        let whole = composition.at(5.0);
        assert_eq!(ids(&whole), ["bg", "in", "over", "move"]);
        // The Merge in the middle: itself and what leads to it, each as the whole composition
        // has it at the frame, and not the Transform after it.
        let middle = composition.at_node("over", 5.0).unwrap();
        assert_eq!(ids(&middle), ["bg", "in", "over"]);
        assert_eq!(middle.output, Some(2));
        assert_eq!(middle.nodes[..], whole.nodes[..3]);
        assert!(matches!(
            &middle.nodes[2].kind,
            FusedKind::Merge { settings, .. } if (settings.blend - 0.5).abs() < 1e-12
        ));
        // A node that reads nothing is there alone.
        let first = composition.at_node("in", 5.0).unwrap();
        assert_eq!(ids(&first), ["in"]);
        assert_eq!(first.output, Some(0));
        let colour = composition.at_node("bg", 5.0).unwrap();
        assert_eq!(ids(&colour), ["bg"]);
        assert_eq!(colour.output, Some(0));
        // The last node before the MediaOut, and the MediaOut itself, are the composition.
        assert_eq!(composition.at_node("move", 5.0).unwrap(), whole);
        assert_eq!(composition.at_node("out", 5.0).unwrap(), whole);
        // There is no such node.
        assert_eq!(composition.at_node("n404", 5.0), None);
        assert_eq!(Composition::default().at_node("over", 0.0), None);
        // The composition that a clip has at first: its MediaIn, by either of its nodes.
        let plain = Composition::default();
        assert_eq!(plain.at_node(MEDIA_IN, 0.0), Some(plain.at(0.0)));
        assert_eq!(plain.at_node(MEDIA_OUT, 0.0), Some(plain.at(0.0)));
    }

    #[test]
    fn a_node_that_the_media_out_does_not_read_has_its_picture_all_the_same() {
        // A mask that is not linked yet, and a branch that is not merged yet: a Transform
        // that reads a Background of its own, limited by an Ellipse.
        let mut composition = merged();
        composition.nodes.extend([
            Node::new("r", Kind::Rectangle),
            Node::new("spare", Kind::Transform),
            Node::new("unseen", Kind::Background),
            Node::new("e", Kind::Ellipse),
        ]);
        composition.links.extend([
            Link::new("unseen", "spare", Input::Input),
            Link::new("e", "spare", Input::Mask),
        ]);
        composition.sort_links();
        assert_eq!(composition.check(), Ok(()));
        // None of them is in the composition as the MediaOut shows it.
        assert_eq!(ids(&composition.at(0.0)), ["bg", "in", "over", "move"]);
        let mask = composition.at_node("r", 0.0).unwrap();
        assert_eq!(ids(&mask), ["r"]);
        assert_eq!(mask.output, Some(0));
        assert!(matches!(
            mask.nodes[0].kind,
            FusedKind::Rectangle { mask: None, .. }
        ));
        // The branch: its picture before its mask, and nothing of the rest.
        let branch = composition.at_node("spare", 0.0).unwrap();
        assert_eq!(ids(&branch), ["unseen", "e", "spare"]);
        assert_eq!(branch.output, Some(2));
        assert_eq!(
            branch.nodes[2].kind,
            FusedKind::Transform {
                input: Some(0),
                mask: Some(1),
                settings: Transform::default().at(0.0),
            }
        );
        // A mask node shows the mask that it makes of the one that comes into it.
        let mut combined = masked();
        combined.links.retain(|link| link.to != "over");
        let shown = combined.at_node("e", 0.0).unwrap();
        assert_eq!(ids(&shown), ["r", "e"]);
        assert_eq!(shown.output, Some(1));
        assert_eq!(shown.nodes[1].kind.mask(), Some(0));
    }

    #[test]
    fn a_node_that_is_passed_through_is_viewed_as_what_it_hands_on() {
        // The Merge passed through: its background, and nothing of its foreground.
        let mut composition = merged();
        composition.nodes[2].pass_through = true;
        let handed = composition.at_node("over", 0.0).unwrap();
        assert_eq!(ids(&handed), ["bg"]);
        assert_eq!(handed.output, Some(0));
        // A node after it reads what it hands on, as in the whole composition.
        let after = composition.at_node("move", 0.0).unwrap();
        assert_eq!(ids(&after), ["bg", "move"]);
        assert_eq!(after, composition.at(0.0));
        // A node with no input to hand on gives no picture, and neither does one whose first
        // input is not linked.
        composition.nodes[1].pass_through = true;
        assert_eq!(composition.at_node("bg", 0.0), Some(Fused::default()));
        assert_eq!(composition.at_node("over", 0.0), Some(Fused::default()));
        let mut bare = merged();
        bare.nodes[3].pass_through = true;
        bare.links.retain(|link| link.to != "move");
        assert_eq!(bare.at_node("move", 0.0), Some(Fused::default()));
        // A mask node that is passed through shows the mask that comes into it.
        let mut limited = masked();
        limited.nodes[6].pass_through = true;
        let mask = limited.at_node("e", 0.0).unwrap();
        assert_eq!(ids(&mask), ["r"]);
        assert_eq!(mask.output, Some(0));
    }

    /// The composition of `merged` with its Merge limited by the Ellipse `e`, into which the
    /// mask of the Rectangle `r` comes.
    fn masked() -> Composition {
        let mut composition = merged();
        composition.nodes.push(Node::new("r", Kind::Rectangle));
        composition.nodes.push(Node::new("e", Kind::Ellipse));
        composition.links.push(Link::new("r", "e", Input::Mask));
        composition.links.push(Link::new("e", "over", Input::Mask));
        composition.sort_links();
        composition
    }

    /// The ids of the nodes of a composition at a moment, in their order.
    fn ids(fused: &Fused) -> Vec<&str> {
        fused.nodes.iter().map(|node| node.id.as_str()).collect()
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "each kind of mask as it is at a frame"
    )]
    fn a_mask_is_read_after_the_pictures_of_its_node_and_one_that_is_not_linked_is_none() {
        let mut composition = masked();
        assert_eq!(composition.check(), Ok(()));
        if let NodeKind::Ellipse(ellipse) = &mut composition.nodes[6].kind {
            ellipse.soft_edge = keys(&[(0, 0.0), (10, 0.5)]);
            ellipse.paint_mode = PaintMode::Subtract;
        }
        let fused = composition.at(5.0);
        // The masks are read through `mask` alone, and are there: each after what it reads.
        assert_eq!(ids(&fused), ["bg", "in", "r", "e", "over", "move"]);
        assert_eq!(fused.output, Some(5));
        let read: Vec<Option<usize>> = fused.nodes.iter().map(|node| node.kind.mask()).collect();
        assert_eq!(read, [None, None, None, Some(2), Some(3), None]);
        assert_eq!(
            fused.nodes[3].kind,
            FusedKind::Ellipse {
                mask: Some(2),
                common: Masked {
                    level: 1.0,
                    soft_edge: 0.25,
                    border_width: 0.0,
                    invert: false,
                    solid: true,
                    paint_mode: PaintMode::Subtract,
                },
                shape: EllipseShape {
                    center: [0.5, 0.5],
                    width: 0.5,
                    height: 0.5,
                    angle: 0.0,
                },
            }
        );
        assert_eq!(
            fused.nodes[2].kind,
            FusedKind::Rectangle {
                mask: None,
                common: Masked {
                    level: 1.0,
                    soft_edge: 0.0,
                    border_width: 0.0,
                    invert: false,
                    solid: true,
                    paint_mode: PaintMode::Merge,
                },
                shape: RectangleShape {
                    center: [0.5, 0.5],
                    width: 0.5,
                    height: 0.5,
                    corner_radius: 0.0,
                    angle: 0.0,
                },
            }
        );
        // The first input of a mask node is the mask that comes into it.
        assert_eq!(fused.nodes[3].kind.first_input(), Some(2));
        assert_eq!(fused.nodes[4].kind.first_input(), Some(0));
        // A mask is a picture like another: it may be the foreground of a Merge, and what a
        // `mask` reads may be any picture.
        let mut shown = masked();
        shown.links.retain(|link| link.to != "over");
        shown.links.push(Link::new("e", "over", Input::Foreground));
        shown.links.push(Link::new("in", "over", Input::Mask));
        shown.sort_links();
        assert_eq!(shown.check(), Ok(()));
        let fused = shown.at(0.0);
        assert_eq!(ids(&fused), ["r", "e", "in", "over", "move"]);
        assert!(matches!(
            fused.nodes[3].kind,
            FusedKind::Merge {
                background: None,
                foreground: Some(1),
                mask: Some(2),
                ..
            }
        ));
        // A polygon is there with its points as they are at the frame.
        let mut drawn = masked();
        drawn.nodes[5].kind = NodeKind::Polygon(Polygon {
            points: vec![
                Point::new(-0.25, -0.25),
                Point::new(0.25, -0.25),
                Point {
                    x: Param::Fixed(0.0),
                    y: keys(&[(0, 0.0), (10, 0.5)]),
                },
            ],
            fill: FillMethod::NonZeroWinding,
            ..Polygon::default()
        });
        assert_eq!(drawn.check(), Ok(()));
        let fused = drawn.at(5.0);
        let FusedKind::Polygon { shape, mask, .. } = &fused.nodes[2].kind else {
            panic!("the third node is the polygon")
        };
        assert_eq!(*mask, None);
        assert_eq!(
            *shape,
            PolygonShape {
                center: [0.5, 0.5],
                size: 1.0,
                angle: 0.0,
                points: vec![[-0.25, -0.25], [0.25, -0.25], [0.0, 0.25]],
                fill: FillMethod::NonZeroWinding,
            }
        );
    }

    #[test]
    fn a_mask_node_that_is_passed_through_hands_on_the_mask_that_comes_into_it() {
        // The ellipse passed through: the Merge is limited by the rectangle.
        let mut composition = masked();
        composition.nodes[6].pass_through = true;
        let fused = composition.at(0.0);
        assert_eq!(ids(&fused), ["bg", "in", "r", "over", "move"]);
        assert_eq!(fused.nodes[3].kind.mask(), Some(2));
        // The rectangle too, into which no mask comes: the Merge is not limited.
        composition.nodes[5].pass_through = true;
        let fused = composition.at(0.0);
        assert_eq!(ids(&fused), ["bg", "in", "over", "move"]);
        assert_eq!(fused.nodes[2].kind.mask(), None);
        assert_eq!(fused, merged().at(0.0));
        // A node that is passed through does not hand on its effect mask: a Background that
        // has one gives no picture, and a Transform its picture as it came in.
        let mut limited = masked();
        limited.links.push(Link::new("r", "bg", Input::Mask));
        limited.links.push(Link::new("r", "move", Input::Mask));
        limited.sort_links();
        assert_eq!(limited.check(), Ok(()));
        let fused = limited.at(0.0);
        assert_eq!(ids(&fused), ["r", "bg", "in", "e", "over", "move"]);
        assert_eq!(fused.nodes[1].kind.mask(), Some(0));
        assert_eq!(fused.nodes[5].kind.mask(), Some(0));
        limited.nodes[1].pass_through = true;
        limited.nodes[3].pass_through = true;
        let fused = limited.at(0.0);
        assert_eq!(ids(&fused), ["in", "r", "e", "over"]);
        assert_eq!(fused.output, Some(3));
        assert!(matches!(
            fused.nodes[3].kind,
            FusedKind::Merge {
                background: None,
                foreground: Some(0),
                mask: Some(2),
                ..
            }
        ));
    }

    #[test]
    fn a_mask_is_linked_into_the_kinds_that_have_one_and_makes_no_loop() {
        let with = |link: Link| {
            let mut composition = masked();
            composition
                .links
                .retain(|had| had.target() != link.target());
            composition.links.push(link);
            composition.sort_links();
            composition.check()
        };
        for to in ["bg", "over", "move", "r", "e"] {
            assert_eq!(with(Link::new("in", to, Input::Mask)), Ok(()), "{to}");
        }
        for to in ["in", "out"] {
            assert_eq!(
                with(Link::new("r", to, Input::Mask)),
                Err(OpError::NoInput {
                    node: to.into(),
                    input: Input::Mask
                })
            );
        }
        // A mask node has no input but the mask that comes into it.
        for input in [Input::Input, Input::Background, Input::Foreground] {
            assert_eq!(
                with(Link::new("in", "e", input)),
                Err(OpError::NoInput {
                    node: "e".into(),
                    input
                })
            );
        }
        // A loop through a mask is a loop: of one node, and of several.
        assert_eq!(
            with(Link::new("e", "e", Input::Mask)),
            Err(OpError::Loop { node: "e".into() })
        );
        assert!(matches!(
            with(Link::new("move", "r", Input::Mask)),
            Err(OpError::Loop { .. })
        ));
        // And a mask takes one link.
        let mut twice = masked();
        twice.links.push(Link::new("r", "over", Input::Mask));
        twice.sort_links();
        assert_eq!(
            twice.check(),
            Err(OpError::InputLinked {
                node: "over".into(),
                input: Input::Mask
            })
        );
    }

    #[test]
    fn a_setting_of_a_mask_is_refused_out_of_its_range_by_its_kind_and_its_path() {
        let polygon = NodeKind::Polygon(Polygon {
            points: vec![Point::new(0.0, 0.0)],
            ..Polygon::default()
        });
        for node in [
            NodeKind::new(Kind::Rectangle),
            NodeKind::new(Kind::Ellipse),
            polygon,
        ] {
            let kind = node.kind();
            for path in node.settings() {
                let entry = crate::catalog::of_fusion(kind, &path).expect("it is listed");
                // A point is refused by `points.x` or `points.y`, whichever point it is.
                let what = format!("{}.{}", kind.name(), path.replace(".0.", "."));
                for (value, fits) in [
                    (entry.low, true),
                    (entry.high, true),
                    (entry.low - 0.001, false),
                    (entry.high + 0.001, false),
                    (f64::NAN, false),
                ] {
                    let mut set = node.clone();
                    *set.setting_mut(&path).expect("it takes keys") = Param::Fixed(value);
                    match set.check() {
                        Ok(()) => assert!(fits, "{what} at {value}"),
                        Err(OpError::OutOfRange { what: named }) => {
                            assert!(!fits, "{what} at {value}");
                            assert_eq!(named, what);
                        }
                        Err(other) => panic!("{what} at {value}: {other:?}"),
                    }
                }
                // And at each key: keys that are not in the order of their frames are refused.
                let mut keyed = node.clone();
                *keyed.setting_mut(&path).expect("it takes keys") =
                    keys(&[(5, entry.default), (5, entry.default)]);
                assert!(
                    matches!(keyed.check(), Err(OpError::OutOfRange { .. })),
                    "{what}"
                );
            }
        }
        // A polygon has at most so many points; any fewer will do, none too.
        let mut many = Polygon {
            points: vec![Point::new(0.0, 0.0); MOST_POINTS],
            ..Polygon::default()
        };
        assert_eq!(NodeKind::Polygon(many.clone()).check(), Ok(()));
        many.points.push(Point::new(0.0, 0.0));
        assert_eq!(
            NodeKind::Polygon(many).check(),
            Err(OpError::OutOfRange {
                what: "polygon.points"
            })
        );
        assert_eq!(NodeKind::new(Kind::Polygon).check(), Ok(()));
    }

    #[test]
    fn a_mask_is_written_with_every_setting_and_read_back() {
        let written = |kind: Kind| serde_json::to_value(Node::new("m", kind)).unwrap();
        assert_eq!(
            written(Kind::Rectangle),
            json!({
                "id": "m", "kind": "rectangle", "label": "", "pass_through": false, "pos": null,
                "params": {
                    "level": 1.0, "soft_edge": 0.0, "border_width": 0.0, "invert": false,
                    "solid": true, "paint_mode": "merge", "center": {"x": 0.5, "y": 0.5},
                    "width": 0.5, "height": 0.5, "corner_radius": 0.0, "angle": 0.0,
                },
            })
        );
        assert_eq!(
            written(Kind::Ellipse)["params"],
            json!({
                "level": 1.0, "soft_edge": 0.0, "border_width": 0.0, "invert": false,
                "solid": true, "paint_mode": "merge", "center": {"x": 0.5, "y": 0.5},
                "width": 0.5, "height": 0.5, "angle": 0.0,
            })
        );
        assert_eq!(
            written(Kind::Polygon)["params"],
            json!({
                "level": 1.0, "soft_edge": 0.0, "border_width": 0.0, "invert": false,
                "solid": true, "paint_mode": "merge", "center": {"x": 0.5, "y": 0.5},
                "size": 1.0, "angle": 0.0, "points": [], "fill": "alternate",
            })
        );
        for kind in [Kind::Rectangle, Kind::Ellipse, Kind::Polygon] {
            let read: Node = serde_json::from_value(written(kind)).unwrap();
            assert_eq!(read, Node::new("m", kind));
        }
        // The Paint Modes, the Fill Methods and the points of a polygon as they are written.
        let mut drawn = written(Kind::Polygon);
        drawn["params"]["paint_mode"] = json!("subtract");
        drawn["params"]["fill"] = json!("non_zero_winding");
        drawn["params"]["points"] = json!([
            {"x": -0.25, "y": 0.0},
            {"x": 0.25, "y": {"keys": [{"frame": 0, "value": 0.1, "interp": "hold"}]}},
        ]);
        let read: Node = serde_json::from_value(drawn.clone()).unwrap();
        let NodeKind::Polygon(polygon) = &read.kind else {
            panic!("it is a polygon")
        };
        assert_eq!(polygon.paint_mode, PaintMode::Subtract);
        assert_eq!(polygon.fill, FillMethod::NonZeroWinding);
        assert_eq!(polygon.points[0], Point::new(-0.25, 0.0));
        assert!(polygon.points[1].y.is_animated());
        assert_eq!(serde_json::to_value(&read).unwrap(), drawn);
        assert_eq!(serde_json::to_value(PaintMode::Add).unwrap(), json!("add"));
        // A setting that is left out, one that is not known, and a mode that is not one of
        // the three are refused, as those of the other kinds are.
        let wrong = |change: &dyn Fn(&mut serde_json::Value)| {
            let mut node = written(Kind::Ellipse);
            change(&mut node["params"]);
            serde_json::from_value::<Node>(node).is_err()
        };
        assert!(wrong(&|params| {
            params.as_object_mut().unwrap().remove("solid");
        }));
        assert!(wrong(&|params| params["corner_radius"] = json!(0.0)));
        assert!(wrong(&|params| params["paint_mode"] = json!("maximum")));
        assert!(wrong(&|params| params["invert"] = json!(1)));
        assert!(!wrong(&|params| params["invert"] = json!(true)));
    }

    /// The composition of `merged` with the picture of its Merge blurred by `soft` and that
    /// laid out in cells by `cells`, into `out`; the Ellipse `e` is the effect mask of both.
    fn filtered() -> Composition {
        let mut composition = merged();
        composition.links.retain(|link| link.to != "out");
        composition.nodes.push(Node::new("soft", Kind::Blur));
        composition.nodes.push(Node::new("cells", Kind::Mosaic));
        composition.nodes.push(Node::new("e", Kind::Ellipse));
        composition.links.extend([
            Link::new("move", "soft", Input::Input),
            Link::new("soft", "cells", Input::Input),
            Link::new("cells", "out", Input::Input),
            Link::new("e", "soft", Input::Mask),
            Link::new("e", "cells", Input::Mask),
        ]);
        composition.sort_links();
        composition
    }

    #[test]
    fn a_blur_and_a_mosaic_are_written_with_every_setting_and_read_back() {
        let written = |kind: Kind| serde_json::to_value(Node::new("f", kind)).unwrap();
        assert_eq!(
            written(Kind::Blur),
            json!({
                "id": "f", "kind": "blur", "label": "", "pass_through": false, "pos": null,
                "params": {
                    "filter": "gaussian", "lock_xy": true, "size": {"x": 5.0, "y": 5.0},
                    "blend": 1.0,
                },
            })
        );
        assert_eq!(
            written(Kind::Mosaic)["params"],
            json!({
                "frequency": 32.0, "aspect": 1.0, "offset": {"x": 0.0, "y": 0.0},
                "whole_cells_only": false,
            })
        );
        for kind in [Kind::Blur, Kind::Mosaic] {
            let read: Node = serde_json::from_value(written(kind)).unwrap();
            assert_eq!(read, Node::new("f", kind));
        }
        // The other filter, and keys of a setting, as they are written.
        let mut boxed = written(Kind::Blur);
        boxed["params"]["filter"] = json!("box");
        boxed["params"]["lock_xy"] = json!(false);
        boxed["params"]["size"]["y"] =
            json!({"keys": [{"frame": 0, "value": 12.5, "interp": "hold"}]});
        let read: Node = serde_json::from_value(boxed.clone()).unwrap();
        let NodeKind::Blur(blur) = &read.kind else {
            panic!("it is a blur")
        };
        assert_eq!(blur.filter, BlurFilter::Box);
        assert!(!blur.lock_xy && blur.size.y.is_animated());
        assert_eq!(serde_json::to_value(&read).unwrap(), boxed);
        assert_eq!(BlurFilter::default(), BlurFilter::Gaussian);
        // A setting that is left out, one that is not known, and a filter that is not one of
        // the two are refused, as those of the other kinds are: a Mosaic has no shape to set.
        let wrong = |kind: Kind, change: &dyn Fn(&mut serde_json::Value)| {
            let mut node = written(kind);
            change(&mut node["params"]);
            serde_json::from_value::<Node>(node).is_err()
        };
        assert!(wrong(Kind::Blur, &|params| {
            params.as_object_mut().unwrap().remove("lock_xy");
        }));
        assert!(wrong(Kind::Blur, &|params| {
            params["size"].as_object_mut().unwrap().remove("y");
        }));
        let bartlett = json!("bartlett");
        assert!(wrong(Kind::Blur, &|params| params["filter"] = bartlett.clone()));
        assert!(wrong(Kind::Blur, &|params| params["size"] = json!(5.0)));
        assert!(wrong(Kind::Blur, &|params| params["clipping"] = json!("frame")));
        assert!(wrong(Kind::Mosaic, &|params| {
            params.as_object_mut().unwrap().remove("whole_cells_only");
        }));
        assert!(wrong(Kind::Mosaic, &|params| params["shape"] = json!("square")));
        assert!(wrong(Kind::Mosaic, &|params| params["offset"] = json!(0.0)));
        assert!(wrong(Kind::Mosaic, &|params| params["whole_cells_only"] =
            json!(1)));
        assert!(!wrong(Kind::Mosaic, &|params| params["aspect"] = json!(2)));
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "each of the two as it is at a frame, and as it is passed through"
    )]
    fn a_blur_and_a_mosaic_at_a_frame_are_their_numbers_there_after_what_they_read() {
        let mut composition = filtered();
        assert_eq!(composition.check(), Ok(()));
        if let NodeKind::Blur(blur) = &mut composition.nodes[5].kind {
            blur.size.x = keys(&[(0, 0.0), (10, 20.0)]);
            blur.size.y = Param::Fixed(40.0);
            blur.blend = keys(&[(0, 1.0), (10, 0.5)]);
        }
        if let NodeKind::Mosaic(mosaic) = &mut composition.nodes[6].kind {
            mosaic.frequency = keys(&[(0, 16.0), (10, 48.0)]);
            mosaic.offset.y = Param::Fixed(-3.5);
            mosaic.whole_cells_only = true;
        }
        assert_eq!(composition.check(), Ok(()));
        assert!(composition.is_animated());
        let fused = composition.at(5.0);
        // Each after its picture and then its mask, which is there once though two read it.
        assert_eq!(
            ids(&fused),
            ["bg", "in", "over", "move", "e", "soft", "cells"]
        );
        assert_eq!(fused.output, Some(6));
        // With Lock X/Y the blur down is the blur across, whatever `size.y` is.
        assert_eq!(
            fused.nodes[5].kind,
            FusedKind::Blur {
                input: Some(3),
                mask: Some(4),
                settings: Blurred {
                    filter: BlurFilter::Gaussian,
                    size: [10.0, 10.0],
                    blend: 0.75,
                },
            }
        );
        assert_eq!(
            fused.nodes[6].kind,
            FusedKind::Mosaic {
                input: Some(5),
                mask: Some(4),
                settings: Mosaicked {
                    frequency: 32.0,
                    aspect: 1.0,
                    offset: [0.0, -3.5],
                    whole_cells_only: true,
                },
            }
        );
        assert_eq!(fused.nodes[5].kind.first_input(), Some(3));
        assert_eq!(fused.nodes[6].kind.first_input(), Some(5));
        assert_eq!(fused.nodes[6].kind.mask(), Some(4));
        // Without Lock X/Y each is its own; and the filter is the one that is set.
        if let NodeKind::Blur(blur) = &mut composition.nodes[5].kind {
            blur.lock_xy = false;
            blur.filter = BlurFilter::Box;
        }
        let unlocked = composition.at(5.0);
        let FusedKind::Blur { settings, .. } = &unlocked.nodes[5].kind else {
            panic!("the sixth node is the blur")
        };
        assert_eq!(settings.size, [10.0, 40.0]);
        assert_eq!(settings.filter, BlurFilter::Box);
        // The keys are moved with the frames that they count, as those of the others are.
        let later = composition.with_key_frames(&|frame| frame + 5);
        assert_eq!(later.at(10.0), composition.at(5.0));
        // Passed through, each hands on its picture, and not its mask: the Ellipse, which
        // nothing reads then, is not there.
        composition.nodes[5].pass_through = true;
        let fused = composition.at(5.0);
        assert_eq!(ids(&fused), ["bg", "in", "over", "move", "e", "cells"]);
        assert!(matches!(
            fused.nodes[5].kind,
            FusedKind::Mosaic {
                input: Some(3),
                mask: Some(4),
                ..
            }
        ));
        composition.nodes[6].pass_through = true;
        let fused = composition.at(5.0);
        assert_eq!(ids(&fused), ["bg", "in", "over", "move"]);
        assert_eq!(fused, merged().at(5.0));
        // One whose picture is not linked reads none, and is there still.
        let mut bare = filtered();
        bare.links.retain(|link| link.to != "soft");
        let fused = bare.at(0.0);
        assert_eq!(ids(&fused), ["soft", "e", "cells"]);
        assert!(matches!(
            fused.nodes[0].kind,
            FusedKind::Blur {
                input: None,
                mask: None,
                ..
            }
        ));
    }

    #[test]
    fn a_blur_and_a_mosaic_have_their_picture_and_their_mask_and_no_other_input() {
        let with = |link: Link| {
            let mut composition = filtered();
            composition
                .links
                .retain(|had| had.target() != link.target());
            composition.links.push(link);
            composition.sort_links();
            composition.check()
        };
        for to in ["soft", "cells"] {
            assert_eq!(with(Link::new("in", to, Input::Mask)), Ok(()), "{to}");
            assert_eq!(with(Link::new("bg", to, Input::Input)), Ok(()), "{to}");
            for input in [Input::Background, Input::Foreground] {
                assert_eq!(
                    with(Link::new("in", to, input)),
                    Err(OpError::NoInput {
                        node: to.into(),
                        input
                    })
                );
            }
        }
        // Each has an output, and a loop through either is a loop.
        assert_eq!(with(Link::new("soft", "out", Input::Input)), Ok(()));
        assert!(matches!(
            with(Link::new("cells", "soft", Input::Mask)),
            Err(OpError::Loop { .. })
        ));
        assert_eq!(
            with(Link::new("soft", "soft", Input::Input)),
            Err(OpError::Loop {
                node: "soft".into()
            })
        );
    }

    #[test]
    fn a_setting_of_a_blur_or_a_mosaic_is_refused_out_of_its_range_by_its_kind_and_its_path() {
        for kind in [Kind::Blur, Kind::Mosaic] {
            let node = NodeKind::new(kind);
            for path in node.settings() {
                let entry = crate::catalog::of_fusion(kind, &path).expect("it is listed");
                let what = format!("{}.{path}", kind.name());
                for (value, fits) in [
                    (entry.low, true),
                    (entry.high, true),
                    (entry.low - 0.001, false),
                    (entry.high + 0.001, false),
                    (f64::NAN, false),
                    (f64::INFINITY, false),
                ] {
                    let mut set = node.clone();
                    *set.setting_mut(&path).expect("it takes keys") = Param::Fixed(value);
                    match set.check() {
                        Ok(()) => assert!(fits, "{what} at {value}"),
                        Err(OpError::OutOfRange { what: named }) => {
                            assert!(!fits, "{what} at {value}");
                            assert_eq!(named, what);
                        }
                        Err(other) => panic!("{what} at {value}: {other:?}"),
                    }
                }
                // And at each key: keys that are not in the order of their frames are refused.
                let mut keyed = node.clone();
                *keyed.setting_mut(&path).expect("it takes keys") =
                    keys(&[(5, entry.default), (5, entry.default)]);
                assert!(
                    matches!(keyed.check(), Err(OpError::OutOfRange { .. })),
                    "{what}"
                );
            }
        }
        // `size.y` is held to its range though Lock X/Y leaves it unread; and what is no number
        // takes no keys.
        let mut blur = NodeKind::new(Kind::Blur);
        *blur.setting_mut("size.y").unwrap() = Param::Fixed(500.5);
        assert_eq!(
            blur.check(),
            Err(OpError::OutOfRange {
                what: "blur.size.y"
            })
        );
        for path in ["filter", "lock_xy", "size"] {
            assert!(blur.setting_mut(path).is_none(), "{path}");
        }
        let mut mosaic = NodeKind::new(Kind::Mosaic);
        for path in ["whole_cells_only", "offset", "shape", "size.x"] {
            assert!(mosaic.setting_mut(path).is_none(), "{path}");
        }
        *mosaic.setting_mut("offset.y").unwrap() = keys(&[(0, -16_384.0), (9, 16_384.0)]);
        assert_eq!(mosaic.check(), Ok(()));
        assert!(mosaic.is_animated());
    }

    /// The composition of `merged` with the picture of its Transform keyed by `key`, into
    /// `out`: the Rectangle `g` is its garbage matte, the Ellipse `s` its solid matte and the
    /// Ellipse `e` its effect mask.
    fn keyed() -> Composition {
        let mut composition = merged();
        composition.links.retain(|link| link.to != "out");
        composition.nodes.push(Node::new("key", Kind::DeltaKeyer));
        composition.nodes.push(Node::new("g", Kind::Rectangle));
        composition.nodes.push(Node::new("s", Kind::Ellipse));
        composition.nodes.push(Node::new("e", Kind::Ellipse));
        composition.links.extend([
            Link::new("move", "key", Input::Input),
            Link::new("g", "key", Input::GarbageMatte),
            Link::new("s", "key", Input::SolidMatte),
            Link::new("e", "key", Input::Mask),
            Link::new("key", "out", Input::Input),
        ]);
        composition.sort_links();
        composition
    }

    #[test]
    fn a_delta_keyer_is_written_with_every_setting_and_read_back() {
        let written = serde_json::to_value(Node::new("k", Kind::DeltaKeyer)).unwrap();
        assert_eq!(
            written,
            json!({
                "id": "k", "kind": "delta_keyer", "label": "", "pass_through": false,
                "pos": null,
                "params": {
                    "screen": {"r": 0.0, "g": 1.0, "b": 0.0}, "gain": 1.0, "balance": 0.5,
                    "pre_blur": 0.0, "low": 0.0, "high": 1.0, "erode_dilate": 0.0,
                    "matte_blur": 0.0, "clean_foreground": 0.0, "clean_background": 0.0,
                    "replace": "soft_color", "replace_color": {"r": 0.5, "g": 0.5, "b": 0.5},
                    "garbage_invert": false, "spill_method": "medium",
                    "spill_suppression": 1.0, "view": "final_result",
                },
            })
        );
        let read: Node = serde_json::from_value(written.clone()).unwrap();
        assert_eq!(read, Node::new("k", Kind::DeltaKeyer));
        // Each spelling of what is no number, and keys of a setting, as they are written.
        for (setting, spellings) in [
            (
                "replace",
                &["none", "source", "hard_color", "soft_color"][..],
            ),
            (
                "spill_method",
                &["none", "rare", "medium", "well_done", "burnt"][..],
            ),
            (
                "view",
                &["pre_matte", "matte", "status", "final_result"][..],
            ),
        ] {
            for spelling in spellings {
                let mut node = written.clone();
                node["params"][setting] = json!(spelling);
                let read: Node = serde_json::from_value(node.clone()).unwrap();
                assert_eq!(serde_json::to_value(&read).unwrap(), node, "{setting}");
            }
            let mut node = written.clone();
            node["params"][setting] = json!("other");
            assert!(serde_json::from_value::<Node>(node).is_err(), "{setting}");
        }
        let mut moving = written.clone();
        moving["params"]["screen"]["b"] =
            json!({"keys": [{"frame": 0, "value": 0.25, "interp": "hold"}]});
        moving["params"]["garbage_invert"] = json!(true);
        let read: Node = serde_json::from_value(moving.clone()).unwrap();
        let NodeKind::DeltaKeyer(keyer) = &read.kind else {
            panic!("it is a delta keyer")
        };
        assert!(keyer.garbage_invert && keyer.screen.b.is_animated());
        assert_eq!(serde_json::to_value(&read).unwrap(), moving);
        // A setting that is left out, one that is not known, and one written another way
        // are refused, as those of the other kinds are.
        let wrong = |change: &dyn Fn(&mut serde_json::Value)| {
            let mut node = written.clone();
            change(&mut node["params"]);
            serde_json::from_value::<Node>(node).is_err()
        };
        for setting in [
            "screen",
            "pre_blur",
            "garbage_invert",
            "view",
            "replace_color",
        ] {
            assert!(
                wrong(&|params| {
                    params.as_object_mut().unwrap().remove(setting);
                }),
                "{setting}"
            );
        }
        assert!(wrong(&|params| params["screen"] = json!(0.5)));
        assert!(wrong(&|params| params["screen"]["a"] = json!(1.0)));
        assert!(wrong(&|params| params["fringe_gamma"] = json!(1.0)));
        assert!(wrong(&|params| params["garbage_invert"] = json!(0)));
        assert!(!wrong(&|params| params["gain"] = json!(2)));
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "a keyer as it is at a frame, after what it reads, and as it is passed through"
    )]
    fn a_delta_keyer_at_a_frame_is_its_numbers_there_after_its_picture_its_mattes_and_its_mask() {
        let mut composition = keyed();
        assert_eq!(composition.check(), Ok(()));
        if let NodeKind::DeltaKeyer(keyer) = &mut composition.nodes[5].kind {
            keyer.screen = KeyColor::new(0.125, 0.5, 0.25);
            keyer.gain = keys(&[(0, 1.0), (10, 3.0)]);
            keyer.low = keys(&[(0, 0.0), (10, 0.5)]);
            keyer.replace_color.b = keys(&[(0, 0.5), (10, 1.5)]);
            keyer.erode_dilate = Param::Fixed(-2.0);
            keyer.garbage_invert = true;
            keyer.spill_method = SpillMethod::Burnt;
            keyer.replace = ReplaceMode::HardColor;
            keyer.view = KeyerView::Status;
        }
        assert_eq!(composition.check(), Ok(()));
        assert!(composition.is_animated());
        let fused = composition.at(5.0);
        // Its picture, then its garbage matte, its solid matte and its mask, and then it.
        assert_eq!(
            ids(&fused),
            ["bg", "in", "over", "move", "g", "s", "e", "key"]
        );
        assert_eq!(fused.output, Some(7));
        assert_eq!(
            fused.nodes[7].kind,
            FusedKind::DeltaKeyer {
                input: Some(3),
                garbage_matte: Some(4),
                solid_matte: Some(5),
                mask: Some(6),
                settings: DeltaKeyed {
                    screen: [0.125, 0.5, 0.25],
                    gain: 2.0,
                    balance: 0.5,
                    pre_blur: 0.0,
                    low: 0.25,
                    high: 1.0,
                    erode_dilate: -2.0,
                    matte_blur: 0.0,
                    clean_foreground: 0.0,
                    clean_background: 0.0,
                    replace: ReplaceMode::HardColor,
                    replace_color: [0.5, 0.5, 1.0],
                    garbage_invert: true,
                    spill_method: SpillMethod::Burnt,
                    spill_suppression: 1.0,
                    view: KeyerView::Status,
                },
            }
        );
        assert_eq!(fused.nodes[7].kind.first_input(), Some(3));
        assert_eq!(fused.nodes[7].kind.mask(), Some(6));
        assert_eq!(fused.nodes[7].kind.reads(), [3, 4, 5, 6]);
        assert_eq!(fused.nodes[2].kind.reads(), [0, 1]);
        assert_eq!(fused.nodes[1].kind.reads(), [] as [usize; 0]);
        // The keys are moved with the frames that they count, as those of the others are.
        let later = composition.with_key_frames(&|frame| frame + 5);
        assert_eq!(later.at(10.0), composition.at(5.0));
        // A node that is read as a matte alone is depended on: with no mask and no solid
        // matte, the Rectangle is there still, and the Ellipses are not.
        let mut garbage = composition.clone();
        garbage
            .links
            .retain(|link| link.input != Input::SolidMatte && link.input != Input::Mask);
        let fused = garbage.at(0.0);
        assert_eq!(ids(&fused), ["bg", "in", "over", "move", "g", "key"]);
        assert_eq!(fused.nodes[5].kind.reads(), [3, 4]);
        assert!(matches!(
            fused.nodes[5].kind,
            FusedKind::DeltaKeyer {
                input: Some(3),
                garbage_matte: Some(4),
                solid_matte: None,
                mask: None,
                ..
            }
        ));
        // A matte whose link brings no picture is not there: a mask node that is passed
        // through and has nothing to hand on.
        garbage.nodes[6].pass_through = true;
        let fused = garbage.at(0.0);
        assert_eq!(ids(&fused), ["bg", "in", "over", "move", "key"]);
        assert_eq!(fused.nodes[4].kind.reads(), [3]);
        // Passed through, it hands on its picture, and neither its mattes nor its mask.
        composition.nodes[5].pass_through = true;
        let fused = composition.at(5.0);
        assert_eq!(ids(&fused), ["bg", "in", "over", "move"]);
        assert_eq!(fused, merged().at(5.0));
        // One whose picture is not linked reads none, and is there still.
        let mut bare = keyed();
        bare.links
            .retain(|link| link.target() != ("key", Input::Input));
        let fused = bare.at(0.0);
        assert_eq!(ids(&fused), ["g", "s", "e", "key"]);
        assert_eq!(fused.nodes[3].kind.first_input(), None);
        assert_eq!(fused.nodes[3].kind.reads(), [0, 1, 2]);
    }

    #[test]
    fn a_delta_keyer_has_its_picture_its_two_mattes_and_its_mask_and_they_make_no_loop() {
        let with = |link: Link| {
            let mut composition = keyed();
            composition
                .links
                .retain(|had| had.target() != link.target());
            composition.links.push(link);
            composition.sort_links();
            composition.check()
        };
        // Any picture may be a matte, as any may be a mask.
        for input in [
            Input::Input,
            Input::GarbageMatte,
            Input::SolidMatte,
            Input::Mask,
        ] {
            assert_eq!(with(Link::new("in", "key", input)), Ok(()), "{input:?}");
        }
        for input in [Input::Background, Input::Foreground] {
            assert_eq!(
                with(Link::new("in", "key", input)),
                Err(OpError::NoInput {
                    node: "key".into(),
                    input
                })
            );
        }
        // No other kind has a matte.
        for to in ["in", "bg", "over", "move", "g", "s", "out"] {
            for input in [Input::GarbageMatte, Input::SolidMatte] {
                assert_eq!(
                    with(Link::new("e", to, input)),
                    Err(OpError::NoInput {
                        node: to.into(),
                        input
                    }),
                    "{to}"
                );
            }
        }
        // A loop through a matte is a loop: of one node, and of several.
        assert_eq!(
            with(Link::new("key", "key", Input::GarbageMatte)),
            Err(OpError::Loop { node: "key".into() })
        );
        assert!(matches!(
            with(Link::new("key", "s", Input::Mask)),
            Err(OpError::Loop { .. })
        ));
        // And a matte takes one link.
        let mut twice = keyed();
        twice.links.push(Link::new("e", "key", Input::SolidMatte));
        twice.sort_links();
        assert_eq!(
            twice.check(),
            Err(OpError::InputLinked {
                node: "key".into(),
                input: Input::SolidMatte
            })
        );
        // The links of a keyer are kept by the names of its inputs.
        let order: Vec<&str> = keyed()
            .links
            .iter()
            .filter(|link| link.to == "key")
            .map(|link| link.input.name())
            .collect();
        assert_eq!(order, ["garbage_matte", "input", "mask", "solid_matte"]);
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "a refusal for each setting of a keyer, and for its screen and its thresholds"
    )]
    fn a_setting_of_a_delta_keyer_is_refused_out_of_its_range_by_its_kind_and_its_path() {
        let node = NodeKind::new(Kind::DeltaKeyer);
        assert_eq!(node.check(), Ok(()));
        for path in node.settings() {
            let entry = crate::catalog::of_fusion(Kind::DeltaKeyer, &path).expect("it is listed");
            let what = format!("delta_keyer.{path}");
            for (end, (value, fits)) in [
                (entry.low, true),
                (entry.high, true),
                (entry.low - 0.001, false),
                (entry.high + 0.001, false),
                (f64::NAN, false),
                (f64::INFINITY, false),
            ]
            .into_iter()
            .enumerate()
            {
                let mut set = node.clone();
                *set.setting_mut(&path).expect("it takes keys") = Param::Fixed(value);
                match set.check() {
                    Ok(()) => assert!(fits, "{what} at {value}"),
                    // The green of the screen at its least leaves no channel above the other
                    // two.
                    Err(OpError::OutOfRange {
                        what: "delta_keyer.screen",
                    }) => {
                        assert_eq!((path.as_str(), end), ("screen.g", 0));
                    }
                    Err(OpError::OutOfRange { what: named }) => {
                        assert!(!fits, "{what} at {value}");
                        assert_eq!(named, what);
                    }
                    Err(other) => panic!("{what} at {value}: {other:?}"),
                }
            }
            // And at each key: keys that are not in the order of their frames are refused.
            let mut unordered = node.clone();
            *unordered.setting_mut(&path).expect("it takes keys") =
                keys(&[(5, entry.default), (5, entry.default)]);
            assert!(
                matches!(unordered.check(), Err(OpError::OutOfRange { .. })),
                "{what}"
            );
        }
        let with = |change: &dyn Fn(&mut DeltaKeyer)| {
            let mut keyer = DeltaKeyer::default();
            change(&mut keyer);
            NodeKind::DeltaKeyer(keyer).check()
        };
        let screen = Err(OpError::OutOfRange {
            what: "delta_keyer.screen",
        });
        // A screen colour has one channel above its other two: a grey has none, and neither
        // has one whose two greatest channels are alike.
        for colour in [[0.5, 0.5, 0.5], [0.0, 0.0, 0.0], [0.2, 0.6, 0.6]] {
            let [r, g, b] = colour;
            assert_eq!(
                with(&|keyer| keyer.screen = KeyColor::new(r, g, b)),
                screen,
                "{colour:?}"
            );
        }
        // Any channel may be the one: a blue screen, a red one, and a green one as a camera
        // sees it; and a channel may be above 1.
        for colour in [
            [0.0, 0.0, 1.0],
            [0.7, 0.1, 0.15],
            [0.08, 0.45, 0.12],
            [4.0, 0.5, 0.25],
            [0.2, 0.6, 0.5],
        ] {
            let [r, g, b] = colour;
            assert_eq!(
                with(&|keyer| keyer.screen = KeyColor::new(r, g, b)),
                Ok(()),
                "{colour:?}"
            );
        }
        assert!((dominance([0.2, 0.6, 0.5]) - 0.1).abs() < 1e-6);
        assert_eq!(dominance([0.5, 0.5, 0.5]), 0.0);
        // With keys the screen colour is not checked: at a frame where it has no such
        // channel the keyer keys nothing.
        assert_eq!(
            with(&|keyer| keyer.screen.g = keys(&[(0, 1.0), (10, 0.0)])),
            Ok(())
        );
        // The thresholds are in order, where they have no keys; alike will do.
        let low = Err(OpError::OutOfRange {
            what: "delta_keyer.low",
        });
        assert_eq!(
            with(&|keyer| {
                keyer.low = Param::Fixed(0.6);
                keyer.high = Param::Fixed(0.4);
            }),
            low
        );
        assert_eq!(
            with(&|keyer| {
                keyer.low = Param::Fixed(0.5);
                keyer.high = Param::Fixed(0.5);
            }),
            Ok(())
        );
        assert_eq!(
            with(&|keyer| {
                keyer.low = keys(&[(0, 0.0), (10, 0.9)]);
                keyer.high = Param::Fixed(0.4);
            }),
            Ok(())
        );
        // What is no number takes no keys.
        let mut keyer = NodeKind::new(Kind::DeltaKeyer);
        for path in [
            "screen",
            "replace",
            "replace_color",
            "garbage_invert",
            "spill_method",
            "view",
            "screen.a",
            "size.x",
        ] {
            assert!(keyer.setting_mut(path).is_none(), "{path}");
        }
        *keyer.setting_mut("replace_color.b").unwrap() = keys(&[(0, 0.0), (9, 65_504.0)]);
        assert_eq!(keyer.check(), Ok(()));
        assert!(keyer.is_animated());
    }

    #[test]
    fn a_composition_that_was_not_checked_is_not_followed_round_a_loop() {
        let mut round = merged();
        round.links.retain(|link| link.from != "bg");
        round
            .links
            .push(Link::new("move", "over", Input::Background));
        round.sort_links();
        let fused = round.at(0.0);
        assert_eq!(fused.output, Some(fused.nodes.len() - 1));
        // Passed through all the way round, it gives nothing.
        for node in &mut round.nodes {
            node.pass_through = true;
        }
        assert_eq!(round.at(0.0), Fused::default());
    }

    #[test]
    fn the_pictures_of_a_composition_are_of_the_size_of_the_source_of_its_clip() {
        let mut project = Project::sample();
        let timeline = project.timelines[0].clone();
        let clip = timeline.tracks[0].clips[0].clone();
        project.media[0].facts.video.as_mut().unwrap().width = 1280;
        project.media[0].facts.video.as_mut().unwrap().height = 720;
        assert_eq!(size_of(&clip, &project, &timeline), (1280, 720));
        // Media that is turned a quarter is as high as it is stored wide.
        for (rotation, size) in [(90, (720, 1280)), (180, (1280, 720)), (270, (720, 1280))] {
            project.media[0].facts.video.as_mut().unwrap().rotation = rotation;
            assert_eq!(size_of(&clip, &project, &timeline), size, "{rotation}");
        }
        // A title and a colour are of the size of the timeline.
        let colour = Clip::new(
            "k1",
            Source::Color {
                color: crate::project::Rgba([0, 0, 0, 255]),
            },
            0,
            10,
        );
        assert_eq!(size_of(&colour, &project, &timeline), (1920, 1080));
    }

    /// The size of the pictures of the compositions with a Tracker: not square.
    const TRACKED: (u32, u32) = (64, 32);

    /// A pattern that goes from a quarter of the width and half the height at frame 0 eight
    /// pixels to the right and eight up by frame 8, in pictures of [`TRACKED`].
    fn rising() -> Pattern {
        Pattern {
            path: [(0, [0.25, 0.5]), (8, [0.375, 0.75])].into_iter().collect(),
            ..Pattern::default()
        }
    }

    /// The composition of `merged` with a Tracker `track` of `patterns` that match moves:
    /// the picture of the clip comes into it, the Merge is its foreground and the Ellipse
    /// `e` its mask, and the Transform `move` moves its picture in place of that of the
    /// Merge.
    fn tracked(patterns: Vec<Pattern>) -> Composition {
        let mut composition = merged();
        composition.nodes.push(Node {
            kind: NodeKind::Tracker(Tracker {
                operation: TrackerOperation::MatchMove,
                trackers: patterns,
                ..Tracker::default()
            }),
            ..Node::new("track", Kind::Tracker)
        });
        composition.nodes.push(Node::new("e", Kind::Ellipse));
        composition.links.retain(|link| link.to != "move");
        composition.links.extend([
            Link::new("in", "track", Input::Input),
            Link::new("over", "track", Input::Foreground),
            Link::new("e", "track", Input::Mask),
            Link::new("track", "move", Input::Input),
        ]);
        composition.sort_links();
        composition
    }

    /// The node `id` of a composition, to be changed.
    fn node_mut<'a>(composition: &'a mut Composition, id: &str) -> &'a mut Node {
        composition
            .nodes
            .iter_mut()
            .find(|node| node.id == id)
            .expect("the composition has the node")
    }

    /// Make the centre of the node `id` of a composition follow `follow`, or nothing.
    fn follow(composition: &mut Composition, id: &str, follow: Option<Follow>) {
        let center = match &mut node_mut(composition, id).kind {
            NodeKind::Merge(merge) => &mut merge.center,
            NodeKind::Transform(transform) => &mut transform.center,
            NodeKind::Rectangle(rectangle) => &mut rectangle.center,
            NodeKind::Ellipse(ellipse) => &mut ellipse.center,
            NodeKind::Polygon(polygon) => &mut polygon.center,
            other => panic!("{other:?} has no centre"),
        };
        center.follow = follow;
    }

    /// The centre of the node `id` of a composition at a moment.
    fn center_of(fused: &Fused, id: &str) -> [f64; 2] {
        let node = fused
            .nodes
            .iter()
            .find(|node| node.id == id)
            .expect("the node is drawn");
        match &node.kind {
            FusedKind::Merge { settings, .. } => settings.center,
            FusedKind::Transform { settings, .. } => settings.center,
            FusedKind::Rectangle { shape, .. } => shape.center,
            FusedKind::Ellipse { shape, .. } => shape.center,
            FusedKind::Polygon { shape, .. } => shape.center,
            other => panic!("{other:?} has no centre"),
        }
    }

    #[test]
    fn a_tracker_at_a_frame_is_what_it_draws_after_what_it_reads() {
        let mut composition = tracked(vec![rising()]);
        assert_eq!(composition.check(), Ok(()));
        assert!(composition.is_animated());
        let fused = composition.at_size(8.0, TRACKED);
        // Its picture, its foreground with what that reads, and its mask, each before it.
        assert_eq!(ids(&fused), ["in", "bg", "over", "e", "track", "move"]);
        assert_eq!(fused.output, Some(5));
        // Held still: the picture is put eight pixels to the left and eight down.
        assert_eq!(
            fused.nodes[4].kind,
            FusedKind::Tracker {
                input: Some(0),
                foreground: Some(2),
                mask: Some(3),
                settings: Tracked {
                    operation: TrackerOperation::MatchMove,
                    merge: TrackerMerge::BgOnly,
                    center: [0.375, 0.25],
                    size: 1.0,
                    angle: 0.0,
                },
            }
        );
        assert_eq!(fused.nodes[4].kind.first_input(), Some(0));
        assert_eq!(fused.nodes[4].kind.mask(), Some(3));
        assert_eq!(fused.nodes[4].kind.reads(), [0, 2, 3]);
        // Half way along its path, half as far; and with no size told, the shift of one
        // pattern is the same share of a square picture.
        let half = composition.at_size(4.0, TRACKED);
        let FusedKind::Tracker { settings, .. } = &half.nodes[4].kind else {
            panic!("the fifth node is the tracker")
        };
        assert_eq!(settings.center, [0.4375, 0.375]);
        assert_eq!(composition.at(8.0), fused);
        // The picture of the Tracker alone, and of what leads to it.
        let alone = composition.at_node_size("track", 8.0, TRACKED).unwrap();
        assert_eq!(ids(&alone), ["in", "bg", "over", "e", "track"]);
        assert_eq!(alone.output, Some(4));
        assert_eq!(alone.nodes[..], fused.nodes[..5]);
        assert_eq!(composition.at_node("track", 8.0), Some(alone));
        // Its paths are moved with the keys, and it draws later what it drew.
        let later = composition.with_key_frames(&|frame| frame + 5);
        assert_eq!(later.at_size(13.0, TRACKED), fused);
        assert_eq!(later.check(), Ok(()));
        // Passed through, it hands on the picture that was tracked, and neither its
        // foreground nor its mask.
        node_mut(&mut composition, "track").pass_through = true;
        assert_eq!(ids(&composition.at_size(8.0, TRACKED)), ["in", "move"]);
        // A Tracker with no path is the same at every frame.
        let idle = tracked(vec![Pattern::default()]);
        assert!(!idle.is_animated());
        assert_eq!(idle.at_size(8.0, TRACKED), idle.at_size(0.0, TRACKED));
        assert!(!tracked(vec![]).is_animated());
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "each place that is followed, and each way in which nothing is published"
    )]
    fn a_centre_that_follows_a_tracker_is_where_the_tracker_publishes() {
        let mut pattern = rising();
        pattern.offset = Place { x: 0.125, y: -0.25 };
        let mut composition = tracked(vec![pattern]);
        follow(&mut composition, "e", Some(Follow::tracker("track", 0)));
        let unsteady = Follow::of("track", FollowOutput::UnsteadyPosition);
        follow(&mut composition, "over", Some(unsteady.clone()));
        let steady = Follow::of("track", FollowOutput::SteadyPosition);
        follow(&mut composition, "move", Some(steady.clone()));
        // The own numbers of a centre that follows are kept, and not read.
        if let NodeKind::Ellipse(ellipse) = &mut node_mut(&mut composition, "e").kind {
            ellipse.center.x = Param::Fixed(-3.0);
            ellipse.center.y = keys(&[(0, 2.0), (8, 4.0)]);
        }
        assert_eq!(composition.check(), Ok(()));
        let fused = composition.at_size(8.0, TRACKED);
        // The pattern with its offset, right and up; the middle and the shift of the
        // patterns; and the middle less it.
        assert_eq!(center_of(&fused, "e"), [0.5, 0.5]);
        assert_eq!(center_of(&fused, "over"), [0.625, 0.75]);
        assert_eq!(center_of(&fused, "move"), [0.375, 0.25]);
        let first = Follow::tracker("track", 0);
        assert_eq!(
            composition.published(&first, 8.0, TRACKED),
            Some([0.5, 0.5])
        );
        assert_eq!(
            composition.published(&first, 4.0, TRACKED),
            Some([0.4375, 0.375])
        );
        assert_eq!(
            composition.published(&unsteady, 4.0, TRACKED),
            Some([0.5625, 0.625])
        );
        assert_eq!(
            composition.published(&steady, 4.0, TRACKED),
            Some([0.4375, 0.375])
        );
        // The node that follows reads no picture of the Tracker: it is its numbers.
        let ellipse = fused.nodes.iter().find(|node| node.id == "e").unwrap();
        assert_eq!(ellipse.kind.reads(), [] as [usize; 0]);
        // A node that follows a Tracker with a path changes with the frame.
        for (id, moves) in [("e", true), ("over", true), ("track", true), ("bg", false)] {
            assert_eq!(composition.node_is_animated(id), moves, "{id}");
        }
        assert!(!composition.node_is_animated("n404"));
        // It is written with what it follows, and one that follows nothing as it always was.
        let written = serde_json::to_value(&composition).unwrap();
        assert_eq!(
            written["nodes"][2]["params"]["center"],
            json!({"x": 0.5, "y": 0.5,
                   "follow": {"node": "track", "output": "unsteady_position", "tracker": 0}})
        );
        assert_eq!(written["nodes"][1]["params"].get("center"), None);
        let read: Composition = serde_json::from_value(written).unwrap();
        assert_eq!(read, composition);
        let plain = serde_json::to_string(&Center::middle()).unwrap();
        assert_eq!(plain, r#"{"x":0.5,"y":0.5}"#);
        let null = json!({"x": 0.5, "y": 0.5, "follow": null});
        assert_eq!(
            serde_json::from_value::<Center>(null).unwrap(),
            Center::middle()
        );
        // Where nothing is published the centre is its own numbers: a pattern that is not
        // there, a Tracker that is passed through, and one with no path.
        let own = [-3.0, 4.0];
        let mut beyond = composition.clone();
        follow(&mut beyond, "e", Some(Follow::tracker("track", 3)));
        assert_eq!(beyond.check(), Ok(()));
        assert_eq!(center_of(&beyond.at_size(8.0, TRACKED), "e"), own);
        let mut passed = composition.clone();
        node_mut(&mut passed, "track").pass_through = true;
        assert_eq!(passed.published(&first, 8.0, TRACKED), None);
        assert_eq!(
            passed.at_node_size("e", 8.0, TRACKED).unwrap().nodes.len(),
            1
        );
        let shown = passed.at_node_size("e", 8.0, TRACKED).unwrap();
        assert_eq!(center_of(&shown, "e"), own);
        let mut pathless = composition.clone();
        if let NodeKind::Tracker(tracker) = &mut node_mut(&mut pathless, "track").kind {
            tracker.trackers[0].path.clear();
        }
        let still = pathless.at_size(8.0, TRACKED);
        assert_eq!(center_of(&still, "e"), own);
        assert_eq!(center_of(&still, "over"), [0.5, 0.5]);
        assert!(!pathless.node_is_animated("over") && pathless.node_is_animated("e"));
        // What is published is held within the bounds of the centre that follows it.
        let mut far = composition.clone();
        if let NodeKind::Tracker(tracker) = &mut node_mut(&mut far, "track").kind {
            tracker.trackers[0].path.insert(8, [14.0, -12.0]);
            tracker.trackers[0].offset = Place { x: 0.0, y: 0.0 };
        }
        assert_eq!(far.check(), Ok(()));
        assert_eq!(center_of(&far.at_size(8.0, TRACKED), "e"), [10.0, -10.0]);
        assert_eq!(
            center_of(&far.at_size(8.0, TRACKED), "over"),
            [14.25, -12.0]
        );
        // The node that is followed is a Tracker of the composition, and the pattern one
        // that a Tracker may have.
        let mut nowhere = composition.clone();
        follow(&mut nowhere, "e", Some(Follow::tracker("n404", 0)));
        assert_eq!(
            nowhere.check(),
            Err(OpError::Missing {
                what: "node",
                id: "n404".into()
            })
        );
        let mut wrong = composition.clone();
        follow(&mut wrong, "e", Some(Follow::tracker("over", 0)));
        assert_eq!(
            wrong.check(),
            Err(OpError::WrongNode {
                node: "over".into(),
                kind: "merge"
            })
        );
        let mut many = composition.clone();
        follow(
            &mut many,
            "e",
            Some(Follow::tracker("track", MOST_TRACKERS)),
        );
        assert_eq!(
            many.check(),
            Err(OpError::OutOfRange {
                what: "center.follow.tracker"
            })
        );
        follow(
            &mut many,
            "e",
            Some(Follow::tracker("track", MOST_TRACKERS - 1)),
        );
        assert_eq!(many.check(), Ok(()));
        // Let go of, the centre is its own numbers again.
        follow(&mut composition, "e", None);
        assert_eq!(center_of(&composition.at_size(8.0, TRACKED), "e"), own);
        assert_eq!(composition.check(), Ok(()));
    }
}
