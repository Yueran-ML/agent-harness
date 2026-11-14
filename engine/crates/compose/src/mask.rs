//! The mathematics of the mask nodes of the Fusion page (Reference Manual of Resolve 20, Chapter
//! 108, pp. 2386–2426): what a Rectangle (pp. 2414–2417), an Ellipse (pp. 2396–2399) or a Polygon
//! (pp. 2404–2408) makes of a pixel of the frame, alone: a mask, 1 where the shape is and 0 where
//! it is not, to be the effect mask of another node. It is written on the processor as a shader
//! computes it, in `f32` and a pixel at a time, and is the reference that the shader of the card
//! is to be held to.
//!
//! Three shapes are done (the daily tier), with the controls that every mask node has: Level,
//! Soft Edge, Border Width, Invert, Solid and the Paint Mode (pp. 2397–2398 for the Ellipse,
//! which the pages of the other two repeat: pp. 2405–2407 and 2415–2417).
//!
//! # Places
//!
//! The settings are in the normalised coordinates of Fusion: a place is `[x, y]`, x from 0 at
//! the left of the frame to 1 at its right and y from 0 at the bottom to 1 at the top (the
//! manual has the bottom left corner at 0, 0 and the top right one at 1, 1, p. 1549), so that
//! the middle of the frame is 0.5, 0.5, where a Center is at first (p. 2164). A length is in
//! widths of the frame: the manual has an offset of 0.5 in these coordinates move a thing by
//! half the width of the frame (p. 2668). So a Width of 1 is the width of the frame whatever its
//! shape. What a Height of 1 is depends on the node:
//!
//! - The Rectangle is at first of the aspect ratio of the composition (p. 2414): its Height is
//!   in heights of the frame, and a Width of 1 with a Height of 1 is the whole frame.
//! - The Ellipse is at first a circle (p. 2396): its Height is in widths of the frame, as its
//!   Width is, and a Width and a Height alike are a circle in a frame of any shape.
//! - The points of a Polygon are in widths of the frame both ways (ours: the manual does not
//!   say), so that turning it does not shear it.
//!
//! An Angle is in degrees, and more of it turns the shape counter-clockwise as it is seen, as
//! the Angle of a Transform does (p. 2811; the pages of the masks do not say which way).
//!
//! The maths is in pixels: `[x, y]` with y down, the pixel `p` of an axis lying from `p` to
//! `p + 1` with its centre at `p + 0.5`, as a card has them. Settings are made pixels in one
//! place ([`Shape::placed`] and [`Common::edge`], over the three functions under "From the frame
//! to pixels"): a place `[x, y]` is the pixel place `[x · W, (1 − y) · H]` of a frame of `W` by
//! `H` pixels, a length in widths `l · W`, and one in heights `l · H`. Pixels are square: the
//! Pixel Aspect of the Image tab (p. 2424) is not kept.
//!
//! # The shapes
//!
//! Each shape gives the distance from the centre of a pixel to its edge, in pixels, below 0
//! inside the shape and above 0 outside ([`Placed::distance`]). Everything after reads that
//! distance alone, so that every shape has the same edge.
//!
//! - **Rectangle** ([`rectangle_distance`]): with `h` half its width and height in pixels, `r`
//!   the radius of its corners and `q = |p| − h + r` (of each axis, `p` the pixel from the
//!   centre of the rectangle, turned back by its Angle), the distance is
//!   `length(max(q, 0)) + min(max(q.x, q.y), 0) − r`. It is exact. Corner Radius is from 0, no
//!   rounding, to 1, the most rounding there is (p. 2417): `r` is that share of the lesser of
//!   the two halves, so that 1 makes the short sides half circles.
//! - **Ellipse** ([`ellipse_distance`]): the distance to the nearest point of the ellipse, which
//!   has no formula that a shader would want. The nearest point is found in [`ELLIPSE_STEPS`]
//!   steps, each of which moves a point of the ellipse along the circle that the ellipse
//!   follows there, towards the pixel. A circle is done in one step. Of the ellipses tried, up
//!   to a thousand times as long as wide, each was within a hundredth of a pixel of the true
//!   distance.
//! - **Polygon** ([`polygon_distance`]): the distance to the nearest of its sides, below 0
//!   where the pixel is inside, as the Polygon window of the Color page has it
//!   (`grade/windows.rs`). Inside is by the Fill Method (p. 2407, [`FillMethod`]). Only straight
//!   sides are done.
//!
//! # The edge
//!
//! With `d` the distance of a pixel, `b` half the Border Width in pixels and `s` half the Soft
//! Edge in pixels ([`Edge`]):
//!
//! 1. **Border Width and Solid** ([`drawn`]). In the manual a solid mask is made thicker or
//!    narrower by its border, and one that is not solid is drawn as an outline as thick as the
//!    Border Width (p. 2398). The border is a line of the Border Width (in widths of the frame)
//!    along the edge, half of it on each side (ours: the manual gives neither a unit nor a
//!    side). Solid, the distance is `d − b`: a Border Width above 0 puts the edge out by half of
//!    it and one below 0 puts it in. Not solid, it is `|d| − |b|`: what is within half the
//!    Border Width of the edge, on either side, and so nothing where there is no Border Width.
//!    The border is grown by distance, so that a corner grown outward is rounded by as much.
//! 2. **Soft Edge** ([`feather`]). In the manual the mask is blurred with the filter chosen, and
//!    the edge fades off further beyond the shape as the value grows (p. 2398). It is not
//!    blurred here: the plan of this phase has the masks feathered by distance as the windows
//!    of the Color page are, and the Filter menu (p. 2397) is not kept. The coverage is
//!    `1 − smoothstep(−s, s, d)`: 1 from `s` inside the edge, 0 from `s` outside it and exactly
//!    0.5 on it, as a blur would leave it. The ramp is centred on the edge and is the Soft Edge
//!    wide, in widths of the frame (ours: the manual gives no unit). With no Soft Edge the
//!    coverage is 1 where `d < 0` and 0 elsewhere: a pixel whose centre is on the edge is
//!    outside, so that a shape of no width, or an outline of no Border Width, covers nothing.
//!    Both are `1 − window_feather(d, s, s)` of `grade/windows.wgsl`.
//! 3. **Invert and Level** ([`value`]): `m = Level · c`, or `Level · (1 − c)` where Invert is
//!    ticked, `c` the coverage. In the manual Invert turns the whole mask over, the pixels that
//!    the shape does not cover too (p. 2398), and Level lowers every pixel that the mask
//!    covers, as less Blend does to an effect (p. 2397). It does not say which is first. Ours:
//!    Invert, then Level, so that a Level of 0 is no mask whether inverted or not.
//!
//! # The Paint Mode
//!
//! The Paint Mode is how the mask made in the node is combined with the mask at its effect mask
//! input (p. 2398). With `m` the value of this mask and `i` the incoming one, both within 0 and
//! 1 ([`painted`]):
//!
//! - **Merge** ([`merge`]), which every mask has at first: `i + m · (1 − i)`, this mask laid
//!   over the incoming one. The manual says no more than that the two are merged. Ours: the
//!   over of two alphas, which is neither Add nor Maximum, both of which the menu has besides.
//! - **Add** ([`add`]): `min(i + m, 1)`.
//! - **Subtract** ([`subtract`]): `max(i − m, 0)`.
//!
//! The menu is shown once a mask is connected to the input (p. 2398). With nothing connected
//! there is no Paint Mode, and the result is `m`, whatever the mode is set to.
//!
//! # What is not a number
//!
//! A setting that is no number is read as it is at first, and one out of its range as the
//! nearest that is within it, so that there is a mask whatever is given ([`Bounds`]);
//! [`Common::check`] and [`Shape::check`] refuse both. An incoming value that is no number is
//! read as 0, and one beyond 0 and 1 as the nearer of them. So every value of a mask is a number
//! from 0 to 1.
//!
//! # What is not here
//!
//! - The other mask nodes: `B-Spline`, Triangle, Bitmap, Ranges, Wand and Mask Paint, and the
//!   list of polygons of a `MultiPoly`.
//! - The Bezier handles of the points of a Polygon, its X and Y Rotation (p. 2407) and its
//!   Double Poly (p. 2408): see [`Polygon`].
//! - The other Paint Modes: see [`PaintMode`].
//! - The Filter of the Soft Edge (above), and the Image tab and the Settings tab (pp. 2424–2426):
//!   Output Size, Pixel Aspect, Depth, Clipping Mode and Motion Blur. A mask is of the size of
//!   the frame that it is asked for.
//! - Anything between the pixels: a crisp edge is not antialiased, as that of a window of the
//!   Color page is not. A Soft Edge of a pixel or two is what smooths it.
//!
//! # On the card
//!
//! One fragment for each pixel, which reads nothing but the incoming mask at its own place
//! (`textureLoad`, no sampler). The shader is to be given what [`Shape::placed`] and
//! [`Common::edge`] make, so that no setting is made pixels twice: for a Rectangle or an
//! Ellipse the centre, the cosine and sine of the Angle, the two halves and the radius of the
//! corners; for a Polygon its points in pixels, in the tail of the parameters as the outline of
//! a window is; and half the Soft Edge, half the Border Width, Solid, Invert, Level and the
//! Paint Mode. The cosine and the sine are worked out here, not on the card.
//!
//! `window_feather` and `window_distance` of `grade/windows.wgsl` serve as they are ([`feather`]
//! and the distance to a side). The even-odd test of `window_outline` is the Alternate fill; Non
//! Zero Winding adds a count to the same loop. A polygon of very many points wants the tree of
//! the windows; this reference reads every side.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    reason = "places of pixels, far within the types that they are cast to"
)]

use std::f32::consts::FRAC_1_SQRT_2;

use crate::grade::maths::smoothstep;

/// The distance of a pixel from a shape that is nowhere (a polygon of fewer than three points):
/// beyond any border and any soft edge, and still a number that a shader holds.
pub const FAR: f32 = 1.0e30;

/// How many steps the nearest point of an ellipse is found in ([`ellipse_distance`]).
pub const ELLIPSE_STEPS: u32 = 8;

/// The least half of an ellipse that the maths divides by, in pixels.
const LEAST_AXIS: f32 = 1.0e-3;

/// The least length that the maths divides by.
const TINY: f32 = 1.0e-12;

/// The range of a setting, and what it is at first.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    /// What a new mask has.
    pub default: f32,
    /// The lowest value.
    pub low: f32,
    /// The highest value.
    pub high: f32,
}

impl Bounds {
    /// Check that `value` is a number from `low` to `high`.
    fn check(self, what: &'static str, value: f32) -> Result<(), OutOfRange> {
        if (self.low..=self.high).contains(&value) {
            Ok(())
        } else {
            Err(OutOfRange { what })
        }
    }

    /// `value` as the maths takes it, checked or not: the nearest to it within the range, and
    /// the default where it is no number.
    fn within(self, value: f32) -> f32 {
        if value.is_nan() {
            self.default
        } else {
            value.clamp(self.low, self.high)
        }
    }
}

/// Level (p. 2397), from 0 to 1, at which the manual has the mask opaque. The manual does not
/// say what it is at first. Ours: 1.
pub const LEVEL: Bounds = Bounds {
    default: 1.0,
    low: 0.0,
    high: 1.0,
};

/// Soft Edge (p. 2398), in widths of the frame: how wide the ramp across the edge is. At 0 the
/// edge is crisp, as the manual has it. The manual gives neither a range nor what it is at
/// first. Ours: none at first, and up to a ramp as wide as the frame.
pub const SOFT_EDGE: Bounds = Bounds {
    default: 0.0,
    low: 0.0,
    high: 1.0,
};

/// Border Width (p. 2398), in widths of the frame: how thick the line along the edge is. The
/// manual gives neither a range nor what it is at first. Ours: none at first, and up to the
/// width of the frame, either way.
pub const BORDER_WIDTH: Bounds = Bounds {
    default: 0.0,
    low: -1.0,
    high: 1.0,
};

/// Center X and Y (pp. 2398, 2407, 2417): the middle of the frame at first, as every Center of
/// Fusion is (p. 2164). The manual gives no range. Ours: as far as ten frames either way.
pub const CENTRE: Bounds = Bounds {
    default: 0.5,
    low: -10.0,
    high: 10.0,
};

/// Width and Height (pp. 2398–2399, 2417). The manual gives neither a range nor what they are
/// at first. Ours: 0.5, half the frame, to be set against `DaVinci`; from 0, a shape that
/// covers nothing, to ten frames.
pub const EXTENT: Bounds = Bounds {
    default: 0.5,
    low: 0.0,
    high: 10.0,
};

/// Corner Radius (p. 2417): from 0, no rounding, to 1, the most rounding there is, both as the
/// manual has them. It does not say what it is at first. Ours: 0, sharp corners.
pub const CORNER_RADIUS: Bounds = Bounds {
    default: 0.0,
    low: 0.0,
    high: 1.0,
};

/// Angle (pp. 2399, 2417; the Z Rotation of a Polygon, p. 2407), in degrees, counter-clockwise.
/// The manual gives neither a range nor what it is at first. Ours: 0 at first, and a hundred
/// turns either way.
pub const ANGLE: Bounds = Bounds {
    default: 0.0,
    low: -36_000.0,
    high: 36_000.0,
};

/// The Size of a Polygon (p. 2407): what its points are multiplied by, about its centre. The
/// manual gives neither a range nor what it is at first. Ours: 1 at first, the points as they
/// were drawn, from 0 to 10.
pub const SIZE: Bounds = Bounds {
    default: 1.0,
    low: 0.0,
    high: 10.0,
};

/// An x or a y of a point of a Polygon, from its centre, in widths of the frame. Ours: as far
/// as 16 frames either way, as a point of a window of the Color page is.
pub const POINT: Bounds = Bounds {
    default: 0.0,
    low: -16.0,
    high: 16.0,
};

/// A setting that a `check` refuses: no number, or one outside its range.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("the setting {what} is not a number within its range")]
pub struct OutOfRange {
    /// The setting, as a path: `mask.level`, `mask.soft_edge`, `mask.border_width`,
    /// `mask.centre.x`, `mask.centre.y`, `mask.width`, `mask.height`, `mask.corner_radius`,
    /// `mask.angle`, `mask.size` or `mask.points`.
    pub what: &'static str,
}

/// How the mask of a node is combined with the mask at its effect mask input (Paint Mode,
/// p. 2398). See the module.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PaintMode {
    /// The mask is merged with the incoming one: laid over it ([`merge`]).
    #[default]
    Merge,
    /// The values of the mask are added to those of the incoming one ([`add`]).
    Add,
    /// The values of the mask are taken from those of the incoming one ([`subtract`]).
    Subtract,
    // Minimum, Maximum, Average, Multiply, Replace, Invert, Copy and Ignore (p. 2398) are of the
    // advanced tier and are not done. Each is one more formula of the two values, and one more
    // arm of `painted`: nothing else of the maths reads the mode.
}

/// Which of the regions that the sides of a Polygon part are inside it (Fill Method, p. 2407).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FillMethod {
    /// A region is inside where a line from it to far away crosses the sides an odd number of
    /// times, so that where the polygon lies over itself there is a hole. The manual does not
    /// say which of the two is the first; it has one switch from this to Non Zero Winding to be
    /// rid of such holes, and this is the test of the windows of the Color page.
    #[default]
    Alternate,
    /// A region is inside where the sides go around it at all: no hole where the polygon lies
    /// over itself.
    NonZeroWinding,
}

/// The settings of a Rectangle mask (pp. 2414–2417).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rectangle {
    /// Center X and Y: where its middle is ([`CENTRE`]).
    pub centre: [f32; 2],
    /// Width, in widths of the frame ([`EXTENT`]).
    pub width: f32,
    /// Height, in heights of the frame ([`EXTENT`]).
    pub height: f32,
    /// Corner Radius: the share of the lesser of its half width and half height, in pixels, that
    /// its corners are rounded by ([`CORNER_RADIUS`]).
    pub corner_radius: f32,
    /// Angle, in degrees, counter-clockwise ([`ANGLE`]).
    pub angle: f32,
}

impl Default for Rectangle {
    /// In the middle of the frame, half of it wide and high (and so of the aspect ratio of the
    /// composition, as the manual has a new one, p. 2414), with sharp corners and not turned.
    /// The numbers are ours (see each range).
    fn default() -> Self {
        Self {
            centre: [CENTRE.default; 2],
            width: EXTENT.default,
            height: EXTENT.default,
            corner_radius: CORNER_RADIUS.default,
            angle: ANGLE.default,
        }
    }
}

impl Rectangle {
    /// Check that each setting is a number within its range.
    ///
    /// # Errors
    ///
    /// The first of Center X, Center Y, Width, Height, Corner Radius and Angle that is not.
    pub fn check(&self) -> Result<(), OutOfRange> {
        check_centre(self.centre)?;
        EXTENT.check("mask.width", self.width)?;
        EXTENT.check("mask.height", self.height)?;
        CORNER_RADIUS.check("mask.corner_radius", self.corner_radius)?;
        ANGLE.check("mask.angle", self.angle)
    }
}

/// The settings of an Ellipse mask (pp. 2396–2399).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ellipse {
    /// Center X and Y: where its middle is ([`CENTRE`]).
    pub centre: [f32; 2],
    /// Width, in widths of the frame ([`EXTENT`]).
    pub width: f32,
    /// Height, in widths of the frame too, so that a Width and a Height alike are a circle
    /// ([`EXTENT`]).
    pub height: f32,
    /// Angle, in degrees, counter-clockwise ([`ANGLE`]).
    pub angle: f32,
}

impl Default for Ellipse {
    /// In the middle of the frame, a circle (as the manual has a new one, p. 2396) half the
    /// frame wide, and not turned. The numbers are ours (see each range).
    fn default() -> Self {
        Self {
            centre: [CENTRE.default; 2],
            width: EXTENT.default,
            height: EXTENT.default,
            angle: ANGLE.default,
        }
    }
}

impl Ellipse {
    /// Check that each setting is a number within its range.
    ///
    /// # Errors
    ///
    /// The first of Center X, Center Y, Width, Height and Angle that is not.
    pub fn check(&self) -> Result<(), OutOfRange> {
        check_centre(self.centre)?;
        EXTENT.check("mask.width", self.width)?;
        EXTENT.check("mask.height", self.height)?;
        ANGLE.check("mask.angle", self.angle)
    }
}

/// The settings of a Polygon mask (pp. 2404–2408): a closed outline of straight sides, from each
/// point to the next and from the last to the first.
#[derive(Clone, Debug, PartialEq)]
pub struct Polygon {
    /// Center X and Y: the place that its points are counted from ([`CENTRE`]).
    pub centre: [f32; 2],
    /// Size: what its points are multiplied by ([`SIZE`]).
    pub size: f32,
    /// Z Rotation, in degrees, counter-clockwise about the centre ([`ANGLE`]).
    pub angle: f32,
    // The points of the polyline of the manual are Bezier points, smooth or linear (p. 2408).
    // Room for the handles: each point would have two more places, into it and out of it, from
    // the point itself, and the sides would be flattened into straight ones on the processor
    // before `Shape::placed` (as `grade/curve.rs` flattens the Curve window), so that the maths
    // of a pixel still reads straight sides alone. They are of the advanced tier and are not
    // done; nor are X and Y Rotation, which tilt the outline out of the plane, or the second
    // outline of a Double Poly.
    /// Its points, each `[x, y]` from the centre, x to the right and y up, in widths of the
    /// frame both ways ([`POINT`]). Fewer than three have no outline, and are a mask of nothing.
    pub points: Vec<[f32; 2]>,
    /// Fill Method.
    pub fill: FillMethod,
}

impl Default for Polygon {
    /// No points (a new Polygon has its Center and its Angle alone, p. 2404), about the middle
    /// of the frame, at the size that they are drawn and not turned, with the Alternate fill.
    /// The numbers are ours (see each range).
    fn default() -> Self {
        Self {
            centre: [CENTRE.default; 2],
            size: SIZE.default,
            angle: ANGLE.default,
            points: Vec::new(),
            fill: FillMethod::Alternate,
        }
    }
}

impl Polygon {
    /// Check that each setting is a number within its range. Any number of points will do: a
    /// polygon that is being drawn has fewer than three.
    ///
    /// # Errors
    ///
    /// The first of Center X, Center Y, Size, Angle and the points that is not.
    pub fn check(&self) -> Result<(), OutOfRange> {
        check_centre(self.centre)?;
        SIZE.check("mask.size", self.size)?;
        ANGLE.check("mask.angle", self.angle)?;
        for point in &self.points {
            POINT.check("mask.points", point[0])?;
            POINT.check("mask.points", point[1])?;
        }
        Ok(())
    }
}

/// Check that a Center is two numbers within their range.
fn check_centre(centre: [f32; 2]) -> Result<(), OutOfRange> {
    CENTRE.check("mask.centre.x", centre[0])?;
    CENTRE.check("mask.centre.y", centre[1])
}

/// The shape of a mask node.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Shape {
    /// A Rectangle mask.
    Rectangle(Rectangle),
    /// An Ellipse mask.
    Ellipse(Ellipse),
    /// A Polygon mask.
    Polygon(Polygon),
    // `B-Spline`, Triangle, Bitmap, Ranges, Wand and Mask Paint are not done (see the module).
}

impl Shape {
    /// Check that each setting of the shape is a number within its range.
    ///
    /// # Errors
    ///
    /// The first setting that is not.
    pub fn check(&self) -> Result<(), OutOfRange> {
        match self {
            Shape::Rectangle(shape) => shape.check(),
            Shape::Ellipse(shape) => shape.check(),
            Shape::Polygon(shape) => shape.check(),
        }
    }

    /// The shape laid on a frame of `width` by `height` pixels: its settings made pixels (see
    /// "Places" of the module), as the maths of a pixel reads them and as a shader is to be
    /// given them. Settings that a `check` refuses are taken as the nearest that it allows, and
    /// one that is no number as it is at first.
    #[must_use]
    pub fn placed(&self, width: u32, height: u32) -> Placed {
        match self {
            Shape::Rectangle(shape) => {
                let half = [
                    across(width, EXTENT.within(shape.width)) * 0.5,
                    down(height, EXTENT.within(shape.height)) * 0.5,
                ];
                Placed::Rectangle {
                    centre: place(width, height, shape.centre),
                    turn: turn(shape.angle),
                    half,
                    radius: CORNER_RADIUS.within(shape.corner_radius) * half[0].min(half[1]),
                }
            }
            Shape::Ellipse(shape) => Placed::Ellipse {
                centre: place(width, height, shape.centre),
                turn: turn(shape.angle),
                half: [
                    across(width, EXTENT.within(shape.width)) * 0.5,
                    across(width, EXTENT.within(shape.height)) * 0.5,
                ],
            },
            Shape::Polygon(shape) => {
                let centre = place(width, height, shape.centre);
                let [cosine, sine] = turn(shape.angle);
                let scale = across(width, SIZE.within(shape.size));
                let points = shape
                    .points
                    .iter()
                    .map(|point| {
                        let x = POINT.within(point[0]);
                        let y = POINT.within(point[1]);
                        // Turned counter-clockwise with y up, and then y down as pixels are.
                        [
                            centre[0] + scale * (x * cosine - y * sine),
                            centre[1] - scale * (x * sine + y * cosine),
                        ]
                    })
                    .collect();
                Placed::Polygon {
                    points,
                    fill: shape.fill,
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// From the frame to pixels: the one place where a setting is made pixels
// ---------------------------------------------------------------------------------------------

/// A length of `share` widths of a frame `width` pixels wide, in pixels.
fn across(width: u32, share: f32) -> f32 {
    share * width as f32
}

/// A length of `share` heights of a frame `height` pixels high, in pixels.
fn down(height: u32, share: f32) -> f32 {
    share * height as f32
}

/// A place of the frame (x from the left, y from the bottom, each from 0 to 1 across the frame)
/// in pixels, y down. One that is no number, or beyond [`CENTRE`], is taken as [`Bounds`] has
/// it.
fn place(width: u32, height: u32, point: [f32; 2]) -> [f32; 2] {
    [
        across(width, CENTRE.within(point[0])),
        down(height, 1.0 - CENTRE.within(point[1])),
    ]
}

/// The cosine and the sine of an Angle in degrees.
fn turn(angle: f32) -> [f32; 2] {
    let (sine, cosine) = ANGLE.within(angle).to_radians().sin_cos();
    [cosine, sine]
}

/// A shape laid on a frame, in pixels with y down ([`Shape::placed`]): what the maths of a pixel
/// reads, and what a shader is to be given.
#[derive(Clone, Debug, PartialEq)]
pub enum Placed {
    /// A rectangle.
    Rectangle {
        /// Its middle.
        centre: [f32; 2],
        /// The cosine and the sine of its Angle.
        turn: [f32; 2],
        /// Half its width and half its height.
        half: [f32; 2],
        /// The radius of its corners.
        radius: f32,
    },
    /// An ellipse.
    Ellipse {
        /// Its middle.
        centre: [f32; 2],
        /// The cosine and the sine of its Angle.
        turn: [f32; 2],
        /// Half its width and half its height.
        half: [f32; 2],
    },
    /// A polygon.
    Polygon {
        /// Its points, already moved, sized and turned.
        points: Vec<[f32; 2]>,
        /// Its Fill Method.
        fill: FillMethod,
    },
}

impl Placed {
    /// The distance from `pixel` (a place in pixels, y down: the centre of the pixel `(x, y)` is
    /// `[x + 0.5, y + 0.5]`) to the edge of the shape, in pixels: below 0 inside the shape and
    /// above 0 outside it.
    #[must_use]
    pub fn distance(&self, pixel: [f32; 2]) -> f32 {
        match self {
            Placed::Rectangle {
                centre,
                turn,
                half,
                radius,
            } => rectangle_distance(local(pixel, *centre, *turn), *half, *radius),
            Placed::Ellipse { centre, turn, half } => {
                ellipse_distance(local(pixel, *centre, *turn), *half)
            }
            Placed::Polygon { points, fill } => polygon_distance(pixel, points, *fill),
        }
    }
}

/// `pixel` from `centre`, along the two axes of a shape turned counter-clockwise (as it is
/// seen, y being down) by the angle whose cosine and sine `turn` is. The second axis may be
/// either way up: a rectangle and an ellipse are alike above and below their middle.
fn local(pixel: [f32; 2], centre: [f32; 2], turn: [f32; 2]) -> [f32; 2] {
    let x = pixel[0] - centre[0];
    let y = pixel[1] - centre[1];
    [x * turn[0] - y * turn[1], x * turn[1] + y * turn[0]]
}

/// The length of a vector, as `length` of a shader has it.
fn length(v: [f32; 2]) -> f32 {
    (v[0] * v[0] + v[1] * v[1]).sqrt()
}

/// The distance from `local`, a place from the middle of a rectangle along its two axes, to the
/// edge of the rectangle: below 0 inside. `half` is half its width and half its height and
/// `radius` the radius of its corners, at most the lesser of the two halves; all in pixels.
///
/// With `q = |local| − half + radius` of each axis, it is
/// `length(max(q, 0)) + min(max(q.x, q.y), 0) − radius`: outside, the distance to the nearest
/// side, corner or arc of a corner; inside, less than 0 by the distance to the nearest side.
#[must_use]
pub fn rectangle_distance(local: [f32; 2], half: [f32; 2], radius: f32) -> f32 {
    let beyond = [
        local[0].abs() - half[0] + radius,
        local[1].abs() - half[1] + radius,
    ];
    let outside = length([beyond[0].max(0.0), beyond[1].max(0.0)]);
    let inside = beyond[0].max(beyond[1]).min(0.0);
    outside + inside - radius
}

/// The distance from `local`, a place from the middle of an ellipse along its two axes, to the
/// edge of the ellipse: below 0 inside. `half` is half its width and half its height, in pixels.
///
/// The place is taken into the quarter where both of its numbers are 0 or more (an ellipse is
/// alike in all four). A point of the ellipse is kept as the cosine and the sine of its
/// parameter, at first those of an eighth of a turn, and moved [`ELLIPSE_STEPS`] times, each
/// time along the circle that the ellipse follows there, towards the place. The distance is
/// that from the place to the point, below 0 where `(x / half.x)² + (y / half.y)² < 1`.
///
/// A half below a thousandth of a pixel is taken as that, and an ellipse of no width or no
/// height has no inside: the distance is then to the line that it is, and is never below 0.
#[must_use]
pub fn ellipse_distance(local: [f32; 2], half: [f32; 2]) -> f32 {
    let axes = [half[0].max(LEAST_AXIS), half[1].max(LEAST_AXIS)];
    let point = [local[0].abs(), local[1].abs()];
    let mut turn = [FRAC_1_SQRT_2; 2];
    for _ in 0..ELLIPSE_STEPS {
        turn = ellipse_step(point, axes, turn);
    }
    let gap = length([point[0] - axes[0] * turn[0], point[1] - axes[1] * turn[1]]);
    let across = point[0] / axes[0];
    let down = point[1] / axes[1];
    if half[0] > 0.0 && half[1] > 0.0 && across * across + down * down < 1.0 {
        -gap
    } else {
        gap
    }
}

/// One step towards the point of an ellipse that is nearest to `point`: `turn` is the cosine
/// and the sine of the parameter of a point `e` of the ellipse of the halves `axes`, and the
/// result those of a nearer one. Everything is in the quarter where both numbers are 0 or more.
///
/// The ellipse is taken as the circle that it follows at `e` (of the radius of its curvature
/// there, `|n|³ / (axes.x · axes.y)`, `n = (axes.y · cos, axes.x · sin)` its normal). The point
/// of that circle that is nearest to `point` is `e` moved round the circle by the angle between
/// the normal and the line from the centre of the circle to `point`. It is taken back onto the
/// ellipse by dividing by `axes`, holding within 0 and 1, and making the length 1.
///
/// The centre of the circle is never worked out: along a nearly flat side it is very far away,
/// and an `f32` would lose the pixel in it. The move is worked out from `point − e` alone.
fn ellipse_step(point: [f32; 2], axes: [f32; 2], turn: [f32; 2]) -> [f32; 2] {
    // The normal at `e`, not of length 1, and its length. The tangent is of the same length.
    let normal = [axes[1] * turn[0], axes[0] * turn[1]];
    let scale = length(normal);
    let radius = scale * scale * scale / (axes[0] * axes[1]);
    let from = [point[0] - axes[0] * turn[0], point[1] - axes[1] * turn[1]];
    // `from` along the tangent, and along the normal.
    let along = (from[1] * normal[0] - from[0] * normal[1]) / scale;
    let outward = (from[0] * normal[0] + from[1] * normal[1]) / scale;
    // From the centre of the circle to `point`: `reach` along the normal, `along` across it.
    let reach = radius + outward;
    let far = length([along, reach]).max(TINY);
    let sine = along / far;
    // The cosine of the angle, less 1: without the difference of two near numbers where the
    // centre is far.
    let less = if reach > 0.0 {
        -(along * along) / (far * (far + reach))
    } else {
        reach / far - 1.0
    };
    let moved = [
        axes[0] * turn[0] + radius * (less * normal[0] - sine * normal[1]) / scale,
        axes[1] * turn[1] + radius * (less * normal[1] + sine * normal[0]) / scale,
    ];
    let next = [
        (moved[0] / axes[0]).clamp(0.0, 1.0),
        (moved[1] / axes[1]).clamp(0.0, 1.0),
    ];
    let norm = length(next);
    if norm > TINY {
        [next[0] / norm, next[1] / norm]
    } else {
        // The middle of a circle: every point of it is as near, and the one so far will do.
        turn
    }
}

/// The distance from `pixel` to the side of a polygon from `from` to `to`: `window_distance` of
/// `grade/windows.wgsl`.
fn side_distance(pixel: [f32; 2], from: [f32; 2], to: [f32; 2]) -> f32 {
    let side = [to[0] - from[0], to[1] - from[1]];
    let reach = [pixel[0] - from[0], pixel[1] - from[1]];
    let square = side[0] * side[0] + side[1] * side[1];
    let share = if square > 0.0 {
        ((reach[0] * side[0] + reach[1] * side[1]) / square).clamp(0.0, 1.0)
    } else {
        0.0
    };
    length([reach[0] - share * side[0], reach[1] - share * side[1]])
}

/// The distance from `pixel` to the edge of the polygon of `points` (in pixels, y down; closed
/// from the last to the first): the distance to the nearest of its sides, below 0 where the
/// pixel is inside by `fill`. A polygon of fewer than three points is nowhere: [`FAR`].
///
/// A side is crossed by the line from the pixel to the right where one of its ends is below the
/// pixel and the other is not, and it is to the right of the pixel at the height of the pixel
/// (the test of `window_outline` of `grade/windows.wgsl`). Alternate is inside where the sides
/// so crossed are odd in number; Non Zero Winding where those crossed downwards and those
/// crossed upwards are not as many. Either is the same whichever way round the points are given.
#[must_use]
pub fn polygon_distance(pixel: [f32; 2], points: &[[f32; 2]], fill: FillMethod) -> f32 {
    if points.len() < 3 {
        return FAR;
    }
    let mut nearest = FAR;
    let mut odd = false;
    let mut winding = 0_i32;
    let mut from = points[points.len() - 1];
    for &to in points {
        nearest = nearest.min(side_distance(pixel, from, to));
        if (from[1] > pixel[1]) != (to[1] > pixel[1]) {
            let crossing = (to[0] - from[0]) * (pixel[1] - from[1]) / (to[1] - from[1]) + from[0];
            if pixel[0] < crossing {
                odd = !odd;
                winding += if to[1] > from[1] { 1 } else { -1 };
            }
        }
        from = to;
    }
    let inside = match fill {
        FillMethod::Alternate => odd,
        FillMethod::NonZeroWinding => winding != 0,
    };
    if inside {
        -nearest
    } else {
        nearest
    }
}

/// The controls that every mask node has (pp. 2397–2398). See the module.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Common {
    /// Level: what the mask is multiplied by ([`LEVEL`]).
    pub level: f32,
    /// Soft Edge: how wide the ramp across the edge is, in widths of the frame ([`SOFT_EDGE`]).
    pub soft_edge: f32,
    /// Border Width: how thick the line along the edge is, in widths of the frame
    /// ([`BORDER_WIDTH`]). Solid, it is added to the shape, or taken from it where it is below
    /// 0; not solid, it is all that is drawn, and its sign is not read.
    pub border_width: f32,
    /// Invert: the mask is 1 where the shape is not.
    pub invert: bool,
    /// Solid: the shape is filled. Without it only its border is drawn.
    pub solid: bool,
    /// Paint Mode: how the mask is combined with the incoming one.
    pub paint_mode: PaintMode,
}

impl Default for Common {
    /// A solid mask at its full level, with a crisp edge and no border, not inverted, merged
    /// with the incoming one. Of these the manual gives Merge, which it has every mask start
    /// with (p. 2398); the others are ours, the values that leave the shape as it is drawn.
    fn default() -> Self {
        Self {
            level: LEVEL.default,
            soft_edge: SOFT_EDGE.default,
            border_width: BORDER_WIDTH.default,
            invert: false,
            solid: true,
            paint_mode: PaintMode::Merge,
        }
    }
}

impl Common {
    /// Check that each setting is a number within its range.
    ///
    /// # Errors
    ///
    /// The first of Level, Soft Edge and Border Width that is not.
    pub fn check(&self) -> Result<(), OutOfRange> {
        LEVEL.check("mask.level", self.level)?;
        SOFT_EDGE.check("mask.soft_edge", self.soft_edge)?;
        BORDER_WIDTH.check("mask.border_width", self.border_width)
    }

    /// The edge of a mask in a frame `width` pixels wide: Soft Edge and Border Width made
    /// pixels, and halved, each being half on either side of the edge. Settings that
    /// [`Common::check`] refuses are taken as the nearest that it allows, and one that is no
    /// number as it is at first.
    #[must_use]
    pub fn edge(&self, width: u32) -> Edge {
        Edge {
            soft: across(width, SOFT_EDGE.within(self.soft_edge)) * 0.5,
            border: across(width, BORDER_WIDTH.within(self.border_width)) * 0.5,
            solid: self.solid,
        }
    }
}

/// The edge of a mask, in pixels ([`Common::edge`]): what the maths of a pixel reads of Soft
/// Edge, Border Width and Solid, and what a shader is to be given.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Edge {
    /// Half the width of the ramp across the edge: the coverage is 1 from this far inside the
    /// edge and 0 from this far outside it. 0 or more.
    pub soft: f32,
    /// Half the Border Width, with its sign.
    pub border: f32,
    /// Solid.
    pub solid: bool,
}

/// The distance to the edge of what is drawn of a shape, `distance` being that to the edge of
/// the shape itself (Border Width and Solid, p. 2398): solid, `distance − border`, the shape
/// grown by half the Border Width (or shrunk, below 0); not solid, `|distance| − |border|`, the
/// line of the Border Width along the edge.
#[must_use]
pub fn drawn(distance: f32, edge: &Edge) -> f32 {
    if edge.solid {
        distance - edge.border
    } else {
        distance.abs() - edge.border.abs()
    }
}

/// The coverage of a pixel at `distance` from an edge (below 0 inside), with a ramp of `soft`
/// pixels on either side of the edge (Soft Edge, p. 2398):
/// `1 − smoothstep(−soft, soft, distance)`, which is 1 from `soft` inside, 0 from `soft` outside
/// and 0.5 on the edge; with no ramp, 1 where the distance is below 0 and 0 elsewhere. It is
/// `1 − window_feather(distance, soft, soft)` of `grade/windows.wgsl`.
#[must_use]
pub fn feather(distance: f32, soft: f32) -> f32 {
    if soft > 0.0 {
        1.0 - smoothstep(-soft, soft, distance)
    } else {
        f32::from(distance < 0.0)
    }
}

/// The coverage of a pixel at `distance` from the edge of a shape: [`feather`] of what is
/// [`drawn`] of it.
#[must_use]
pub fn coverage(distance: f32, edge: &Edge) -> f32 {
    feather(drawn(distance, edge), edge.soft)
}

/// `value` as the maths reads a mask: 0 where it is no number, and within 0 and 1.
fn unit(value: f32) -> f32 {
    if value.is_nan() {
        return 0.0;
    }
    value.clamp(0.0, 1.0)
}

/// The value of a mask at a pixel of the coverage `coverage` (Invert, p. 2398, and Level,
/// p. 2397): `level · coverage`, or `level · (1 − coverage)` where it is inverted. A Level that
/// is no number is read as 1, and one beyond 0 and 1 as the nearer of them.
#[must_use]
pub fn value(coverage: f32, level: f32, invert: bool) -> f32 {
    let shown = if invert { 1.0 - coverage } else { coverage };
    unit(shown * LEVEL.within(level))
}

/// Merge (p. 2398): `incoming + own · (1 − incoming)`, the mask of the node laid over the
/// incoming one. An `own` of 0 leaves the incoming mask to the bit, and one of 1 is 1.
#[must_use]
pub fn merge(own: f32, incoming: f32) -> f32 {
    let under = unit(incoming);
    unit(under + unit(own) * (1.0 - under))
}

/// Add (p. 2398): `min(incoming + own, 1)`.
#[must_use]
pub fn add(own: f32, incoming: f32) -> f32 {
    unit(unit(incoming) + unit(own))
}

/// Subtract (p. 2398): `max(incoming − own, 0)`.
#[must_use]
pub fn subtract(own: f32, incoming: f32) -> f32 {
    unit(unit(incoming) - unit(own))
}

/// The mask of a node, `own`, combined with the incoming mask by the Paint Mode `mode`: `own`
/// itself where nothing is connected to the effect mask input of the node, whatever the mode.
/// Both are read within 0 and 1, and as 0 where they are no number.
#[must_use]
pub fn painted(mode: PaintMode, own: f32, incoming: Option<f32>) -> f32 {
    let Some(incoming) = incoming else {
        return unit(own);
    };
    match mode {
        PaintMode::Merge => merge(own, incoming),
        PaintMode::Add => add(own, incoming),
        PaintMode::Subtract => subtract(own, incoming),
    }
}

/// The mask of a node over a frame of `width` by `height` pixels: a value from 0 to 1 for each
/// pixel, row after row from the top. `incoming` is the mask at the effect mask input of the
/// node, of the same size, or none where nothing is connected.
///
/// Each pixel is done as the card does it: the distance from its centre to the edge of the
/// shape ([`Placed::distance`]), the coverage of that distance ([`coverage`]), Invert and Level
/// ([`value`]), and the Paint Mode with the incoming value at the pixel ([`painted`]).
///
/// # Panics
///
/// Where `incoming` is not of `width` by `height` pixels.
#[must_use]
pub fn mask(
    width: u32,
    height: u32,
    shape: &Shape,
    common: &Common,
    incoming: Option<&[f32]>,
) -> Vec<f32> {
    let pixels = width as usize * height as usize;
    if let Some(incoming) = incoming {
        assert_eq!(
            incoming.len(),
            pixels,
            "an incoming mask of the size of the frame"
        );
    }
    let placed = shape.placed(width, height);
    let edge = common.edge(width);
    let mut out = Vec::with_capacity(pixels);
    for y in 0..height {
        for x in 0..width {
            let centre = [x as f32 + 0.5, y as f32 + 0.5];
            let covered = coverage(placed.distance(centre), &edge);
            let own = value(covered, common.level, common.invert);
            let under = incoming.map(|values| values[out.len()]);
            out.push(painted(common.paint_mode, own, under));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::float_cmp,
        reason = "masks of 0 and 1, and numbers that floats keep exactly"
    )]

    use super::*;

    /// The mask of `shape` alone, with the common controls as they are at first.
    fn plain(width: u32, height: u32, shape: &Shape) -> Vec<f32> {
        mask(width, height, shape, &Common::default(), None)
    }

    /// A rectangle in the middle of the frame, `width` by `height`.
    fn rectangle(width: f32, height: f32) -> Rectangle {
        Rectangle {
            width,
            height,
            ..Rectangle::default()
        }
    }

    /// A polygon of `points` about the middle of the frame.
    fn polygon(points: &[[f32; 2]]) -> Polygon {
        Polygon {
            points: points.to_vec(),
            ..Polygon::default()
        }
    }

    /// The value of the pixel `(x, y)` of a mask `width` pixels wide.
    fn at(values: &[f32], width: u32, x: u32, y: u32) -> f32 {
        values[(y * width + x) as usize]
    }

    /// The first and the last column and row that a mask `width` pixels wide is above 0 in:
    /// left, top, right and bottom.
    fn reach(values: &[f32], width: u32) -> [u32; 4] {
        let mut sides = [u32::MAX, u32::MAX, 0, 0];
        for (index, value) in values.iter().enumerate() {
            if *value > 0.0 {
                let x = index as u32 % width;
                let y = index as u32 / width;
                sides = [
                    sides[0].min(x),
                    sides[1].min(y),
                    sides[2].max(x),
                    sides[3].max(y),
                ];
            }
        }
        sides
    }

    /// How many pixels of a mask are 1.
    fn ones(values: &[f32]) -> usize {
        values.iter().filter(|value| **value == 1.0).count()
    }

    #[test]
    fn a_mask_is_at_first_solid_and_whole_with_a_crisp_edge_and_merged() {
        let common = Common::default();
        assert_eq!(common.level, 1.0);
        assert_eq!(common.soft_edge, 0.0);
        assert_eq!(common.border_width, 0.0);
        assert!(!common.invert);
        assert!(common.solid);
        assert_eq!(common.paint_mode, PaintMode::Merge);
        assert_eq!(common.check(), Ok(()));
        let rectangle = Rectangle::default();
        assert_eq!(rectangle.centre, [0.5, 0.5]);
        assert_eq!((rectangle.width, rectangle.height), (0.5, 0.5));
        assert_eq!(rectangle.corner_radius, 0.0);
        assert_eq!(rectangle.angle, 0.0);
        assert_eq!(rectangle.check(), Ok(()));
        let ellipse = Ellipse::default();
        assert_eq!(ellipse.centre, [0.5, 0.5]);
        assert_eq!((ellipse.width, ellipse.height), (0.5, 0.5));
        assert_eq!(ellipse.angle, 0.0);
        assert_eq!(ellipse.check(), Ok(()));
        let polygon = Polygon::default();
        assert_eq!(polygon.centre, [0.5, 0.5]);
        assert_eq!(polygon.size, 1.0);
        assert_eq!(polygon.angle, 0.0);
        assert_eq!(polygon.points, Vec::<[f32; 2]>::new());
        assert_eq!(polygon.fill, FillMethod::Alternate);
        assert_eq!(polygon.check(), Ok(()));
        assert_eq!(PaintMode::default(), PaintMode::Merge);
        assert_eq!(FillMethod::default(), FillMethod::Alternate);
    }

    #[test]
    fn the_centre_of_each_shape_is_1_and_far_outside_it_is_0() {
        let triangle = polygon(&[[-0.25, -0.25], [0.25, -0.25], [0.0, 0.25]]);
        let shapes = [
            Shape::Rectangle(Rectangle::default()),
            Shape::Ellipse(Ellipse::default()),
            Shape::Polygon(triangle),
        ];
        for shape in &shapes {
            assert_eq!(shape.check(), Ok(()), "{shape:?}");
            let got = plain(32, 32, shape);
            assert_eq!(got.len(), 32 * 32);
            // The four pixels about the middle of the frame.
            for (x, y) in [(15, 15), (16, 15), (15, 16), (16, 16)] {
                assert_eq!(at(&got, 32, x, y), 1.0, "{shape:?}");
            }
            for (x, y) in [(0, 0), (31, 0), (0, 31), (31, 31), (16, 0), (0, 16)] {
                assert_eq!(at(&got, 32, x, y), 0.0, "{shape:?}");
            }
            // With no soft edge a mask is of 0 and 1 alone.
            assert!(
                got.iter().all(|value| *value == 0.0 || *value == 1.0),
                "{shape:?}"
            );
        }
    }

    #[test]
    fn a_rectangle_of_width_1_and_height_1_covers_the_frame() {
        for (width, height) in [(16, 9), (9, 16), (1, 1), (64, 2)] {
            let whole = Shape::Rectangle(rectangle(1.0, 1.0));
            let got = plain(width, height, &whole);
            assert_eq!(
                got,
                vec![1.0; (width * height) as usize],
                "{width} by {height}"
            );
        }
        // Half of each is the middle half of the columns and of the rows: its height is in
        // heights of the frame.
        let got = plain(16, 8, &Shape::Rectangle(rectangle(0.5, 0.5)));
        assert_eq!(reach(&got, 16), [4, 2, 11, 5]);
        assert_eq!(ones(&got), 8 * 4);
        // A pixel whose centre is on the edge is outside: three quarters of five pixels are
        // from 0.625 to 4.375, and hold the centres of the pixels 1, 2 and 3.
        let got = plain(5, 1, &Shape::Rectangle(rectangle(0.75, 1.0)));
        assert_eq!(got, [0.0, 1.0, 1.0, 1.0, 0.0]);
        let got = plain(4, 1, &Shape::Rectangle(rectangle(0.25, 1.0)));
        assert_eq!(got, [0.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn a_place_is_from_the_left_and_from_the_bottom_of_the_frame() {
        // A quarter of the way from the right and from the top.
        let moved = Rectangle {
            centre: [0.75, 0.75],
            ..rectangle(0.25, 0.25)
        };
        let got = plain(32, 16, &Shape::Rectangle(moved));
        assert_eq!(reach(&got, 32), [20, 2, 27, 5]);
        assert_eq!(
            Shape::Rectangle(moved).placed(32, 16),
            Placed::Rectangle {
                centre: [24.0, 4.0],
                turn: [1.0, 0.0],
                half: [4.0, 2.0],
                radius: 0.0,
            }
        );
        // The height of an ellipse is in widths of the frame, as its width is.
        let ellipse = Ellipse {
            centre: [0.25, 0.25],
            width: 0.5,
            height: 0.25,
            angle: 0.0,
        };
        assert_eq!(
            Shape::Ellipse(ellipse).placed(32, 16),
            Placed::Ellipse {
                centre: [8.0, 12.0],
                turn: [1.0, 0.0],
                half: [8.0, 4.0],
            }
        );
        // The points of a polygon are from its centre, y up, in widths of the frame.
        let moved = Polygon {
            centre: [0.25, 0.5],
            ..polygon(&[[0.0, 0.0], [0.25, 0.0], [0.0, 0.125]])
        };
        assert_eq!(
            Shape::Polygon(moved).placed(32, 16),
            Placed::Polygon {
                points: vec![[8.0, 8.0], [16.0, 8.0], [8.0, 4.0]],
                fill: FillMethod::Alternate,
            }
        );
    }

    #[test]
    fn a_quarter_turn_swaps_the_extent_of_a_rectangle() {
        let upright = rectangle(0.5, 0.25);
        let got = plain(32, 32, &Shape::Rectangle(upright));
        assert_eq!(reach(&got, 32), [8, 12, 23, 19]);
        assert_eq!(ones(&got), 16 * 8);
        for angle in [90.0, -90.0, 270.0] {
            let turned = Rectangle { angle, ..upright };
            let got = plain(32, 32, &Shape::Rectangle(turned));
            assert_eq!(reach(&got, 32), [12, 8, 19, 23], "{angle}");
            assert_eq!(ones(&got), 16 * 8, "{angle}");
        }
        let turned = Rectangle {
            angle: 180.0,
            ..upright
        };
        assert_eq!(
            plain(32, 32, &Shape::Rectangle(turned)),
            plain(32, 32, &Shape::Rectangle(upright))
        );
        // In a frame twice as wide as high, half of each is 16 pixels by 8, and turned it is 8
        // by 16: a turn is of the pixels, not of the shares of the frame.
        let turned = Rectangle {
            angle: 90.0,
            ..rectangle(0.5, 0.5)
        };
        let got = plain(32, 16, &Shape::Rectangle(turned));
        assert_eq!(reach(&got, 32), [12, 0, 19, 15]);
        assert_eq!(ones(&got), 8 * 16);
    }

    #[test]
    fn more_angle_turns_a_shape_counter_clockwise() {
        // A rectangle 16 pixels by 8 turned an eighth of a turn: its long axis runs from the
        // bottom left to the top right.
        let turned = Rectangle {
            angle: 45.0,
            ..rectangle(0.5, 0.25)
        };
        let got = plain(32, 32, &Shape::Rectangle(turned));
        assert_eq!(at(&got, 32, 20, 11), 1.0);
        assert_eq!(at(&got, 32, 11, 20), 1.0);
        assert_eq!(at(&got, 32, 20, 20), 0.0);
        assert_eq!(at(&got, 32, 11, 11), 0.0);
        // The other way, it runs from the top left to the bottom right.
        let turned = Rectangle {
            angle: -45.0,
            ..rectangle(0.5, 0.25)
        };
        let got = plain(32, 32, &Shape::Rectangle(turned));
        assert_eq!(at(&got, 32, 20, 20), 1.0);
        assert_eq!(at(&got, 32, 20, 11), 0.0);
        // An ellipse likewise.
        let turned = Ellipse {
            height: 0.25,
            angle: 45.0,
            ..Ellipse::default()
        };
        let got = plain(32, 32, &Shape::Ellipse(turned));
        assert_eq!(at(&got, 32, 20, 11), 1.0);
        assert_eq!(at(&got, 32, 20, 20), 0.0);
        // A square of four pixels to the right of the centre of a polygon goes above it.
        let right = polygon(&[
            [0.1875, -0.0625],
            [0.3125, -0.0625],
            [0.3125, 0.0625],
            [0.1875, 0.0625],
        ]);
        let got = plain(32, 32, &Shape::Polygon(right.clone()));
        assert_eq!(reach(&got, 32), [22, 14, 25, 17]);
        let turned = Polygon {
            angle: 90.0,
            ..right
        };
        let got = plain(32, 32, &Shape::Polygon(turned));
        assert_eq!(reach(&got, 32), [14, 6, 17, 9]);
        assert_eq!(ones(&got), 16);
    }

    #[test]
    fn an_ellipse_of_equal_axes_is_a_circle_to_within_half_a_pixel() {
        // In a frame that is not square: a circle of a radius of 10 pixels about (20, 12).
        let got = plain(40, 24, &Shape::Ellipse(Ellipse::default()));
        for y in 0..24_u32 {
            for x in 0..40_u32 {
                let across = x as f32 + 0.5 - 20.0;
                let down = y as f32 + 0.5 - 12.0;
                let radius = (across * across + down * down).sqrt();
                let value = at(&got, 40, x, y);
                if radius < 9.5 {
                    assert_eq!(value, 1.0, "{x}, {y}");
                } else if radius > 10.5 {
                    assert_eq!(value, 0.0, "{x}, {y}");
                }
                // And the distance is that of a circle, far closer than half a pixel.
                let distance = ellipse_distance([across, down], [10.0, 10.0]);
                assert!((distance - (radius - 10.0)).abs() < 1e-3, "{x}, {y}");
            }
        }
        // As many pixels across as down.
        assert_eq!(reach(&got, 40), [10, 2, 29, 21]);
        // Its middle is as far inside as its radius.
        assert!((ellipse_distance([0.0, 0.0], [10.0, 10.0]) + 10.0).abs() < 1e-3);
    }

    #[test]
    fn the_distance_to_an_ellipse_is_that_to_the_nearest_point_of_its_edge() {
        // The nearest of very many points of the edge, in `f64`.
        let nearest = |point: [f32; 2], half: [f32; 2]| {
            let mut least = f64::INFINITY;
            for step in 0..100_000_u32 {
                let turn = f64::from(step) * std::f64::consts::TAU / 100_000.0;
                let across = f64::from(half[0]) * turn.cos() - f64::from(point[0]);
                let down = f64::from(half[1]) * turn.sin() - f64::from(point[1]);
                least = least.min(across.hypot(down));
            }
            least
        };
        let shares = [
            (0.0, 0.0),
            (0.5, 0.0),
            (0.0, 0.5),
            (0.99, 0.0),
            (1.5, 0.0),
            (0.0, 2.0),
            (0.6, 0.6),
            (0.3, 0.9),
            (0.9, 0.3),
            (1.2, 1.2),
            (-0.7, 0.2),
            (0.4, -1.3),
        ];
        for half in [[60.0, 20.0], [20.0, 60.0], [200.0, 2.0], [7.0, 7.0]] {
            for (x, y) in shares {
                let point = [x * half[0], y * half[1]];
                let got = f64::from(ellipse_distance(point, half));
                let want = if x * x + y * y < 1.0 {
                    -nearest(point, half)
                } else {
                    nearest(point, half)
                };
                assert!((got - want).abs() < 0.02, "{half:?}, {point:?}: {got}");
            }
        }
        // An ellipse of no height has no inside: the distance is to the line that it is.
        assert!((ellipse_distance([3.0, 4.0], [8.0, 0.0]) - 4.0).abs() < 0.01);
        assert!(ellipse_distance([3.0, 0.0], [8.0, 0.0]) >= 0.0);
        assert!((ellipse_distance([11.0, 4.0], [8.0, 0.0]) - 5.0).abs() < 0.01);
    }

    #[test]
    fn the_distance_to_a_rectangle_is_that_to_its_nearest_side_or_corner() {
        let half = [8.0, 4.0];
        assert_eq!(rectangle_distance([0.0, 0.0], half, 0.0), -4.0);
        assert_eq!(rectangle_distance([7.0, 0.0], half, 0.0), -1.0);
        assert_eq!(rectangle_distance([-8.0, 1.0], half, 0.0), 0.0);
        assert_eq!(rectangle_distance([10.0, -2.0], half, 0.0), 2.0);
        assert_eq!(rectangle_distance([0.0, 7.0], half, 0.0), 3.0);
        // Beyond a corner, the distance to the corner: 3 and 4 make 5.
        assert_eq!(rectangle_distance([11.0, 8.0], half, 0.0), 5.0);
        assert_eq!(rectangle_distance([-11.0, -8.0], half, 0.0), 5.0);
        // A corner of a radius of 2 is an arc about (6, 2): the corner itself is outside it.
        let outside = rectangle_distance([8.0, 4.0], half, 2.0);
        assert!((outside - (8.0_f32.sqrt() - 2.0)).abs() < 1e-6, "{outside}");
        assert_eq!(rectangle_distance([6.0, 2.0], half, 2.0), -2.0);
        assert_eq!(rectangle_distance([9.0, 0.0], half, 2.0), 1.0);
        assert_eq!(rectangle_distance([0.0, 0.0], half, 2.0), -4.0);
    }

    #[test]
    fn corner_radius_rounds_the_corner_pixel_out() {
        // A square of 16 pixels, from 8 to 24 both ways.
        let rounded = |corner_radius: f32| {
            let shape = Rectangle {
                corner_radius,
                ..Rectangle::default()
            };
            plain(32, 32, &Shape::Rectangle(shape))
        };
        let sharp = rounded(0.0);
        assert_eq!(ones(&sharp), 16 * 16);
        for corner_radius in [0.5, 1.0] {
            let got = rounded(corner_radius);
            for (x, y) in [(8, 8), (23, 8), (8, 23), (23, 23)] {
                assert_eq!(at(&sharp, 32, x, y), 1.0);
                assert_eq!(at(&got, 32, x, y), 0.0, "{corner_radius}");
            }
            // The middles of its sides stay, and nothing is added.
            for (x, y) in [(8, 16), (23, 15), (16, 8), (15, 23), (16, 16)] {
                assert_eq!(at(&got, 32, x, y), 1.0, "{corner_radius}");
            }
            assert_eq!(reach(&got, 32), [8, 8, 23, 23], "{corner_radius}");
            assert!(ones(&got) < ones(&sharp), "{corner_radius}");
        }
        assert!(ones(&rounded(1.0)) < ones(&rounded(0.5)));
        // The most rounding makes a square a circle.
        assert_eq!(
            rounded(1.0),
            plain(32, 32, &Shape::Ellipse(Ellipse::default()))
        );
        // And of a rectangle twice as wide as high, half circles of its short sides: the radius
        // is of the lesser half.
        let stadium = Rectangle {
            corner_radius: 1.0,
            ..rectangle(0.5, 0.25)
        };
        assert_eq!(
            Shape::Rectangle(stadium).placed(32, 32),
            Placed::Rectangle {
                centre: [16.0, 16.0],
                turn: [1.0, 0.0],
                half: [8.0, 4.0],
                radius: 4.0,
            }
        );
    }

    #[test]
    fn a_polygon_given_clockwise_or_counter_clockwise_is_the_same() {
        let points = [
            [-0.3, -0.2],
            [0.1, -0.35],
            [0.35, 0.05],
            [0.05, 0.1],
            [-0.1, 0.4],
        ];
        let mut reversed = points;
        reversed.reverse();
        for fill in [FillMethod::Alternate, FillMethod::NonZeroWinding] {
            let one = Polygon {
                fill,
                ..polygon(&points)
            };
            let other = Polygon {
                fill,
                ..polygon(&reversed)
            };
            let got = plain(48, 48, &Shape::Polygon(one.clone()));
            assert!(ones(&got) > 100, "{fill:?}");
            assert_eq!(
                got,
                plain(48, 48, &Shape::Polygon(other.clone())),
                "{fill:?}"
            );
            // With a soft edge too, to what a float tells apart.
            let soft = Common {
                soft_edge: 0.25,
                ..Common::default()
            };
            let one = mask(48, 48, &Shape::Polygon(one), &soft, None);
            let other = mask(48, 48, &Shape::Polygon(other), &soft, None);
            for (a, b) in one.iter().zip(&other) {
                assert!((a - b).abs() < 1e-5, "{fill:?}: {a} and {b}");
            }
        }
        // A square of points is the rectangle of the same sides.
        let square = polygon(&[[-0.25, -0.25], [0.25, -0.25], [0.25, 0.25], [-0.25, 0.25]]);
        assert_eq!(
            plain(32, 32, &Shape::Polygon(square)),
            plain(32, 32, &Shape::Rectangle(Rectangle::default()))
        );
    }

    #[test]
    fn a_concave_polygon_leaves_its_notch_out() {
        // A square from 8 to 24 with a notch cut into its top, from 12 to 20 across and down to
        // the middle.
        let notched = polygon(&[
            [-0.25, -0.25],
            [0.25, -0.25],
            [0.25, 0.25],
            [0.125, 0.25],
            [0.125, 0.0],
            [-0.125, 0.0],
            [-0.125, 0.25],
            [-0.25, 0.25],
        ]);
        for fill in [FillMethod::Alternate, FillMethod::NonZeroWinding] {
            let shape = Polygon {
                fill,
                ..notched.clone()
            };
            let got = plain(32, 32, &Shape::Polygon(shape));
            for y in 0..32_u32 {
                for x in 0..32_u32 {
                    let square = (8..24).contains(&x) && (8..24).contains(&y);
                    let notch = (12..20).contains(&x) && (8..16).contains(&y);
                    let want = f32::from(square && !notch);
                    assert_eq!(at(&got, 32, x, y), want, "{fill:?}: {x}, {y}");
                }
            }
        }
        // Inside the notch the nearest side is a side of the notch.
        let Placed::Polygon { points, fill } = Shape::Polygon(notched).placed(32, 32) else {
            panic!("a polygon is placed as a polygon");
        };
        assert_eq!(polygon_distance([16.0, 12.0], &points, fill), 4.0);
        assert_eq!(polygon_distance([16.0, 20.0], &points, fill), -4.0);
        assert_eq!(polygon_distance([13.0, 9.0], &points, fill), 1.0);
        assert_eq!(polygon_distance([4.0, 16.0], &points, fill), 4.0);
    }

    #[test]
    fn where_a_polygon_lies_over_itself_alternate_leaves_a_hole() {
        // A star of five points drawn in one line, each point to the next but one: the line
        // goes twice around its middle.
        let points: Vec<[f32; 2]> = (0..5_u8)
            .map(|point| {
                let angle = (90.0 + 144.0 * f32::from(point)).to_radians();
                [0.4 * angle.cos(), 0.4 * angle.sin()]
            })
            .collect();
        let star = |fill: FillMethod| {
            let shape = Polygon {
                fill,
                ..polygon(&points)
            };
            plain(64, 64, &Shape::Polygon(shape))
        };
        let alternate = star(FillMethod::Alternate);
        let winding = star(FillMethod::NonZeroWinding);
        // The middle of the star.
        assert_eq!(at(&alternate, 64, 32, 32), 0.0);
        assert_eq!(at(&winding, 64, 32, 32), 1.0);
        // Its top point, which the line goes around once, and beside it.
        assert_eq!(at(&alternate, 64, 32, 12), 1.0);
        assert_eq!(at(&winding, 64, 32, 12), 1.0);
        assert_eq!(at(&alternate, 64, 20, 12), 0.0);
        assert_eq!(at(&winding, 64, 20, 12), 0.0);
        // Non Zero Winding is Alternate with its holes filled.
        for (hole, filled) in alternate.iter().zip(&winding) {
            assert!(hole <= filled);
        }
        assert!(ones(&winding) > ones(&alternate));
    }

    #[test]
    fn a_polygon_of_fewer_than_three_points_is_nothing() {
        assert_eq!(
            polygon_distance([1.0, 1.0], &[], FillMethod::Alternate),
            FAR
        );
        let line = [[0.0, 0.0], [8.0, 8.0]];
        assert_eq!(
            polygon_distance([4.0, 4.0], &line, FillMethod::Alternate),
            FAR
        );
        // Whatever its border, and inverted it is everything.
        let wide = Common {
            border_width: 1.0,
            soft_edge: 1.0,
            ..Common::default()
        };
        let few: [&[[f32; 2]]; 3] = [&[], &[[0.0, 0.0]], &[[-0.25, 0.0], [0.25, 0.0]]];
        for points in few {
            let shape = Shape::Polygon(polygon(points));
            assert_eq!(shape.check(), Ok(()));
            assert_eq!(plain(8, 8, &shape), vec![0.0; 64]);
            assert_eq!(mask(8, 8, &shape, &wide, None), vec![0.0; 64]);
            let outline = Common {
                solid: false,
                ..wide
            };
            assert_eq!(mask(8, 8, &shape, &outline, None), vec![0.0; 64]);
            let inverted = Common {
                invert: true,
                ..Common::default()
            };
            assert_eq!(mask(8, 8, &shape, &inverted, None), vec![1.0; 64]);
        }
    }

    #[test]
    fn size_scales_a_polygon_about_its_centre() {
        let square = polygon(&[
            [-0.125, -0.125],
            [0.125, -0.125],
            [0.125, 0.125],
            [-0.125, 0.125],
        ]);
        let got = plain(32, 32, &Shape::Polygon(square.clone()));
        assert_eq!(reach(&got, 32), [12, 12, 19, 19]);
        let doubled = Polygon {
            size: 2.0,
            ..square.clone()
        };
        let got = plain(32, 32, &Shape::Polygon(doubled));
        assert_eq!(reach(&got, 32), [8, 8, 23, 23]);
        // Its centre moves it whole: a quarter of the frame to the right and an eighth up.
        let moved = Polygon {
            centre: [0.75, 0.625],
            ..square.clone()
        };
        let got = plain(32, 32, &Shape::Polygon(moved));
        assert_eq!(reach(&got, 32), [20, 8, 27, 15]);
        // A size of 0 is nothing.
        let none = Polygon {
            size: 0.0,
            ..square
        };
        assert_eq!(plain(32, 32, &Shape::Polygon(none)), vec![0.0; 32 * 32]);
    }

    #[test]
    fn soft_edge_is_a_ramp_of_its_width_across_the_edge_and_a_half_on_it() {
        // A rectangle from 16 to 48 across a frame 64 wide, far taller than the frame: the row
        // 4 crosses its left and right edges alone. A Soft Edge of a quarter is 16 pixels.
        let shape = Shape::Rectangle(rectangle(0.5, 10.0));
        let soft = Common {
            soft_edge: 0.25,
            ..Common::default()
        };
        assert_eq!(
            soft.edge(64),
            Edge {
                soft: 8.0,
                border: 0.0,
                solid: true
            }
        );
        let got = mask(64, 8, &shape, &soft, None);
        let row: Vec<f32> = (0..64).map(|x| at(&got, 64, x, 4)).collect();
        // 1 up to 8 pixels inside the edge at 48, and 0 from 8 pixels outside it.
        assert_eq!(row[32..40], [1.0; 8]);
        assert_eq!(row[56..64], [0.0; 8]);
        // Between, the 16 pixels of the ramp, each less than the one before.
        let ramp = &row[40..56];
        assert!(ramp.iter().all(|value| *value > 0.0 && *value < 1.0));
        assert_eq!(
            row.iter()
                .filter(|value| **value > 0.0 && **value < 1.0)
                .count(),
            32
        );
        for pair in row[39..57].windows(2) {
            assert!(pair[1] < pair[0], "{pair:?}");
        }
        // The curve: the centre of the pixel 44 is 3.5 pixels inside the edge.
        assert_eq!(row[44], 1.0 - smoothstep(-8.0, 8.0, -3.5));
        // As much is taken inside the edge as is given outside it, and the other edge is the
        // same the other way round.
        for step in 0..8 {
            assert!(
                (row[47 - step] + row[48 + step] - 1.0).abs() < 1e-6,
                "{step}"
            );
        }
        for x in 0..32 {
            assert_eq!(row[x], row[63 - x], "{x}");
        }
        // Exactly a half on the edge: moved half a pixel, the edge is on the centre of the
        // pixel 48.
        let moved = Rectangle {
            centre: [0.5 + 0.5 / 64.0, 0.5],
            ..rectangle(0.5, 10.0)
        };
        let got = mask(64, 8, &Shape::Rectangle(moved), &soft, None);
        assert_eq!(at(&got, 64, 48, 4), 0.5);
        assert_eq!(feather(0.0, 8.0), 0.5);
        assert_eq!(feather(-8.0, 8.0), 1.0);
        assert_eq!(feather(8.0, 8.0), 0.0);
        assert_eq!(feather(-100.0, 8.0), 1.0);
        assert_eq!(feather(100.0, 8.0), 0.0);
        // With no soft edge, the pixel on the edge is outside.
        assert_eq!(feather(0.0, 0.0), 0.0);
        assert_eq!(feather(-0.001, 0.0), 1.0);
        assert_eq!(feather(0.001, 0.0), 0.0);
    }

    #[test]
    fn border_width_grows_or_shrinks_a_solid_shape_by_half_of_it() {
        // A square from 8 to 24; a Border Width of an eighth of 32 pixels is 2 on each side.
        let bordered = |border_width: f32| {
            let common = Common {
                border_width,
                ..Common::default()
            };
            assert_eq!(common.edge(32).border, border_width * 16.0);
            mask(
                32,
                32,
                &Shape::Rectangle(Rectangle::default()),
                &common,
                None,
            )
        };
        assert_eq!(reach(&bordered(0.0), 32), [8, 8, 23, 23]);
        let grown = bordered(0.125);
        assert_eq!(reach(&grown, 32), [6, 6, 25, 25]);
        assert_eq!(at(&grown, 32, 6, 16), 1.0);
        assert_eq!(at(&grown, 32, 5, 16), 0.0);
        // Grown by distance, its corners are rounded: the corner pixel is 2.1 pixels away.
        assert_eq!(at(&grown, 32, 6, 6), 0.0);
        assert_eq!(at(&grown, 32, 7, 7), 1.0);
        assert_eq!(ones(&grown), 20 * 20 - 4);
        // Shrunk, they stay sharp.
        let shrunk = bordered(-0.125);
        assert_eq!(reach(&shrunk, 32), [10, 10, 21, 21]);
        assert_eq!(ones(&shrunk), 12 * 12);
    }

    #[test]
    fn border_width_without_solid_is_a_ring() {
        let outline = |border_width: f32| Common {
            border_width,
            solid: false,
            ..Common::default()
        };
        // A square from 8 to 24 with 2 pixels on each side of its edge.
        let square = Shape::Rectangle(Rectangle::default());
        let got = mask(32, 32, &square, &outline(0.125), None);
        let row: Vec<f32> = (0..32).map(|x| at(&got, 32, x, 16)).collect();
        assert_eq!(row[..6], [0.0; 6]);
        assert_eq!(row[6..10], [1.0; 4]);
        assert_eq!(row[10..22], [0.0; 12]);
        assert_eq!(row[22..26], [1.0; 4]);
        assert_eq!(row[26..], [0.0; 6]);
        assert_eq!(reach(&got, 32), [6, 6, 25, 25]);
        // The sign of the Border Width is not read.
        assert_eq!(got, mask(32, 32, &square, &outline(-0.125), None));
        // With no Border Width there is nothing.
        assert_eq!(
            mask(32, 32, &square, &outline(0.0), None),
            vec![0.0; 32 * 32]
        );
        // A circle of a radius of 16 about (32, 32), with 4 pixels on each side of its edge.
        let circle = Shape::Ellipse(Ellipse::default());
        let got = mask(64, 64, &circle, &outline(0.125), None);
        for y in 0..64_u32 {
            for x in 0..64_u32 {
                let across = x as f32 + 0.5 - 32.0;
                let down = y as f32 + 0.5 - 32.0;
                let off = ((across * across + down * down).sqrt() - 16.0).abs();
                let want = f32::from(off < 4.0);
                assert_eq!(at(&got, 64, x, y), want, "{x}, {y}");
            }
        }
        // A soft ring is at its most on the edge of the shape.
        let soft = Common {
            soft_edge: 0.125,
            ..outline(0.125)
        };
        let edge = soft.edge(64);
        assert_eq!(coverage(0.0, &edge), 1.0);
        assert_eq!(coverage(4.0, &edge), 0.5);
        assert_eq!(coverage(-4.0, &edge), 0.5);
        assert_eq!(coverage(8.0, &edge), 0.0);
        assert_eq!(coverage(-8.0, &edge), 0.0);
        assert_eq!(drawn(-3.0, &edge), -1.0);
        assert_eq!(drawn(3.0, &edge), -1.0);
    }

    #[test]
    fn invert_is_1_less_the_mask() {
        let inverted = Common {
            invert: true,
            ..Common::default()
        };
        let soft = Common {
            soft_edge: 0.25,
            ..Common::default()
        };
        let soft_inverted = Common {
            invert: true,
            ..soft
        };
        let shapes = [
            Shape::Rectangle(Rectangle::default()),
            Shape::Ellipse(Ellipse::default()),
        ];
        for shape in &shapes {
            let straight = plain(32, 32, shape);
            let got = mask(32, 32, shape, &inverted, None);
            for (value, straight) in got.iter().zip(&straight) {
                assert_eq!(*value, 1.0 - straight);
            }
            assert_eq!(at(&got, 32, 16, 16), 0.0);
            assert_eq!(at(&got, 32, 0, 0), 1.0);
            let straight = mask(32, 32, shape, &soft, None);
            let got = mask(32, 32, shape, &soft_inverted, None);
            for (value, straight) in got.iter().zip(&straight) {
                assert_eq!(*value, 1.0 - straight);
            }
        }
        assert_eq!(value(0.25, 1.0, true), 0.75);
        assert_eq!(value(0.25, 1.0, false), 0.25);
    }

    #[test]
    fn level_scales_the_mask() {
        let shape = Shape::Ellipse(Ellipse::default());
        let soft = Common {
            soft_edge: 0.25,
            ..Common::default()
        };
        let whole = mask(32, 32, &shape, &soft, None);
        let quarter = Common {
            level: 0.25,
            ..soft
        };
        let got = mask(32, 32, &shape, &quarter, None);
        for (value, whole) in got.iter().zip(&whole) {
            assert_eq!(*value, whole * 0.25);
        }
        assert_eq!(at(&got, 32, 16, 16), 0.25);
        // Inverted, the level is of what is then covered.
        let inverted = Common {
            invert: true,
            ..quarter
        };
        let got = mask(32, 32, &shape, &inverted, None);
        for (value, whole) in got.iter().zip(&whole) {
            assert_eq!(*value, (1.0 - whole) * 0.25);
        }
        assert_eq!(at(&got, 32, 0, 0), 0.25);
        assert_eq!(at(&got, 32, 16, 16), 0.0);
        // A level of 0 is no mask, inverted or not.
        for invert in [false, true] {
            let none = Common {
                level: 0.0,
                invert,
                ..soft
            };
            assert_eq!(mask(32, 32, &shape, &none, None), vec![0.0; 32 * 32]);
        }
        assert_eq!(value(0.75, 0.5, false), 0.375);
        assert_eq!(value(0.75, 0.5, true), 0.125);
        assert_eq!(value(f32::NAN, 1.0, false), 0.0);
    }

    #[test]
    fn the_paint_modes_combine_a_mask_with_the_incoming_one() {
        // Merge: the mask over the incoming one.
        assert_eq!(merge(0.5, 0.5), 0.75);
        assert_eq!(merge(0.5, 0.25), 0.625);
        assert_eq!(merge(0.0, 0.3), 0.3);
        assert_eq!(merge(0.3, 0.0), 0.3);
        assert_eq!(merge(1.0, 0.3), 1.0);
        assert_eq!(merge(0.3, 1.0), 1.0);
        // Add, up to 1.
        assert_eq!(add(0.25, 0.5), 0.75);
        assert_eq!(add(0.75, 0.5), 1.0);
        assert_eq!(add(0.0, 0.3), 0.3);
        // Subtract: the mask from the incoming one, down to 0.
        assert_eq!(subtract(0.25, 0.75), 0.5);
        assert_eq!(subtract(0.75, 0.25), 0.0);
        assert_eq!(subtract(0.0, 0.3), 0.3);
        assert_eq!(subtract(1.0, 1.0), 0.0);
        // Each by its mode.
        assert_eq!(painted(PaintMode::Merge, 0.5, Some(0.5)), 0.75);
        assert_eq!(painted(PaintMode::Add, 0.5, Some(0.75)), 1.0);
        assert_eq!(painted(PaintMode::Subtract, 0.5, Some(0.75)), 0.25);
        // What is beyond 0 and 1 is read as the nearer of them, and what is no number as 0.
        assert_eq!(merge(0.5, 2.0), 1.0);
        assert_eq!(add(0.5, -1.0), 0.5);
        assert_eq!(subtract(0.25, 3.0), 0.75);
        assert_eq!(add(0.5, f32::NAN), 0.5);
        assert_eq!(merge(f32::NAN, 0.5), 0.5);
        assert_eq!(subtract(f32::NAN, 0.5), 0.5);
    }

    #[test]
    fn with_no_incoming_mask_every_paint_mode_gives_the_mask_itself() {
        for mode in [PaintMode::Merge, PaintMode::Add, PaintMode::Subtract] {
            for own in [0.0, 0.25, 1.0] {
                assert_eq!(painted(mode, own, None), own, "{mode:?}");
            }
            assert_eq!(painted(mode, 2.0, None), 1.0, "{mode:?}");
            assert_eq!(painted(mode, f32::NAN, None), 0.0, "{mode:?}");
            let common = Common {
                paint_mode: mode,
                ..Common::default()
            };
            let shape = Shape::Ellipse(Ellipse::default());
            assert_eq!(mask(16, 16, &shape, &common, None), plain(16, 16, &shape));
        }
    }

    #[test]
    fn a_frame_is_combined_with_the_incoming_mask_pixel_by_pixel() {
        // The left half of a frame of four pixels by two, at a level of a half.
        let left = Rectangle {
            centre: [0.25, 0.5],
            ..rectangle(0.5, 1.0)
        };
        let shape = Shape::Rectangle(left);
        let painted_by = |paint_mode: PaintMode, incoming: &[f32]| {
            let common = Common {
                level: 0.5,
                paint_mode,
                ..Common::default()
            };
            mask(4, 2, &shape, &common, Some(incoming))
        };
        let incoming = [0.25, 1.0, 0.25, 1.0, 0.0, 0.5, 0.0, 0.5];
        assert_eq!(
            painted_by(PaintMode::Merge, &incoming),
            [0.625, 1.0, 0.25, 1.0, 0.5, 0.75, 0.0, 0.5]
        );
        assert_eq!(
            painted_by(PaintMode::Add, &incoming),
            [0.75, 1.0, 0.25, 1.0, 0.5, 1.0, 0.0, 0.5]
        );
        assert_eq!(
            painted_by(PaintMode::Subtract, &incoming),
            [0.0, 0.5, 0.25, 1.0, 0.0, 0.0, 0.0, 0.5]
        );
        // An incoming mask of nothing leaves the mask itself under Merge and Add, and nothing
        // under Subtract.
        let nothing = [0.0; 8];
        let own = [0.5, 0.5, 0.0, 0.0, 0.5, 0.5, 0.0, 0.0];
        assert_eq!(painted_by(PaintMode::Merge, &nothing), own);
        assert_eq!(painted_by(PaintMode::Add, &nothing), own);
        assert_eq!(painted_by(PaintMode::Subtract, &nothing), nothing);
        // Incoming values that are no values of a mask.
        let odd = [f32::NAN, 2.0, -1.0, f32::INFINITY, 0.0, 0.0, 0.0, 0.0];
        assert_eq!(
            painted_by(PaintMode::Merge, &odd),
            [0.5, 1.0, 0.0, 1.0, 0.5, 0.5, 0.0, 0.0]
        );
    }

    #[test]
    #[should_panic(expected = "an incoming mask of the size of the frame")]
    fn an_incoming_mask_of_another_size_is_refused() {
        let shape = Shape::Ellipse(Ellipse::default());
        let incoming = [0.0; 7];
        let _ = mask(4, 2, &shape, &Common::default(), Some(incoming.as_slice()));
    }

    #[test]
    fn check_refuses_each_common_setting_that_is_out_of_its_range() {
        let refused = |common: Common, what: &'static str| {
            assert_eq!(common.check(), Err(OutOfRange { what }), "{common:?}");
        };
        for level in [-0.01, 1.01, f32::NAN, f32::INFINITY] {
            let common = Common {
                level,
                ..Common::default()
            };
            refused(common, "mask.level");
        }
        for soft_edge in [-0.01, 1.01, f32::NAN, f32::INFINITY] {
            let common = Common {
                soft_edge,
                ..Common::default()
            };
            refused(common, "mask.soft_edge");
        }
        for border_width in [-1.01, 1.01, f32::NAN, f32::NEG_INFINITY] {
            let common = Common {
                border_width,
                ..Common::default()
            };
            refused(common, "mask.border_width");
        }
        // The first that is out of its range is the one named.
        let both = Common {
            soft_edge: f32::NAN,
            border_width: f32::NAN,
            ..Common::default()
        };
        refused(both, "mask.soft_edge");
        // The ends of each range are within it.
        let least = Common {
            level: LEVEL.low,
            soft_edge: SOFT_EDGE.low,
            border_width: BORDER_WIDTH.low,
            ..Common::default()
        };
        assert_eq!(least.check(), Ok(()));
        let most = Common {
            level: LEVEL.high,
            soft_edge: SOFT_EDGE.high,
            border_width: BORDER_WIDTH.high,
            ..Common::default()
        };
        assert_eq!(most.check(), Ok(()));
        let error = OutOfRange { what: "mask.level" };
        assert_eq!(
            error.to_string(),
            "the setting mask.level is not a number within its range"
        );
    }

    #[test]
    fn check_refuses_each_setting_of_a_shape_that_is_out_of_its_range() {
        let refused = |shape: Shape, what: &'static str| {
            assert_eq!(shape.check(), Err(OutOfRange { what }), "{shape:?}");
        };
        let rectangle = Rectangle::default();
        let ellipse = Ellipse::default();
        let polygon = polygon(&[[-0.25, -0.25], [0.25, -0.25], [0.0, 0.25]]);
        for centre in [-10.5, 10.5, f32::NAN, f32::INFINITY] {
            let across = [centre, 0.5];
            let up = [0.5, centre];
            for (centre, what) in [(across, "mask.centre.x"), (up, "mask.centre.y")] {
                refused(
                    Shape::Rectangle(Rectangle {
                        centre,
                        ..rectangle
                    }),
                    what,
                );
                refused(Shape::Ellipse(Ellipse { centre, ..ellipse }), what);
                let moved = Polygon {
                    centre,
                    ..polygon.clone()
                };
                refused(Shape::Polygon(moved), what);
            }
        }
        for extent in [-0.01, 10.5, f32::NAN, f32::INFINITY] {
            let wide = Rectangle {
                width: extent,
                ..rectangle
            };
            refused(Shape::Rectangle(wide), "mask.width");
            let high = Rectangle {
                height: extent,
                ..rectangle
            };
            refused(Shape::Rectangle(high), "mask.height");
            let wide = Ellipse {
                width: extent,
                ..ellipse
            };
            refused(Shape::Ellipse(wide), "mask.width");
            let high = Ellipse {
                height: extent,
                ..ellipse
            };
            refused(Shape::Ellipse(high), "mask.height");
        }
        for corner_radius in [-0.01, 1.01, f32::NAN, f32::INFINITY] {
            let rounded = Rectangle {
                corner_radius,
                ..rectangle
            };
            refused(Shape::Rectangle(rounded), "mask.corner_radius");
        }
        for angle in [-36_001.0, 36_001.0, f32::NAN, f32::NEG_INFINITY] {
            refused(
                Shape::Rectangle(Rectangle { angle, ..rectangle }),
                "mask.angle",
            );
            refused(Shape::Ellipse(Ellipse { angle, ..ellipse }), "mask.angle");
            let turned = Polygon {
                angle,
                ..polygon.clone()
            };
            refused(Shape::Polygon(turned), "mask.angle");
        }
        for size in [-0.01, 10.5, f32::NAN, f32::INFINITY] {
            let sized = Polygon {
                size,
                ..polygon.clone()
            };
            refused(Shape::Polygon(sized), "mask.size");
        }
        for coordinate in [-16.5, 16.5, f32::NAN, f32::INFINITY] {
            for point in [[coordinate, 0.0], [0.0, coordinate]] {
                let mut bent = polygon.clone();
                bent.points[1] = point;
                refused(Shape::Polygon(bent), "mask.points");
            }
        }
        // The first that is out of its range is the one named.
        let both = Rectangle {
            width: f32::NAN,
            angle: f32::NAN,
            ..rectangle
        };
        refused(Shape::Rectangle(both), "mask.width");
    }

    #[test]
    fn the_ends_of_each_range_of_a_shape_are_within_it() {
        let least = Rectangle {
            centre: [CENTRE.low, CENTRE.high],
            width: EXTENT.low,
            height: EXTENT.high,
            corner_radius: CORNER_RADIUS.low,
            angle: ANGLE.low,
        };
        assert_eq!(Shape::Rectangle(least).check(), Ok(()));
        let most = Ellipse {
            centre: [CENTRE.high, CENTRE.low],
            width: EXTENT.high,
            height: EXTENT.low,
            angle: ANGLE.high,
        };
        assert_eq!(Shape::Ellipse(most).check(), Ok(()));
        let far = Polygon {
            centre: [CENTRE.low, CENTRE.low],
            size: SIZE.high,
            angle: ANGLE.low,
            points: vec![[POINT.low, POINT.high], [POINT.high, POINT.low]],
            fill: FillMethod::NonZeroWinding,
        };
        assert_eq!(Shape::Polygon(far).check(), Ok(()));
        let none = Polygon {
            size: SIZE.low,
            ..Polygon::default()
        };
        assert_eq!(Shape::Polygon(none).check(), Ok(()));
    }

    #[test]
    fn settings_out_of_their_ranges_are_taken_as_the_nearest_within_them() {
        let placed = |shape: Rectangle| Shape::Rectangle(shape).placed(64, 32);
        let odd = Rectangle {
            centre: [f32::NAN, f32::INFINITY],
            width: f32::NAN,
            height: -3.0,
            corner_radius: 7.0,
            angle: f32::NAN,
        };
        let held = Rectangle {
            centre: [CENTRE.default, CENTRE.high],
            width: EXTENT.default,
            height: EXTENT.low,
            corner_radius: CORNER_RADIUS.high,
            angle: ANGLE.default,
        };
        assert_eq!(placed(odd), placed(held));
        let edge = |common: Common| common.edge(64);
        let odd = Common {
            soft_edge: f32::INFINITY,
            border_width: f32::NAN,
            ..Common::default()
        };
        let held = Common {
            soft_edge: SOFT_EDGE.high,
            border_width: BORDER_WIDTH.default,
            ..Common::default()
        };
        assert_eq!(edge(odd), edge(held));
        // A level beyond 1 is 1, below 0 is 0, and no number is the level at first.
        assert_eq!(value(0.5, 2.0, false), 0.5);
        assert_eq!(value(0.5, -1.0, false), 0.0);
        assert_eq!(value(0.5, f32::NAN, false), 0.5);
        // A point of a polygon that is no number is at its centre.
        let bent = polygon(&[[f32::NAN, f32::NAN], [0.25, 0.0], [0.0, 0.25]]);
        let straight = polygon(&[[0.0, 0.0], [0.25, 0.0], [0.0, 0.25]]);
        assert_eq!(
            Shape::Polygon(bent).placed(64, 32),
            Shape::Polygon(straight).placed(64, 32)
        );
        // A frame of no pixel has no mask, and a shape of no width covers nothing.
        let shape = Shape::Ellipse(Ellipse::default());
        assert_eq!(plain(0, 5, &shape), Vec::<f32>::new());
        assert_eq!(plain(5, 0, &shape), Vec::<f32>::new());
        for (width, height) in [(0.0, 0.5), (0.5, 0.0), (0.0, 0.0)] {
            let flat = Ellipse {
                width,
                height,
                ..Ellipse::default()
            };
            assert_eq!(plain(9, 9, &Shape::Ellipse(flat)), vec![0.0; 81]);
            assert_eq!(
                plain(9, 9, &Shape::Rectangle(rectangle(width, height))),
                vec![0.0; 81]
            );
        }
    }
}
