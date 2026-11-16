//! The composition of a clip on the card: the graph of nodes that makes its picture
//! ([`columbina_model::fusion`]), drawn into textures of its own before the layer of the clip is
//! laid on the canvas.
//!
//! # Where it is in a frame
//!
//! A layer of a clip that has a composition shows what comes out of it ([`Fusion`],
//! [`crate::Source::Fused`]). The source of the clip is what a `MediaIn` shows: it is drawn
//! alone, decoded and through the input transform of the layer, into a texture; the nodes are
//! drawn from it, each into a texture; and the texture of the node that the `MediaOut` reads is
//! the picture of the layer, which then goes through the grade of the clip, its crop, its
//! transform, its composite mode and its opacity as the picture of a title does, in the pass
//! that lays it on the canvas, unchanged. A layer without a composition is drawn as it always
//! was, and nothing of this is made for it.
//!
//! # The pictures
//!
//! Every picture of a composition is of one size, that of the composition (the source of its
//! clip, [`columbina_model::fusion::size_of`]): a Merge gives a picture of the size of its
//! background, a Transform one of the size that came in, and an input that is not linked is
//! a transparent picture of that size. They hold R, G, B and A premultiplied by the alpha, in
//! the working space of the timeline, in which the canvas lays its layers too: as they are
//! coded there, with their gamma where the project is not managed. They are of half floats
//! (the format of the canvas and of the textures of neighbourhoods, whose spares they
//! share), so that what is above 1 or below 0 is kept, as `merge.rs` asks.
//!
//! The settings of the nodes are shares of the picture, so that it may be drawn at any size
//! (`drawn_size`): at that of the composition where the layer shows it as large or larger,
//! and smaller where the layer shows it smaller (a preview at half the size of the timeline, a
//! source of 4K in a timeline of 1080 lines), so that the passes cost what is seen.
//!
//! # The passes
//!
//! `plan` makes the passes of a composition at one moment:
//!
//! - a `MediaIn` is the picture of the source, drawn once however many there are;
//! - a Background is a texture cleared to its colour (`Pass::Fill`), where it has no mask;
//! - a Merge and a Transform are one pass each of `fusion.wgsl` (`Pass::Lay`): the foreground
//!   moved, made larger or smaller and turned (and, of a Transform, flipped), read between its
//!   texels and smoothed at its edges as a layer is, then merged with the background by the
//!   mathematics of [`crate::merge`], step by step. A Transform is its picture laid over
//!   nothing by a merge that changes nothing;
//! - a Rectangle, an Ellipse and a Polygon are one pass each of `fusion_mask.wgsl`
//!   (`Pass::Mask`), always: see Masks below;
//! - a node that changes nothing is no pass: its picture is that of its input. So is a
//!   Transform as it is at first, a Merge whose foreground gives no picture or whose Blend is
//!   0 (its background), and a Merge as it is at first over no background (its foreground):
//!   each where it has no mask, and Masks below tells of those that have one. A node whose
//!   kind is not known here hands on the picture of its first input.
//!
//! A composition whose `MediaOut` shows its `MediaIn`, as the one that a clip has at first
//! does, has no pass at all: its layer is drawn as that of a clip without a composition, to
//! the bit. One of `n` nodes costs `n` passes at the most, and one more for its source.
//!
//! The textures are those of the neighbourhoods of grades ([`crate::neighbourhood`]), kept from
//! frame to frame by their size: a pass draws into one that no pass after it reads
//! (`Plan::slots`), so that a chain of nodes takes two or three, however long it is. The
//! picture of a mask is one of them, free for the passes after the last that reads it.
//!
//! # Masks
//!
//! What a mask is and what it does is told with the model ([`columbina_model::fusion`],
//! Masks), and its mathematics is [`crate::mask`], which the shader is held to.
//!
//! A mask node is a pass that reads no picture but the mask that comes into it: each pixel
//! is white premultiplied by the value of the mask there, so that its R, G, B and A are all
//! that value. The value is that of [`crate::mask::mask`] for a frame of the size that the
//! pictures are drawn at: the settings are made pixels of that size by
//! [`crate::mask::Shape::placed`] and [`crate::mask::Common::edge`], here and not on the card,
//! so that a mask is the same share of the picture at any size. The sides of a Polygon go to
//! the card in a buffer of their own, those of all the polygons of a frame one after another,
//! as the outlines of the windows of the Color page go in the parameters of their grade: a
//! polygon may have any number of points. Read as a picture, by a Merge, a Transform or the
//! `MediaOut`, a mask node is that white picture.
//!
//! A `mask` input reads the alpha of the picture that is linked to it, at the pixel, within 0
//! and 1: an alpha below 0, or one that is no number, is 0, and one above 1 is 1, as
//! [`crate::mask`] reads an incoming mask. What a mask does to the node that it comes into:
//!
//! - a Merge and a Transform are limited in the pass that lays them, which reads the mask
//!   too: `first + (own - first) * mask`, channel by channel, `own` what the node draws
//!   without a mask and `first` the picture of its first input as it came in (of a Merge its
//!   background, of a Transform its picture where it lay; nothing where that is not linked).
//!   Where the mask is 1 the pixel is `own` to the bit, and where it is 0 it is `first`;
//! - a Background that has a mask is a pass of `fusion_mask.wgsl` in place of the clear: its
//!   colour, premultiplied, times the mask, all four channels;
//! - a mask node combines the mask that comes in with its own by its Paint Mode.
//!
//! A node without a mask is drawn by the passes that it was drawn by before there were masks,
//! and its picture is the same to the bit. A node with one is no pass where the mask cannot
//! change it, `own` and `first` being one picture:
//!
//! - a Transform as it is at first is its input, whatever its mask;
//! - a Merge whose foreground gives no picture, of which nothing is left or whose Blend is 0
//!   is its background, whatever its mask;
//! - a Transform of no picture is no picture; and so is a Background of which nothing is
//!   seen.
//!
//! But a node with a mask is a pass where it would have been none without, `own` and `first`
//! being two pictures:
//!
//! - a Merge as it is at first over no background is its foreground times its mask, not its
//!   foreground;
//! - a Transform that leaves nothing of its picture (a Size of 0) is that picture as it came
//!   in times what the mask leaves, `1 - mask`, not nothing.
//!
//! A `mask` that names a node which gives no picture here is no mask, and the node is not
//! limited, as the model has a link that brings no picture: so a `MediaIn` of a clip whose
//! source shows nothing, and so too a picture that is known to be nothing without a pass (a
//! Background whose alpha is 0, a Transform of nothing). A mask node is never such a
//! picture: a mask of 0 everywhere is a picture, and limits what reads it to nothing.
//!
//! A mask node into which no picture comes has no Paint Mode: the mask is its own.

use columbina_model::fusion::{self as model, Fused, FusedKind, Merged, Transformed};

use crate::geometry::{fit, raster_scale, samples, Affine};
use crate::gpu::Gpu;
use crate::grade::layout;
use crate::mask::{self, Common, FillMethod, PaintMode, Placed, Shape};
use crate::merge::{Operator, Settings};
use crate::Source;

/// The shader of a pass that lays a foreground over a background.
const SHADER: &str = include_str!("fusion.wgsl");

/// The shader of a pass that draws a mask, or a Background times its effect mask.
const MASK_SHADER: &str = include_str!("fusion_mask.wgsl");

/// The format of the pictures of a composition: that of the canvas, half floats, which keep
/// what is above 1 and below 0.
pub(crate) const FORMAT: wgpu::TextureFormat = crate::WORKING_FORMAT;

/// How many values the uniform of `fusion.wgsl` has, and that of `fusion_mask.wgsl`, which is
/// of the same size, so that the uniforms of all the passes of a frame lie in one buffer, a
/// slot each.
pub(crate) const PARAMS: usize = 20;

/// The bytes of the uniform of `fusion.wgsl`, and of that of `fusion_mask.wgsl`.
const PARAMS_SIZE: u64 = 80;

/// The place in the uniform of `fusion.wgsl` of how the pass is limited by an effect mask
/// (`extra.w`): 0 where the node has none.
const LIMITED_AT: usize = 11;

/// What the uniform of `fusion.wgsl` has at [`LIMITED_AT`] for a node that has an effect mask
/// and whose first input is the background of the pass: a Merge.
const LIMITED_TO_BACKGROUND: f32 = 1.0;

/// What the uniform of `fusion.wgsl` has at [`LIMITED_AT`] for a node that has an effect mask
/// and whose first input is the foreground of the pass as it came in: a Transform.
const LIMITED_TO_FOREGROUND: f32 = 2.0;

/// The place in the uniform of `fusion_mask.wgsl` of the first of the sides of a Polygon among
/// those of the frame (`kind.z`): whoever puts the sides of the passes of a frame one after
/// another writes it ([`Pass::Mask`]).
pub(crate) const SIDES_AT: usize = 6;

/// What the uniform of `fusion_mask.wgsl` has as what is drawn (`kind.x`) for a Background
/// that has an effect mask; a Rectangle is 0, an Ellipse 1 and a Polygon 2.
const TINTED: f32 = 3.0;

/// What a layer of a clip that has a composition shows ([`crate::Source::Fused`]): the picture
/// that comes out of the composition, in place of the source of the clip.
#[derive(Clone, Debug)]
pub struct Fusion {
    /// The source of the clip, which a `MediaIn` of the composition shows: a picture, two
    /// mixed, a title or a colour, as the layer of a clip without a composition has it.
    pub source: Source,
    /// The composition at the frame, as `resolve` gives it with the layer.
    pub composition: Fused,
    /// The size of the pictures of the composition, in pixels
    /// ([`columbina_model::fusion::size_of`]): that of the source of the clip. The settings of
    /// the nodes are shares of it, and the picture that comes out is fitted to the canvas by
    /// its shape, as a picture of that size is.
    pub size: (u32, u32),
}

impl Fusion {
    /// The ids of the nodes of the composition whose kinds are not drawn here: each hands on
    /// the picture of its first input, or none. For whoever makes the scene to tell of them.
    #[must_use]
    pub fn unknown(&self) -> Vec<&str> {
        self.composition
            .nodes
            .iter()
            .filter(|node| !is_drawn(&node.kind))
            .map(|node| node.id.as_str())
            .collect()
    }
}

/// Whether a node of the kind is drawn as what it is.
fn is_drawn(kind: &FusedKind) -> bool {
    matches!(
        kind,
        FusedKind::MediaIn
            | FusedKind::Background { .. }
            | FusedKind::Merge { .. }
            | FusedKind::Transform { .. }
            | FusedKind::Rectangle { .. }
            | FusedKind::Ellipse { .. }
            | FusedKind::Polygon { .. }
    )
}

/// The size that the pictures of a composition of `nominal` pixels are drawn at, for a layer
/// that is fitted to a canvas of `canvas` pixels and made `scale` times larger there; none
/// where either has no pixels.
///
/// It is the size of the composition where the layer shows it as large or larger. Where the
/// layer shows it smaller, it is the size that it is fitted to the canvas at, so that a layer
/// that is not made larger or smaller shows each of its pixels as it was drawn; times the
/// scale of the layer in steps of a quarter of an octave above 1, and below a half as a title
/// is drawn smaller ([`raster_scale`]), so that a scale that moves has the pictures made at
/// another size only now and then. No side is more than `most`, what the card takes.
#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "sides of pictures, from 1 to what the card takes"
)]
pub(crate) fn drawn_size(
    nominal: (u32, u32),
    canvas: (f64, f64),
    scale: f64,
    most: u32,
) -> Option<(u32, u32)> {
    let own = (f64::from(nominal.0), f64::from(nominal.1));
    let frame = fit(canvas, own);
    if frame.0 <= 0.0 || frame.1 <= 0.0 {
        return None;
    }
    let steps = if scale > 1.0 {
        2.0_f64.powf((4.0 * scale.log2()).ceil() / 4.0)
    } else {
        raster_scale(scale)
    };
    // The pixels of the canvas to a pixel of the composition as it is fitted: the same across
    // and down, but for their rounding.
    let fitted = (frame.0 / own.0).max(frame.1 / own.1);
    let per = (fitted * steps)
        .min(1.0)
        .min(f64::from(most) / own.0.max(own.1));
    let side = |pixels: f64| (pixels * per).round().clamp(1.0, f64::from(most.max(1))) as u32;
    Some((side(own.0), side(own.1)))
}

/// The sine and the cosine of an angle in degrees: those of a whole number of quarter turns
/// are 0, 1 and -1 themselves, so that a picture turned a quarter is read at the middles of its
/// texels.
#[allow(
    clippy::cast_possible_truncation,
    reason = "a whole number of quarter turns, of at most ten turns"
)]
fn turn(angle: f64) -> (f64, f64) {
    let quarters = angle / 90.0;
    if quarters.is_finite() && quarters.fract() == 0.0 {
        return match (quarters as i64).rem_euclid(4) {
            1 => (1.0, 0.0),
            2 => (0.0, -1.0),
            3 => (-1.0, 0.0),
            _ => (0.0, 1.0),
        };
    }
    angle.to_radians().sin_cos()
}

/// How a pixel of the picture that comes out of a node reads the picture that the node moves,
/// both of `size` pixels: where the point (x, y) of the first, in pixels from its top left
/// corner, lies in the second.
///
/// The settings are Fusion's ([`columbina_model::fusion::Center`]): the middle of the picture
/// is put at `center`, shares of the width and of the height from the lower left corner, `y`
/// up; it is made `scale` times as large about its middle; and it is turned `angle` degrees
/// counter-clockwise there. A picture that is flipped (`flips`: left for right, top for
/// bottom) is flipped in its own frame, before it is made larger and turned. The map is the
/// other way round, from where a pixel is drawn to what it reads: `scale` is to be above 0.
pub(crate) fn reading(
    size: (u32, u32),
    center: [f64; 2],
    scale: f64,
    angle: f64,
    flips: (bool, bool),
) -> Affine {
    let (width, height) = (f64::from(size.0), f64::from(size.1));
    let (sin, cos) = turn(angle);
    let across = if flips.0 { -1.0 } else { 1.0 };
    let down = if flips.1 { -1.0 } else { 1.0 };
    // With y down, a point (x, y) from the middle of the picture is put, turned
    // counter-clockwise, at (x cos + y sin, -x sin + y cos) from where the middle is put; so a
    // point (x, y) from there reads (x cos - y sin, x sin + y cos) from the middle.
    let (a, b) = (across * cos / scale, -across * sin / scale);
    let (c, d) = (down * sin / scale, down * cos / scale);
    // Where the middle is put: `center.y` counts from the bottom.
    let middle = (center[0] * width, (1.0 - center[1]) * height);
    Affine {
        a,
        b,
        c,
        d,
        tx: width / 2.0 - (a * middle.0 + b * middle.1),
        ty: height / 2.0 - (c * middle.0 + d * middle.1),
    }
}

/// Whether a picture put at `center`, made `scale` times as large and turned `angle` degrees
/// lies where it lay: in the middle, as large as it is, and not turned, or turned whole turns.
#[allow(
    clippy::float_cmp,
    reason = "the settings as they are at first, which are these numbers themselves"
)]
fn lies_as_it_is(center: [f64; 2], scale: f64, angle: f64) -> bool {
    center == [0.5, 0.5] && scale == 1.0 && angle % 360.0 == 0.0
}

/// Where a pass reads a picture from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Reads {
    /// No picture: a transparent one.
    #[default]
    Nothing,
    /// The picture of the source of the clip, which the compositor draws alone.
    Source,
    /// The picture that the pass of this number makes.
    Pass(usize),
}

/// A pass of a composition: what one texture is drawn with.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Pass {
    /// A colour over the whole picture, premultiplied: a Background that has no mask.
    Fill([f32; 4]),
    /// A foreground moved and merged with a background (`fusion.wgsl`): a Merge, or a
    /// Transform, whose background is nothing. The foreground is never nothing.
    Lay {
        background: Reads,
        foreground: Reads,
        /// The picture whose alpha is the effect mask of the node; nothing where it has none,
        /// and the pass is not limited.
        mask: Reads,
        /// The uniform of `fusion.wgsl`.
        uniform: [f32; PARAMS],
    },
    /// A mask node, white premultiplied by its mask; or a Background that has an effect mask,
    /// its colour times that mask (`fusion_mask.wgsl`).
    Mask {
        /// The picture whose alpha is the mask that comes in; nothing where none comes in. Of
        /// a Background it is never nothing.
        incoming: Reads,
        /// The uniform of `fusion_mask.wgsl`, but for the first of the sides of a Polygon
        /// among those of the frame ([`SIDES_AT`]), which whoever draws the frame knows.
        uniform: [f32; PARAMS],
        /// The sides of a Polygon of three points or more, each from where it starts (x, y)
        /// to where it ends (x, y), in the pixels of the picture; none of any other pass.
        sides: Vec<[f32; 4]>,
    },
}

impl Pass {
    /// The pictures that the pass reads: nothing in the places of those that it has not.
    fn reads(&self) -> [Reads; 3] {
        match self {
            Pass::Fill(_) => [Reads::Nothing; 3],
            Pass::Lay {
                background,
                foreground,
                mask,
                ..
            } => [*background, *foreground, *mask],
            Pass::Mask { incoming, .. } => [*incoming, Reads::Nothing, Reads::Nothing],
        }
    }

    /// The pass, each picture that it reads made what `renumbered` makes of it.
    fn with_reads(self, renumbered: impl Fn(Reads) -> Reads) -> Pass {
        match self {
            fill @ Pass::Fill(_) => fill,
            Pass::Lay {
                background,
                foreground,
                mask,
                uniform,
            } => Pass::Lay {
                background: renumbered(background),
                foreground: renumbered(foreground),
                mask: renumbered(mask),
                uniform,
            },
            Pass::Mask {
                incoming,
                uniform,
                sides,
            } => Pass::Mask {
                incoming: renumbered(incoming),
                uniform,
                sides,
            },
        }
    }
}

/// The passes of a composition at one moment, each after those that it reads, and the picture
/// that goes back to the timeline.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Plan {
    pub passes: Vec<Pass>,
    pub output: Reads,
}

/// The textures that the passes of a [`Plan`] draw into, numbered from 0: that of the source
/// (none where no pass reads it), that of each pass, and how many there are.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Slots {
    pub source: Option<usize>,
    pub passes: Vec<usize>,
    pub count: usize,
}

impl Plan {
    /// Whether the picture of the source of the clip is read: by a pass, or as the picture
    /// that comes out.
    pub(crate) fn reads_source(&self) -> bool {
        self.output == Reads::Source
            || self
                .passes
                .iter()
                .any(|pass| pass.reads().contains(&Reads::Source))
    }

    /// Whether a pass of it is one of `fusion.wgsl`, which needs its pipeline.
    pub(crate) fn lays(&self) -> bool {
        self.passes
            .iter()
            .any(|pass| matches!(pass, Pass::Lay { .. }))
    }

    /// Whether a pass of it is one of `fusion_mask.wgsl`, which needs its pipeline.
    pub(crate) fn masks(&self) -> bool {
        self.passes
            .iter()
            .any(|pass| matches!(pass, Pass::Mask { .. }))
    }

    /// The textures of its passes: a pass draws into one that no pass from it on reads, and
    /// not into one that it reads itself, so that as few are taken as the graph needs. The one
    /// of the picture that comes out is drawn into by no pass after.
    pub(crate) fn slots(&self) -> Slots {
        // The last pass that reads the picture of each pass and that of the source: a number
        // beyond every pass for the picture that comes out, which the layer reads after them.
        let after = self.passes.len();
        let mut last = vec![0_usize; self.passes.len()];
        let mut last_of_source = None;
        let mut read = |reads: Reads, by: usize| match reads {
            Reads::Nothing => {}
            Reads::Source => last_of_source = Some(by),
            Reads::Pass(at) => {
                if let Some(last) = last.get_mut(at) {
                    *last = by;
                }
            }
        };
        for (by, pass) in self.passes.iter().enumerate() {
            for reads in pass.reads() {
                read(reads, by);
            }
        }
        read(self.output, after);
        let mut slots = Slots::default();
        let mut free: Vec<usize> = Vec::new();
        let take = |free: &mut Vec<usize>, count: &mut usize| {
            free.pop().unwrap_or_else(|| {
                *count += 1;
                *count - 1
            })
        };
        slots.source = last_of_source.map(|_| take(&mut free, &mut slots.count));
        for (by, pass) in self.passes.iter().enumerate() {
            let slot = take(&mut free, &mut slots.count);
            slots.passes.push(slot);
            // What this pass is the last to read is free for the passes after it: each
            // picture once, though the pass read it in two places.
            let all = pass.reads();
            for (place, reads) in all.into_iter().enumerate() {
                if all[..place].contains(&reads) {
                    continue;
                }
                let freed = match reads {
                    Reads::Nothing => None,
                    Reads::Source => slots.source.filter(|_| last_of_source == Some(by)),
                    Reads::Pass(at) => slots
                        .passes
                        .get(at)
                        .copied()
                        .filter(|_| last.get(at) == Some(&by)),
                };
                free.extend(freed);
            }
        }
        slots
    }
}

/// The passes of `composition`, its pictures drawn `size` pixels large, `luma` the weights of
/// R, G and B in the luma of the working space, which some Apply Modes read. `source` says
/// whether the source of the clip gives a picture: a title that is not seen gives none, and a
/// `MediaIn` is then no picture.
pub(crate) fn plan(composition: &Fused, size: (u32, u32), luma: [f32; 3], source: bool) -> Plan {
    let mut passes: Vec<Pass> = Vec::new();
    let mut made: Vec<Reads> = Vec::with_capacity(composition.nodes.len());
    for node in &composition.nodes {
        // A node reads the nodes before it alone: one that names another reads nothing.
        let read = |input: Option<usize>| {
            input
                .and_then(|at| made.get(at).copied())
                .unwrap_or_default()
        };
        // A `mask` that names a node which gives no picture is no mask: `read` gives nothing
        // for it, as for one that is not linked.
        let reads = match &node.kind {
            FusedKind::MediaIn if source => Reads::Source,
            // A `MediaIn` of a source that gives no picture.
            FusedKind::MediaIn => Reads::Nothing,
            FusedKind::Background { rgba, mask } => match (filled(*rgba), read(*mask)) {
                (None, _) => Reads::Nothing,
                (Some(colour), Reads::Nothing) => drawn(&mut passes, Pass::Fill(colour)),
                (Some(colour), incoming) => drawn(
                    &mut passes,
                    Pass::Mask {
                        incoming,
                        uniform: tinted(colour),
                        sides: Vec::new(),
                    },
                ),
            },
            FusedKind::Transform {
                input,
                mask,
                settings,
            } => transformed(read(*input), read(*mask), settings, size, luma, &mut passes),
            FusedKind::Merge {
                background,
                foreground,
                mask,
                settings,
            } => merged(
                read(*background),
                read(*foreground),
                read(*mask),
                settings,
                size,
                luma,
                &mut passes,
            ),
            other => match mask::of_node(other) {
                // A mask node: a pass whatever its settings are, as a mask of nothing is a
                // picture too, which limits what reads it to nothing.
                Some((shape, common)) => {
                    let incoming = read(other.mask());
                    drawn(&mut passes, masked(&shape, &common, incoming, size))
                }
                // A kind that is not known here: the picture of its first input, as a node
                // that is passed through hands it on.
                None => read(other.first_input()),
            },
        };
        made.push(reads);
    }
    let output = composition
        .output
        .and_then(|at| made.get(at).copied())
        .unwrap_or_default();
    needed(passes, output)
}

/// Put `pass` among `passes`, and give the picture that it makes.
fn drawn(passes: &mut Vec<Pass>, pass: Pass) -> Reads {
    passes.push(pass);
    Reads::Pass(passes.len() - 1)
}

/// The uniform of `fusion_mask.wgsl` for a Background of the colour `colour`, premultiplied,
/// that has an effect mask: its colour, and that it reads the mask that comes in.
fn tinted(colour: [f32; 4]) -> [f32; PARAMS] {
    let mut uniform = [0.0; PARAMS];
    uniform[..4].copy_from_slice(&colour);
    uniform[4] = TINTED;
    uniform[5] = 1.0;
    uniform
}

/// The pass of a mask node of the shape `shape` and the controls `common`, into which the
/// mask `incoming` comes (nothing: none), for a picture drawn `size` pixels large.
///
/// The settings are made pixels of that size here, by [`Shape::placed`] and [`Common::edge`],
/// as [`mask::mask`] makes them for a frame of that size: the shader is given what the maths
/// of a pixel reads, and makes nothing pixels itself. A Polygon of fewer than three points has
/// no sides, and is a mask of nothing.
#[allow(
    clippy::cast_precision_loss,
    reason = "how many sides a polygon has, far below what a float counts"
)]
fn masked(shape: &Shape, common: &Common, incoming: Reads, size: (u32, u32)) -> Pass {
    let mut sides = Vec::new();
    let (what, place, extent) = match shape.placed(size.0, size.1) {
        Placed::Rectangle {
            centre,
            turn,
            half,
            radius,
        } => (
            0.0,
            [centre[0], centre[1], turn[0], turn[1]],
            [half[0], half[1], radius, 0.0],
        ),
        Placed::Ellipse { centre, turn, half } => (
            1.0,
            [centre[0], centre[1], turn[0], turn[1]],
            [half[0], half[1], 0.0, 0.0],
        ),
        Placed::Polygon { points, fill } => {
            // Closed from the last point to the first, as the maths reads its sides.
            if points.len() >= 3 {
                let mut start = points[points.len() - 1];
                for &end in &points {
                    sides.push([start[0], start[1], end[0], end[1]]);
                    start = end;
                }
            }
            let winds = f32::from(fill == FillMethod::NonZeroWinding);
            (2.0, [0.0; 4], [0.0, 0.0, 0.0, winds])
        }
    };
    // With nothing coming in there is no Paint Mode, whatever it is set to.
    let paint = match (incoming, common.paint_mode) {
        (Reads::Nothing, _) => 0.0,
        (_, PaintMode::Merge) => 1.0,
        (_, PaintMode::Add) => 2.0,
        (_, PaintMode::Subtract) => 3.0,
    };
    let edge = common.edge(size.0);
    // Level as the maths holds it: what it makes of a pixel that is covered whole.
    let level = mask::value(1.0, common.level, false);
    let uniform = [
        place[0],
        place[1],
        place[2],
        place[3],
        what,
        paint,
        // The first of its sides among those of the frame: [`SIDES_AT`].
        0.0,
        sides.len() as f32,
        extent[0],
        extent[1],
        extent[2],
        extent[3],
        edge.soft,
        edge.border,
        f32::from(edge.solid),
        f32::from(common.invert),
        level,
        0.0,
        0.0,
        0.0,
    ];
    Pass::Mask {
        incoming,
        uniform,
        sides,
    }
}

/// The colour of a Background, premultiplied by its alpha: its channels held within 0 and 1,
/// as those of the colour of a clip are. None where nothing of it is seen.
#[allow(
    clippy::cast_possible_truncation,
    reason = "channels from 0 to 1, as the card takes them"
)]
fn filled(rgba: [f64; 4]) -> Option<[f32; 4]> {
    let [r, g, b, a] = rgba.map(|channel| if channel > 0.0 { channel.min(1.0) } else { 0.0 });
    (a > 0.0).then_some([(r * a) as f32, (g * a) as f32, (b * a) as f32, a as f32])
}

/// The picture of a Transform of `input`, limited by the effect mask `limit` (nothing: it has
/// none): none where it is given none; that of its input where it changes nothing, whatever
/// its mask; and else a pass that lays it over nothing, and mixes that with the picture as it
/// came in by the mask, where it has one.
///
/// A Transform that leaves nothing of its picture is no picture where it has no mask. With
/// one it is a pass too: the picture as it came in where the mask is 0, nothing where it is
/// 1.
fn transformed(
    input: Reads,
    limit: Reads,
    settings: &Transformed,
    size: (u32, u32),
    luma: [f32; 3],
    passes: &mut Vec<Pass>,
) -> Reads {
    let scale = settings.size;
    if input == Reads::Nothing {
        return Reads::Nothing;
    }
    let lay = |uniform: [f32; PARAMS]| Pass::Lay {
        background: Reads::Nothing,
        foreground: input,
        mask: limit,
        uniform: limited(uniform, limit, LIMITED_TO_FOREGROUND),
    };
    if scale.is_nan() || scale <= 0.0 {
        if limit == Reads::Nothing {
            return Reads::Nothing;
        }
        return drawn(
            passes,
            lay(uniform(&NOWHERE, 1.0, &Settings::default(), luma)),
        );
    }
    let flips = (settings.flip_horizontal, settings.flip_vertical);
    if flips == (false, false) && lies_as_it_is(settings.center, scale, settings.angle) {
        return input;
    }
    let map = reading(size, settings.center, scale, settings.angle, flips);
    drawn(
        passes,
        lay(uniform(&map, scale, &Settings::default(), luma)),
    )
}

/// A map by which every pixel of a picture reads a place that the foreground does not reach,
/// a pixel beyond its top left corner: the foreground of a pass laid by it is nothing.
const NOWHERE: Affine = Affine {
    a: 0.0,
    b: 0.0,
    c: 0.0,
    d: 0.0,
    tx: -1.0,
    ty: -1.0,
};

/// The picture of a Merge, limited by the effect mask `limit` (nothing: it has none): its
/// background where its foreground gives nothing, or nothing is left of it, or its Blend is
/// 0, whatever its mask; its foreground where that lies as it is over no background, the
/// merge is the plain over of it and it has no mask; and else a pass, which mixes what it
/// lays with its background by the mask, where it has one.
fn merged(
    background: Reads,
    foreground: Reads,
    limit: Reads,
    settings: &Merged,
    size: (u32, u32),
    luma: [f32; 3],
    passes: &mut Vec<Pass>,
) -> Reads {
    let held = held(settings);
    let scale = settings.size;
    if foreground == Reads::Nothing || scale.is_nan() || scale <= 0.0 || held.blend <= 0.0 {
        return background;
    }
    let placed = lies_as_it_is(settings.center, scale, settings.angle);
    let plain = background == Reads::Nothing && placed && adds_as_it_is(&held);
    if plain && limit == Reads::Nothing {
        return foreground;
    }
    let map = reading(size, settings.center, scale, settings.angle, (false, false));
    drawn(
        passes,
        Pass::Lay {
            background,
            foreground,
            mask: limit,
            uniform: limited(
                uniform(&map, scale, &held, luma),
                limit,
                LIMITED_TO_BACKGROUND,
            ),
        },
    )
}

/// The uniform of `fusion.wgsl` `uniform`, of a pass that the effect mask `limit` limits
/// (nothing: it has none, and the uniform is as it was): `first` says which of the pictures
/// of the pass came into the first input of its node ([`LIMITED_TO_BACKGROUND`],
/// [`LIMITED_TO_FOREGROUND`]).
fn limited(mut uniform: [f32; PARAMS], limit: Reads, first: f32) -> [f32; PARAMS] {
    if limit != Reads::Nothing {
        uniform[LIMITED_AT] = first;
    }
    uniform
}

/// The settings of a Merge of the model as the mathematics takes them
/// ([`Settings::held`]).
#[allow(
    clippy::cast_possible_truncation,
    reason = "sliders from 0 to 1, as the card takes them"
)]
fn held(settings: &Merged) -> Settings {
    Settings {
        apply: settings.apply,
        operator: match settings.operator {
            model::Operator::Over => Operator::Over,
        },
        subtractive_additive: settings.subtractive_additive as f32,
        alpha_gain: settings.alpha_gain as f32,
        burn_in: settings.burn_in as f32,
        blend: settings.blend as f32,
    }
    .held()
}

/// Whether a merge of these settings, as [`Settings::held`] makes them, over no background is
/// its foreground as it is: Additive, with an Alpha Gain of 1, no Burn In and a Blend of 1. In
/// any Apply Mode: the share of the foreground that lies over a background is none.
#[allow(
    clippy::float_cmp,
    reason = "the ends of the sliders, which are these numbers themselves"
)]
fn adds_as_it_is(held: &Settings) -> bool {
    held.subtractive_additive == 1.0
        && held.alpha_gain == 1.0
        && held.burn_in == 0.0
        && held.blend >= 1.0
}

/// The uniform of `fusion.wgsl` for a foreground read by `map`, made `scale` times as large,
/// merged by `settings` as [`Settings::held`] makes them.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    reason = "pixels and shares, as the card takes them; a count from 1 to 8"
)]
fn uniform(map: &Affine, scale: f64, settings: &Settings, luma: [f32; 3]) -> [f32; PARAMS] {
    let mode = u8::try_from(layout::composite(settings.apply)).unwrap_or(0);
    // A foreground made smaller is read at several points of each pixel, as a picture is.
    let points = samples(1.0 / scale);
    [
        map.a as f32,
        map.b as f32,
        map.tx as f32,
        0.0,
        map.c as f32,
        map.d as f32,
        map.ty as f32,
        0.0,
        scale as f32,
        points as f32,
        f32::from(mode),
        0.0,
        settings.subtractive_additive,
        settings.alpha_gain,
        settings.burn_in,
        settings.blend,
        luma[0],
        luma[1],
        luma[2],
        0.0,
    ]
}

/// The uniform of `fusion.wgsl` for a foreground of `size` pixels that lies as it is over a
/// background of its size, merged by `settings`: for the tests that hold the shader to
/// [`crate::merge::merge_pixel`].
pub(crate) fn merge_uniform(
    size: (u32, u32),
    settings: &Settings,
    luma: [f32; 3],
) -> [f32; PARAMS] {
    let map = reading(size, [0.5, 0.5], 1.0, 0.0, (false, false));
    uniform(&map, 1.0, &settings.held(), luma)
}

/// The passes of `passes` that the picture `output` is made of, numbered anew: a pass that a
/// node which changes nothing left unread is not drawn.
fn needed(passes: Vec<Pass>, output: Reads) -> Plan {
    let mut read = vec![false; passes.len()];
    let mut next = vec![output];
    while let Some(reads) = next.pop() {
        let Reads::Pass(at) = reads else { continue };
        let Some(seen) = read.get_mut(at) else {
            continue;
        };
        if std::mem::replace(seen, true) {
            continue;
        }
        if let Some(pass) = passes.get(at) {
            next.extend(pass.reads());
        }
    }
    if read.iter().all(|read| *read) {
        return Plan { passes, output };
    }
    // The number of each pass among those that are kept.
    let mut numbers = Vec::with_capacity(passes.len());
    let mut count = 0;
    for kept in &read {
        numbers.push(count);
        count += usize::from(*kept);
    }
    let renumbered = |reads: Reads| match reads {
        Reads::Pass(at) => numbers
            .get(at)
            .map_or(Reads::Nothing, |at| Reads::Pass(*at)),
        other => other,
    };
    let passes = passes
        .into_iter()
        .zip(&read)
        .filter(|(_, kept)| **kept)
        .map(|(pass, _)| pass.with_reads(renumbered))
        .collect();
    Plan {
        passes,
        output: renumbered(output),
    }
}

/// The shader of the passes that lay: the maths of the composite modes, that of the Layer
/// Mixer of a grade, which is not written a second time ([`crate::grade::composite_library`],
/// as the shader of a layer that is blended has it), and then `fusion.wgsl`.
pub(crate) fn shader() -> String {
    format!("{}\n{SHADER}", crate::grade::composite_library())
}

/// The shader of the passes that draw masks: the feather of an edge and the distance to a
/// side as the windows of the Color page have them, which are not written a second time
/// ([`crate::grade::window_edge_library`]), and then `fusion_mask.wgsl`.
pub(crate) fn mask_shader() -> String {
    format!("{}\n{MASK_SHADER}", crate::grade::window_edge_library())
}

/// The pipeline of the passes that lay, of the shader `source` ([`shader`]), or of those that
/// draw masks ([`mask_shader`]), `layout` binding what they read: it writes each pixel as it
/// makes it, into a texture of [`FORMAT`].
pub(crate) fn pipeline(
    device: &wgpu::Device,
    source: &str,
    layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("columbina composition"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("columbina composition"),
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("columbina composition"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// Clear `target` to `colour`, premultiplied: the pass of a Background.
pub(crate) fn fill(encoder: &mut wgpu::CommandEncoder, target: &wgpu::Texture, colour: [f32; 4]) {
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let [r, g, b, a] = colour.map(f64::from);
    // A pass that draws nothing: the clear is all of it.
    drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("columbina background of a composition"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color { r, g, b, a }),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    }));
}

/// A texture of floats of `width` pixels a row that holds `pixels` as they are, to be read
/// texel by texel: for the tests that hold the shader to the processor, whose pictures are of
/// floats of 32 bits.
pub(crate) fn floats(gpu: &Gpu, width: u32, pixels: &[[f32; 4]]) -> wgpu::Texture {
    let height = u32::try_from(pixels.len() / width.max(1) as usize).unwrap_or(0);
    let texture = gpu.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("columbina floats"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    if width > 0 && height > 0 {
        let bytes: Vec<u8> = pixels
            .as_flattened()
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        gpu.queue().write_texture(
            texture.as_image_copy(),
            &bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 16),
                rows_per_image: Some(height),
            },
            texture.size(),
        );
    }
    texture
}

/// What the passes that lay and those that draw masks are drawn with on a card, but for their
/// pipelines, which the compositor keeps with its others: made the first time a composition
/// has such a pass, so that a compositor that draws none has nothing of it.
pub(crate) struct Card {
    /// What binds the uniform of a pass that lays, its background, its foreground and its
    /// effect mask.
    pub layout: wgpu::BindGroupLayout,
    /// What binds the uniform of a pass that draws a mask, the mask that comes into it and
    /// the sides of the polygons of the frame.
    pub mask_layout: wgpu::BindGroupLayout,
    /// A texel of nothing: the background of a pass that has none, and what is bound in the
    /// place of a mask where there is none, which is not read.
    nothing: wgpu::TextureView,
    /// The uniforms of the passes of a frame, a slot each.
    uniform: wgpu::Buffer,
    /// The sides of the polygons of the masks of a frame, one after another: never empty,
    /// as what is bound is to hold a side at the least.
    sides: wgpu::Buffer,
}

/// How many sides the buffer of the sides of polygons holds when it is made: those of one
/// polygon of as many points as the model allows.
const FIRST_SIDES: u64 = model::MOST_POINTS as u64;

/// The bytes of a side of a polygon: where it starts and where it ends, four floats.
const SIDE_SIZE: u64 = 16;

impl Card {
    /// What the passes are drawn with on the card of `gpu`, whose uniforms lie `slot` bytes
    /// apart.
    pub(crate) fn new(gpu: &Gpu, slot: u32) -> Card {
        let device = gpu.device();
        let picture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let buffer = |binding, ty, has_dynamic_offset, size| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty,
                has_dynamic_offset,
                min_binding_size: wgpu::BufferSize::new(size),
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("columbina composition"),
            entries: &[
                buffer(0, wgpu::BufferBindingType::Uniform, true, PARAMS_SIZE),
                picture(1),
                picture(2),
                picture(3),
            ],
        });
        let storage = wgpu::BufferBindingType::Storage { read_only: true };
        let mask_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("columbina mask of a composition"),
            entries: &[
                buffer(0, wgpu::BufferBindingType::Uniform, true, PARAMS_SIZE),
                picture(1),
                buffer(2, storage, false, SIDE_SIZE),
            ],
        });
        Card {
            layout,
            mask_layout,
            nothing: nothing(gpu),
            uniform: uniform_buffer(device, u64::from(slot) * 4),
            sides: sides_buffer(device, FIRST_SIDES * SIDE_SIZE),
        }
    }

    /// Write the sides of the polygons of the masks of a frame, one after another, into
    /// their buffer, made larger if it is too small. A frame without a polygon writes
    /// nothing: no pass reads a side.
    pub(crate) fn write_sides(&mut self, gpu: &Gpu, sides: &[[f32; 4]]) {
        if sides.is_empty() {
            return;
        }
        let bytes: Vec<u8> = sides
            .as_flattened()
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        if self.sides.size() < bytes.len() as u64 {
            self.sides = sides_buffer(gpu.device(), (bytes.len() as u64).next_power_of_two());
        }
        gpu.queue().write_buffer(&self.sides, 0, &bytes);
    }

    /// Write the uniforms of the passes of a frame, a slot of `slot` bytes each, into their
    /// buffer, made larger if it is too small; give the offset of each.
    pub(crate) fn write(&mut self, gpu: &Gpu, slot: u32, uniforms: &[[f32; PARAMS]]) -> Vec<u32> {
        let slot = slot as usize;
        let mut bytes = vec![0_u8; uniforms.len() * slot];
        for (values, at) in uniforms.iter().zip(bytes.chunks_mut(slot.max(1))) {
            for (value, into) in values.iter().zip(at.chunks_mut(4)) {
                into.copy_from_slice(&value.to_le_bytes());
            }
        }
        if self.uniform.size() < bytes.len() as u64 {
            self.uniform = uniform_buffer(gpu.device(), (bytes.len() as u64).next_power_of_two());
        }
        gpu.queue().write_buffer(&self.uniform, 0, &bytes);
        (0..uniforms.len())
            .map(|i| u32::try_from(i * slot).unwrap_or(u32::MAX))
            .collect()
    }

    /// Draw a pass that lays `foreground` over `background` (nothing, where there is none)
    /// into `target`, by `pipeline` and the uniform at `offset`, limited by the alpha of
    /// `mask` where the node has an effect mask.
    #[allow(
        clippy::too_many_arguments,
        reason = "what a pass is drawn with, each of another kind"
    )]
    pub(crate) fn lay(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::RenderPipeline,
        offset: u32,
        background: Option<&wgpu::Texture>,
        foreground: &wgpu::Texture,
        mask: Option<&wgpu::Texture>,
        target: &wgpu::Texture,
    ) {
        let under = self.view(background);
        let over = self.view(Some(foreground));
        let limit = self.view(mask);
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("columbina composition"),
            layout: &self.layout,
            entries: &[
                self.params(),
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&under),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&over),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&limit),
                },
            ],
        });
        draw(encoder, pipeline, &bind, offset, target);
    }

    /// Draw a pass that draws a mask, or a Background times its effect mask, into `target`,
    /// by `pipeline` and the uniform at `offset`: `incoming` is the picture whose alpha is the
    /// mask that comes in (nothing, where none does). The sides of its polygon are among
    /// those written last ([`Card::write_sides`]).
    pub(crate) fn mask(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::RenderPipeline,
        offset: u32,
        incoming: Option<&wgpu::Texture>,
        target: &wgpu::Texture,
    ) {
        let under = self.view(incoming);
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("columbina mask of a composition"),
            layout: &self.mask_layout,
            entries: &[
                self.params(),
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&under),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.sides.as_entire_binding(),
                },
            ],
        });
        draw(encoder, pipeline, &bind, offset, target);
    }

    /// What binds the uniform of a pass: a slot of the buffer of the uniforms of the frame,
    /// which the offset of the pass chooses.
    fn params(&self) -> wgpu::BindGroupEntry<'_> {
        wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer: &self.uniform,
                offset: 0,
                size: wgpu::BufferSize::new(PARAMS_SIZE),
            }),
        }
    }

    /// The whole of `picture` as a pass reads it; the texel of nothing where there is none.
    fn view(&self, picture: Option<&wgpu::Texture>) -> wgpu::TextureView {
        picture.map_or_else(
            || self.nothing.clone(),
            |picture| picture.create_view(&wgpu::TextureViewDescriptor::default()),
        )
    }
}

/// Draw a pass of a composition into `target`, over nothing, by `pipeline`, with what `bind`
/// binds and the uniform at `offset`.
fn draw(
    encoder: &mut wgpu::CommandEncoder,
    pipeline: &wgpu::RenderPipeline,
    bind: &wgpu::BindGroup,
    offset: u32,
    target: &wgpu::Texture,
) {
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("columbina composition"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, bind, &[offset]);
    pass.draw(0..3, 0..1);
}

/// A view of a texel of nothing on the card of `gpu`: what a pass reads where it has no
/// picture.
fn nothing(gpu: &Gpu) -> wgpu::TextureView {
    let nothing = gpu.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("columbina nothing"),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    gpu.queue().write_texture(
        nothing.as_image_copy(),
        &[0; 4],
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4),
            rows_per_image: Some(1),
        },
        nothing.size(),
    );
    nothing.create_view(&wgpu::TextureViewDescriptor::default())
}

/// A buffer of the uniforms of passes, of `size` bytes.
fn uniform_buffer(device: &wgpu::Device, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("columbina composition"),
        size,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// A buffer of the sides of the polygons of masks, of `size` bytes.
fn sides_buffer(device: &wgpu::Device, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("columbina sides of polygons"),
        size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::float_cmp,
        reason = "maps of whole pixels and quarter turns, which floats keep exactly"
    )]

    use columbina_model::fusion::FusedNode;
    use columbina_model::grade::Composite;

    use super::*;
    use crate::grade::REC709_LUMA;

    const SIZE: (u32, u32) = (16, 8);

    fn node(id: &str, kind: FusedKind) -> FusedNode {
        FusedNode {
            id: id.into(),
            kind,
        }
    }

    /// A Merge as it is at first.
    fn merge() -> Merged {
        Merged {
            apply: Composite::Normal,
            operator: model::Operator::Over,
            subtractive_additive: 1.0,
            alpha_gain: 1.0,
            burn_in: 0.0,
            blend: 1.0,
            center: [0.5, 0.5],
            size: 1.0,
            angle: 0.0,
        }
    }

    /// A Transform as it is at first.
    fn transform() -> Transformed {
        Transformed {
            center: [0.5, 0.5],
            size: 1.0,
            angle: 0.0,
            flip_horizontal: false,
            flip_vertical: false,
        }
    }

    fn background(rgba: [f64; 4]) -> FusedKind {
        FusedKind::Background { rgba, mask: None }
    }

    fn planned(nodes: Vec<FusedNode>, output: Option<usize>) -> Plan {
        plan_of(nodes, output, SIZE, true)
    }

    /// The passes of `nodes`, the `MediaOut` reading the one at `output`, drawn `size` pixels
    /// large; `source` says whether the source of the clip gives a picture.
    fn plan_of(
        nodes: Vec<FusedNode>,
        output: Option<usize>,
        size: (u32, u32),
        source: bool,
    ) -> Plan {
        plan(&Fused { nodes, output }, size, REC709_LUMA, source)
    }

    #[test]
    fn the_composition_that_a_clip_has_at_first_is_its_source_with_no_pass() {
        let first = planned(vec![node("in", FusedKind::MediaIn)], Some(0));
        assert_eq!(
            first,
            Plan {
                passes: vec![],
                output: Reads::Source
            }
        );
        assert!(first.reads_source() && !first.lays());
        // A MediaOut that reads nothing, and one of a clip whose source gives no picture.
        assert_eq!(planned(vec![], None).output, Reads::Nothing);
        let unseen = plan(
            &Fused {
                nodes: vec![node("in", FusedKind::MediaIn)],
                output: Some(0),
            },
            SIZE,
            REC709_LUMA,
            false,
        );
        assert_eq!(unseen, Plan::default());
        // A place that is not in the list reads nothing.
        assert_eq!(planned(vec![], Some(3)).output, Reads::Nothing);
    }

    #[test]
    fn a_node_that_changes_nothing_is_no_pass() {
        // A Transform as it is at first, and one turned a whole turn.
        for angle in [0.0, 360.0, -720.0] {
            let kind = FusedKind::Transform {
                input: Some(0),
                mask: None,
                settings: Transformed {
                    angle,
                    ..transform()
                },
            };
            let plan = planned(
                vec![node("in", FusedKind::MediaIn), node("t", kind)],
                Some(1),
            );
            assert_eq!(plan.passes, [] as [Pass; 0], "{angle}");
            assert_eq!(plan.output, Reads::Source, "{angle}");
        }
        // A Merge as it is at first over no background is its foreground; with no foreground,
        // with nothing left of it, or with a Blend of 0, it is its background.
        let over = |background, foreground, settings| {
            planned(
                vec![
                    node("in", FusedKind::MediaIn),
                    node("bg", self::background([0.2, 0.4, 0.6, 1.0])),
                    node(
                        "over",
                        FusedKind::Merge {
                            background,
                            foreground,
                            mask: None,
                            settings,
                        },
                    ),
                ],
                Some(2),
            )
        };
        assert_eq!(over(None, Some(0), merge()).output, Reads::Source);
        assert_eq!(over(None, Some(0), merge()).passes, [] as [Pass; 0]);
        let fill = Pass::Fill([0.2, 0.4, 0.6, 1.0]);
        for (foreground, settings) in [
            (None, merge()),
            (
                Some(0),
                Merged {
                    size: 0.0,
                    ..merge()
                },
            ),
            (
                Some(0),
                Merged {
                    blend: 0.0,
                    ..merge()
                },
            ),
        ] {
            let plan = over(Some(1), foreground, settings);
            assert_eq!(plan.passes, vec![fill.clone()]);
            assert_eq!(plan.output, Reads::Pass(0));
            assert!(!plan.reads_source() && !plan.lays());
        }
        // A Merge that is not the plain over of its foreground is a pass, over nothing too.
        let half = over(
            None,
            Some(0),
            Merged {
                blend: 0.5,
                ..merge()
            },
        );
        assert_eq!(half.passes.len(), 1);
        assert!(half.lays() && half.reads_source());
        // A Transform that leaves nothing of its picture, and one that is given none.
        for (input, size) in [(Some(0), 0.0), (None, 2.0)] {
            let kind = FusedKind::Transform {
                input,
                mask: None,
                settings: Transformed {
                    size,
                    ..transform()
                },
            };
            let plan = planned(
                vec![node("in", FusedKind::MediaIn), node("t", kind)],
                Some(1),
            );
            assert_eq!(plan, Plan::default());
        }
        // A Background of which nothing is seen is no picture.
        let clear = planned(vec![node("bg", background([1.0, 1.0, 1.0, 0.0]))], Some(0));
        assert_eq!(clear, Plan::default());
    }

    #[test]
    fn a_background_is_premultiplied_and_held_within_0_and_1() {
        assert_eq!(filled([1.0, 0.5, 0.25, 0.5]), Some([0.5, 0.25, 0.125, 0.5]));
        assert_eq!(
            filled([2.0, -1.0, f64::NAN, 3.0]),
            Some([1.0, 0.0, 0.0, 1.0])
        );
        assert_eq!(filled([1.0, 1.0, 1.0, 0.0]), None);
        assert_eq!(filled([1.0, 1.0, 1.0, f64::NAN]), None);
    }

    #[test]
    fn the_nodes_of_a_graph_are_passes_each_after_those_that_it_reads() {
        // A background under the picture of the clip, moved: bg and in into over, whose
        // picture a Transform flips.
        let nodes = vec![
            node("in", FusedKind::MediaIn),
            node("bg", background([0.0, 0.0, 0.0, 1.0])),
            node(
                "over",
                FusedKind::Merge {
                    background: Some(1),
                    foreground: Some(0),
                    mask: None,
                    settings: Merged {
                        center: [0.75, 0.5],
                        ..merge()
                    },
                },
            ),
            node(
                "flip",
                FusedKind::Transform {
                    input: Some(2),
                    mask: None,
                    settings: Transformed {
                        flip_horizontal: true,
                        ..transform()
                    },
                },
            ),
        ];
        let plan = planned(nodes, Some(3));
        assert_eq!(plan.output, Reads::Pass(2));
        assert!(matches!(plan.passes[0], Pass::Fill(_)));
        let Pass::Lay {
            background: under,
            foreground: over,
            ..
        } = &plan.passes[1]
        else {
            panic!("a merge");
        };
        assert_eq!((*under, *over), (Reads::Pass(0), Reads::Source));
        let Pass::Lay {
            background: under,
            foreground: over,
            ..
        } = &plan.passes[2]
        else {
            panic!("a transform");
        };
        assert_eq!((*under, *over), (Reads::Nothing, Reads::Pass(1)));
        // The source and the background are read by the merge alone, and the flip draws into
        // the texture of one of them: three textures, where five pictures are made.
        let slots = plan.slots();
        assert_eq!(slots.count, 3);
        assert_eq!(slots.source, Some(0));
        assert_eq!(slots.passes[..2], [1, 2]);
        assert_ne!(slots.passes[2], slots.passes[1]);
        // A long chain of flips takes two, each drawn into the one that the last one read.
        let mut chain = vec![node("bg", background([1.0, 0.0, 0.0, 1.0]))];
        for at in 0..9 {
            chain.push(node(
                "flip",
                FusedKind::Transform {
                    input: Some(at),
                    mask: None,
                    settings: Transformed {
                        flip_vertical: true,
                        ..transform()
                    },
                },
            ));
        }
        let plan = planned(chain, Some(9));
        assert_eq!(plan.passes.len(), 10);
        let slots = plan.slots();
        assert_eq!(slots.count, 2);
        assert_eq!(slots.source, None);
        for pair in slots.passes.windows(2) {
            assert_ne!(pair[0], pair[1]);
        }
    }

    #[test]
    fn a_pass_that_nothing_reads_is_not_drawn() {
        // The merge has a Blend of 0, and is its background: the flip of its foreground is
        // read by nothing.
        let nodes = vec![
            node("in", FusedKind::MediaIn),
            node(
                "flip",
                FusedKind::Transform {
                    input: Some(0),
                    mask: None,
                    settings: Transformed {
                        flip_horizontal: true,
                        ..transform()
                    },
                },
            ),
            node("bg", background([0.0, 1.0, 0.0, 1.0])),
            node(
                "over",
                FusedKind::Merge {
                    background: Some(2),
                    foreground: Some(1),
                    mask: None,
                    settings: Merged {
                        blend: 0.0,
                        ..merge()
                    },
                },
            ),
        ];
        let plan = planned(nodes, Some(3));
        assert_eq!(plan.passes, vec![Pass::Fill([0.0, 1.0, 0.0, 1.0])]);
        assert_eq!(plan.output, Reads::Pass(0));
        assert!(!plan.reads_source());
        assert_eq!(
            plan.slots(),
            Slots {
                source: None,
                passes: vec![0],
                count: 1
            }
        );
    }

    #[test]
    fn a_picture_is_put_where_the_settings_of_fusion_say() {
        // As it is: each pixel reads itself.
        let same = reading(SIZE, [0.5, 0.5], 1.0, 0.0, (false, false));
        assert_eq!(same.apply((3.5, 2.5)), (3.5, 2.5));
        // Two pixels to the right and one up: `center.y` counts from the bottom.
        let moved = reading(
            SIZE,
            [0.5 + 2.0 / 16.0, 0.5 + 1.0 / 8.0],
            1.0,
            0.0,
            (false, false),
        );
        assert_eq!(moved.apply((5.5, 2.5)), (3.5, 3.5));
        // Twice as large about its middle: a pixel reads half as far from the middle.
        let larger = reading(SIZE, [0.5, 0.5], 2.0, 0.0, (false, false));
        assert_eq!(larger.apply((12.0, 6.0)), (10.0, 5.0));
        // Flipped left for right, and top for bottom.
        let flipped = reading(SIZE, [0.5, 0.5], 1.0, 0.0, (true, false));
        assert_eq!(flipped.apply((0.5, 2.5)), (15.5, 2.5));
        let flipped = reading(SIZE, [0.5, 0.5], 1.0, 0.0, (false, true));
        assert_eq!(flipped.apply((0.5, 2.5)), (0.5, 5.5));
        // A quarter turn counter-clockwise, of a square: its top right corner is put at the
        // top left, so that the pixel there reads it.
        let turned = reading((8, 8), [0.5, 0.5], 1.0, 90.0, (false, false));
        assert_eq!(turned.apply((0.5, 0.5)), (7.5, 0.5));
        assert_eq!(turned.apply((0.5, 7.5)), (0.5, 0.5));
        // And clockwise: its top left corner is put at the top right.
        let turned = reading((8, 8), [0.5, 0.5], 1.0, -90.0, (false, false));
        assert_eq!(turned.apply((7.5, 0.5)), (0.5, 0.5));
        // Any other angle, by its sine and cosine.
        let (sin, cos) = turn(30.0);
        assert!((sin - 0.5).abs() < 1e-12 && (cos - 0.75_f64.sqrt()).abs() < 1e-12);
        assert_eq!(turn(180.0), (0.0, -1.0));
        assert_eq!(turn(-90.0), (-1.0, 0.0));
        assert_eq!(turn(3600.0), (0.0, 1.0));
    }

    #[test]
    fn the_pictures_of_a_composition_are_drawn_as_large_as_they_are_seen() {
        let most = 8192;
        // As large as the canvas, or smaller than it: at its own size.
        assert_eq!(
            drawn_size((1920, 1080), (1920.0, 1080.0), 1.0, most),
            Some((1920, 1080))
        );
        assert_eq!(
            drawn_size((640, 360), (1920.0, 1080.0), 1.0, most),
            Some((640, 360))
        );
        // Larger than the canvas: as it is fitted, so that each pixel is seen as it is drawn.
        assert_eq!(
            drawn_size((3840, 2160), (1920.0, 1080.0), 1.0, most),
            Some((1920, 1080))
        );
        assert_eq!(
            drawn_size((1920, 1080), (960.0, 540.0), 1.0, most),
            Some((960, 540))
        );
        // Made larger, at more of its own pixels, and never at more than it has.
        assert_eq!(
            drawn_size((3840, 2160), (1920.0, 1080.0), 2.0, most),
            Some((3840, 2160))
        );
        assert_eq!(
            drawn_size((3840, 2160), (1920.0, 1080.0), 8.0, most),
            Some((3840, 2160))
        );
        // Made smaller: as a title is, the same down to a half, and then by quarters of an
        // octave.
        assert_eq!(
            drawn_size((1920, 1080), (1920.0, 1080.0), 0.6, most),
            Some((1920, 1080))
        );
        assert_eq!(
            drawn_size((1920, 1080), (1920.0, 1080.0), 0.25, most),
            Some((480, 270))
        );
        // No side beyond what the card takes, and nothing of what has no pixels.
        assert_eq!(
            drawn_size((16384, 8192), (16384.0, 8192.0), 1.0, most),
            Some((8192, 4096))
        );
        assert_eq!(drawn_size((0, 1080), (1920.0, 1080.0), 1.0, most), None);
        assert_eq!(drawn_size((1920, 1080), (0.0, 1080.0), 1.0, most), None);
    }

    /// The controls of a mask as they are at first.
    fn common() -> model::Masked {
        model::Ellipse::default().common_at(0.0)
    }

    /// An Ellipse mask as it is at first, into which the mask `mask` comes.
    fn ellipse(mask: Option<usize>) -> FusedKind {
        FusedKind::Ellipse {
            mask,
            common: common(),
            shape: model::Ellipse::default().at(0.0),
        }
    }

    /// A Rectangle mask as it is at first, into which the mask `mask` comes.
    fn rectangle(mask: Option<usize>) -> FusedKind {
        FusedKind::Rectangle {
            mask,
            common: common(),
            shape: model::Rectangle::default().at(0.0),
        }
    }

    /// A Polygon mask of `points` about the middle of the picture, added to the mask `mask`
    /// that comes into it.
    fn polygon(mask: Option<usize>, points: Vec<[f64; 2]>, fill: model::FillMethod) -> FusedKind {
        FusedKind::Polygon {
            mask,
            common: model::Masked {
                paint_mode: model::PaintMode::Add,
                ..common()
            },
            shape: model::PolygonShape {
                center: [0.5, 0.5],
                size: 1.0,
                angle: 0.0,
                points,
                fill,
            },
        }
    }

    /// A Transform of the picture at `input` that flips it left for right, with the effect
    /// mask `mask`.
    fn flipped(input: usize, mask: Option<usize>) -> FusedKind {
        FusedKind::Transform {
            input: Some(input),
            mask,
            settings: Transformed {
                flip_horizontal: true,
                ..transform()
            },
        }
    }

    #[test]
    fn a_mask_node_is_a_pass_of_its_settings_in_the_pixels_of_the_picture() {
        let empty = polygon(None, vec![], model::FillMethod::Alternate);
        for kind in [ellipse(None), rectangle(None), empty] {
            assert!(is_drawn(&kind), "{kind:?}");
        }
        // A Rectangle a quarter of the way from the left and from the top, with rounded
        // corners, an edge, a border and a level of its own, not solid and inverted.
        let kind = FusedKind::Rectangle {
            mask: None,
            common: model::Masked {
                level: 0.5,
                soft_edge: 0.25,
                border_width: -0.125,
                invert: true,
                solid: false,
                paint_mode: model::PaintMode::Subtract,
            },
            shape: model::RectangleShape {
                center: [0.25, 0.75],
                width: 0.5,
                height: 0.5,
                corner_radius: 1.0,
                angle: 0.0,
            },
        };
        let plan = planned(vec![node("r", kind)], Some(0));
        assert_eq!(plan.output, Reads::Pass(0));
        assert!(plan.masks() && !plan.lays() && !plan.reads_source());
        let [Pass::Mask {
            incoming,
            uniform,
            sides,
        }] = plan.passes.as_slice()
        else {
            panic!("a mask");
        };
        assert_eq!(*incoming, Reads::Nothing);
        assert_eq!(*sides, [] as [[f32; 4]; 0]);
        // In a picture of 16 by 8: its middle at (4, 2), `center.y` counting from the bottom;
        // not turned.
        assert_eq!(uniform[..4], [4.0, 2.0, 1.0, 0.0]);
        // A Rectangle; no Paint Mode, whatever it is set to, as nothing comes in; no sides.
        assert_eq!(uniform[4..8], [0.0, 0.0, 0.0, 0.0]);
        // Half its width and half its height, and corners as round as the lesser allows.
        assert_eq!(uniform[8..12], [4.0, 2.0, 2.0, 0.0]);
        // Half the Soft Edge and half the Border Width, in widths of the picture made
        // pixels; not solid; inverted. And its Level.
        assert_eq!(uniform[12..16], [2.0, -1.0, 0.0, 1.0]);
        assert_eq!(uniform[16..], [0.5, 0.0, 0.0, 0.0]);
        assert_eq!(u64::try_from(uniform.len() * 4).unwrap(), PARAMS_SIZE);
        // At twice the size the same shares are twice the pixels.
        let twice = plan_of(vec![node("e", ellipse(None))], Some(0), (32, 16), true);
        let [Pass::Mask { uniform, .. }] = twice.passes.as_slice() else {
            panic!("a mask");
        };
        // An Ellipse, whose height is in widths of the picture as its width is: a circle.
        assert_eq!(uniform[..4], [16.0, 8.0, 1.0, 0.0]);
        assert_eq!(uniform[4], 1.0);
        assert_eq!(uniform[8..12], [8.0, 8.0, 0.0, 0.0]);
        assert_eq!(uniform[12..17], [0.0, 0.0, 1.0, 0.0, 1.0]);
    }

    #[test]
    fn the_sides_of_a_polygon_go_with_its_pass_and_the_mask_that_comes_in_is_read() {
        // A triangle about the middle, its points in widths of the picture, y up.
        let triangle = vec![[-0.25, -0.25], [0.25, -0.25], [0.0, 0.25]];
        let nodes = |fill| {
            vec![
                node("in", FusedKind::MediaIn),
                node("p", polygon(Some(0), triangle.clone(), fill)),
            ]
        };
        let plan = planned(nodes(model::FillMethod::NonZeroWinding), Some(1));
        assert!(plan.masks() && !plan.lays() && plan.reads_source());
        let [Pass::Mask {
            incoming,
            uniform,
            sides,
        }] = plan.passes.as_slice()
        else {
            panic!("a mask");
        };
        // The mask that comes in is the alpha of the picture of the clip, added to.
        assert_eq!(*incoming, Reads::Source);
        // In pixels of a picture of 16 by 8, y down; closed from the last point to the first.
        assert_eq!(
            *sides,
            [
                [8.0, 0.0, 4.0, 8.0],
                [4.0, 8.0, 12.0, 8.0],
                [12.0, 8.0, 8.0, 0.0]
            ]
        );
        // A Polygon; Add; the first of its sides is for who draws the frame to say; three
        // sides. And the Non Zero Winding fill.
        assert_eq!(uniform[4..8], [2.0, 2.0, 0.0, 3.0]);
        assert_eq!(uniform[SIDES_AT], 0.0);
        assert_eq!(uniform[8..12], [0.0, 0.0, 0.0, 1.0]);
        // The Alternate fill; and of a clip whose source shows nothing no mask comes in, so
        // that there is no Paint Mode.
        let unseen = plan_of(nodes(model::FillMethod::Alternate), Some(1), SIZE, false);
        let [Pass::Mask {
            incoming, uniform, ..
        }] = unseen.passes.as_slice()
        else {
            panic!("a mask");
        };
        assert_eq!(*incoming, Reads::Nothing);
        assert_eq!(uniform[4..8], [2.0, 0.0, 0.0, 3.0]);
        assert_eq!(uniform[11], 0.0);
        // Fewer than three points have no sides: a mask of nothing, which is a pass still.
        let two = vec![[0.0, 0.0], [0.25, 0.25]];
        let open = planned(
            vec![node("p", polygon(None, two, model::FillMethod::Alternate))],
            Some(0),
        );
        let [Pass::Mask { uniform, sides, .. }] = open.passes.as_slice() else {
            panic!("a mask");
        };
        assert_eq!(*sides, [] as [[f32; 4]; 0]);
        assert_eq!(uniform[4..8], [2.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "a graph and what each of its passes reads"
    )]
    fn the_effect_mask_of_a_node_is_read_by_the_pass_that_draws_it() {
        let fill = |mask| FusedKind::Background {
            rgba: [1.0, 0.5, 0.25, 0.5],
            mask,
        };
        let over = |background, foreground, mask| FusedKind::Merge {
            background,
            foreground,
            mask,
            settings: Merged {
                center: [0.75, 0.5],
                ..merge()
            },
        };
        // The picture of the clip over a Background: the Merge limited by an Ellipse into
        // which a Rectangle comes, and the Background by the Rectangle.
        let plan = planned(
            vec![
                node("r", rectangle(None)),
                node("bg", fill(Some(0))),
                node("in", FusedKind::MediaIn),
                node("e", ellipse(Some(0))),
                node("over", over(Some(1), Some(2), Some(3))),
            ],
            Some(4),
        );
        assert_eq!(plan.passes.len(), 4);
        assert_eq!(plan.output, Reads::Pass(3));
        assert!(plan.lays() && plan.masks() && plan.reads_source());
        // The Background: a pass in place of the clear, its colour premultiplied, which reads
        // the mask.
        let Pass::Mask {
            incoming,
            uniform,
            sides,
        } = &plan.passes[1]
        else {
            panic!("a background with a mask");
        };
        assert_eq!(*incoming, Reads::Pass(0));
        assert_eq!(uniform[..8], [0.5, 0.25, 0.125, 0.5, 3.0, 1.0, 0.0, 0.0]);
        assert_eq!(uniform[8..], [0.0; 12]);
        assert_eq!(*sides, [] as [[f32; 4]; 0]);
        // The Ellipse: merged with the mask that comes in.
        let Pass::Mask {
            incoming, uniform, ..
        } = &plan.passes[2]
        else {
            panic!("an ellipse");
        };
        assert_eq!(*incoming, Reads::Pass(0));
        assert_eq!(uniform[4..6], [1.0, 1.0]);
        // The Merge: one pass, which reads its mask too, and mixes with its background.
        let Pass::Lay {
            background,
            foreground,
            mask,
            uniform,
        } = &plan.passes[3]
        else {
            panic!("a merge");
        };
        assert_eq!(
            (*background, *foreground, *mask),
            (Reads::Pass(1), Reads::Source, Reads::Pass(2))
        );
        assert_eq!(uniform[LIMITED_AT], LIMITED_TO_BACKGROUND);
        // The picture of the Rectangle is free once the Ellipse, the last to read it, is
        // drawn: the Merge draws into its texture.
        assert_eq!(
            plan.slots(),
            Slots {
                source: Some(0),
                passes: vec![1, 2, 3, 1],
                count: 4
            }
        );
        // A Transform: its mask is read by its pass, which mixes with its picture as it
        // came in. Without a mask the pass is as it was, and the mask node is not drawn.
        let turned = |mask| {
            planned(
                vec![
                    node("in", FusedKind::MediaIn),
                    node("e", ellipse(None)),
                    node("flip", flipped(0, mask)),
                ],
                Some(2),
            )
        };
        let limited = turned(Some(1));
        assert_eq!(limited.passes.len(), 2);
        let Pass::Lay {
            background,
            foreground,
            mask,
            uniform,
        } = &limited.passes[1]
        else {
            panic!("a transform");
        };
        assert_eq!(
            (*background, *foreground, *mask),
            (Reads::Nothing, Reads::Source, Reads::Pass(0))
        );
        assert_eq!(uniform[LIMITED_AT], LIMITED_TO_FOREGROUND);
        let bare = turned(None);
        let [Pass::Lay {
            mask,
            uniform: unlimited,
            ..
        }] = bare.passes.as_slice()
        else {
            panic!("a transform alone");
        };
        assert_eq!(*mask, Reads::Nothing);
        assert_eq!(unlimited[LIMITED_AT], 0.0);
        let mut same = *uniform;
        same[LIMITED_AT] = 0.0;
        assert_eq!(same, *unlimited);
        assert!(!bare.masks());
        // A mask node read as a picture: the foreground of a Merge over a Background.
        let white = planned(
            vec![
                node("bg", fill(None)),
                node("e", ellipse(None)),
                node("over", over(Some(0), Some(1), None)),
            ],
            Some(2),
        );
        let Pass::Lay {
            foreground, mask, ..
        } = &white.passes[2]
        else {
            panic!("a merge of a mask");
        };
        assert_eq!((*foreground, *mask), (Reads::Pass(1), Reads::Nothing));
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "a graph for each node that a mask cannot change"
    )]
    fn a_node_with_a_mask_is_no_pass_where_the_mask_cannot_change_it() {
        // In each graph the `MediaIn` is at 0 and an Ellipse, the mask, at 1.
        let with = |kinds: Vec<FusedKind>| {
            let mut nodes = vec![node("in", FusedKind::MediaIn), node("e", ellipse(None))];
            nodes.extend(kinds.into_iter().map(|kind| node("n", kind)));
            let output = nodes.len() - 1;
            planned(nodes, Some(output))
        };
        // A Transform as it is at first is its input, whatever its mask: and the mask, which
        // nothing reads then, is not drawn.
        let first = with(vec![FusedKind::Transform {
            input: Some(0),
            mask: Some(1),
            settings: transform(),
        }]);
        assert_eq!(first.passes, [] as [Pass; 0]);
        assert_eq!(first.output, Reads::Source);
        // A Merge whose result is its background, whatever its mask: with no foreground,
        // with nothing left of it, or with a Blend of 0.
        let fill = Pass::Fill([0.2, 0.4, 0.6, 1.0]);
        for (foreground, settings) in [
            (None, merge()),
            (
                Some(0),
                Merged {
                    size: 0.0,
                    ..merge()
                },
            ),
            (
                Some(0),
                Merged {
                    blend: 0.0,
                    ..merge()
                },
            ),
        ] {
            let plan = with(vec![
                background([0.2, 0.4, 0.6, 1.0]),
                FusedKind::Merge {
                    background: Some(2),
                    foreground,
                    mask: Some(1),
                    settings,
                },
            ]);
            assert_eq!(plan.passes, vec![fill.clone()]);
            assert_eq!(plan.output, Reads::Pass(0));
        }
        // A Transform of nothing is nothing, and so is a Background of which nothing is seen.
        let nothing = with(vec![FusedKind::Transform {
            input: None,
            mask: Some(1),
            settings: Transformed {
                angle: 30.0,
                ..transform()
            },
        }]);
        assert_eq!(nothing, Plan::default());
        let clear = with(vec![FusedKind::Background {
            rgba: [1.0, 1.0, 1.0, 0.0],
            mask: Some(1),
        }]);
        assert_eq!(clear, Plan::default());
        // But a Merge as it is at first over no background, which without a mask is its
        // foreground, is with one its foreground times the mask: a pass.
        let over = |mask| {
            with(vec![FusedKind::Merge {
                background: None,
                foreground: Some(0),
                mask,
                settings: merge(),
            }])
        };
        assert_eq!(over(None).passes, [] as [Pass; 0]);
        assert_eq!(over(None).output, Reads::Source);
        let limited = over(Some(1));
        assert_eq!(limited.passes.len(), 2);
        let Pass::Lay {
            background,
            foreground,
            mask,
            uniform,
        } = &limited.passes[1]
        else {
            panic!("a merge");
        };
        assert_eq!(
            (*background, *foreground, *mask),
            (Reads::Nothing, Reads::Source, Reads::Pass(0))
        );
        assert_eq!(uniform[LIMITED_AT], LIMITED_TO_BACKGROUND);
        // And a Transform that leaves nothing of its picture, which without a mask is no
        // picture, is with one the picture as it came in where the mask is not 1: a pass
        // whose foreground is laid nowhere.
        let gone = |mask| {
            with(vec![FusedKind::Transform {
                input: Some(0),
                mask,
                settings: Transformed {
                    size: 0.0,
                    ..transform()
                },
            }])
        };
        assert_eq!(gone(None), Plan::default());
        let left = gone(Some(1));
        assert_eq!(left.passes.len(), 2);
        let Pass::Lay {
            background,
            foreground,
            mask,
            uniform,
        } = &left.passes[1]
        else {
            panic!("a transform");
        };
        assert_eq!(
            (*background, *foreground, *mask),
            (Reads::Nothing, Reads::Source, Reads::Pass(0))
        );
        assert_eq!(uniform[..8], [0.0, 0.0, -1.0, 0.0, 0.0, 0.0, -1.0, 0.0]);
        assert_eq!(uniform[8..12], [1.0, 1.0, 0.0, LIMITED_TO_FOREGROUND]);
        assert_eq!(uniform[12..16], [1.0, 1.0, 0.0, 1.0]);
    }

    #[test]
    fn a_mask_that_gives_no_picture_is_no_mask() {
        // Every kind that has a mask, each with the mask `mask`: a Background, an Ellipse, a
        // Transform and a Merge, all of which the last Merge reads.
        let graph = |mask: Option<usize>| {
            vec![
                node("in", FusedKind::MediaIn),
                node("clear", background([1.0, 1.0, 1.0, 0.0])),
                node("red", background([1.0, 0.0, 0.0, 1.0])),
                node(
                    "bg",
                    FusedKind::Background {
                        rgba: [0.0, 0.0, 1.0, 1.0],
                        mask,
                    },
                ),
                node("e", ellipse(mask)),
                node("flip", flipped(2, mask)),
                node(
                    "over",
                    FusedKind::Merge {
                        background: Some(3),
                        foreground: Some(5),
                        mask,
                        settings: Merged {
                            center: [0.75, 0.5],
                            ..merge()
                        },
                    },
                ),
                node(
                    "last",
                    FusedKind::Merge {
                        background: Some(6),
                        foreground: Some(4),
                        mask: None,
                        settings: Merged {
                            center: [0.25, 0.5],
                            ..merge()
                        },
                    },
                ),
            ]
        };
        let without = planned(graph(None), Some(7));
        assert_eq!(without.passes.len(), 6);
        // The `MediaIn` of a clip whose source shows nothing.
        let unseen = |mask| plan_of(graph(mask), Some(7), SIZE, false);
        assert_eq!(unseen(Some(0)), unseen(None));
        assert_eq!(unseen(None), without);
        // A picture that is known to be nothing: a Background of which nothing is seen.
        assert_eq!(planned(graph(Some(1)), Some(7)), without);
        // A place that is not before the node.
        assert_eq!(planned(graph(Some(9)), Some(7)), without);
        // But a picture is a mask, whatever made it: the picture of the clip (at 0), which
        // is that of its source, and the red Background (at 2), which is that of the first
        // pass.
        for (mask, picture) in [(0, Reads::Source), (2, Reads::Pass(0))] {
            let limited = planned(graph(Some(mask)), Some(7));
            assert_ne!(limited, without, "{mask}");
            assert_eq!(limited.passes.len(), 6, "{mask}");
            assert_eq!(limited.passes[0], Pass::Fill([1.0, 0.0, 0.0, 1.0]));
            // The Background, the Ellipse, the Transform and the Merge read it, each in one
            // pass.
            let readers = limited
                .passes
                .iter()
                .filter(|pass| pass.reads().contains(&picture))
                .count();
            assert_eq!(readers, 4, "{mask}");
        }
        // And a mask node is a picture though its mask is 0 everywhere: it limits the node
        // that reads it, to nothing.
        let none = FusedKind::Ellipse {
            mask: None,
            common: model::Masked {
                level: 0.0,
                ..common()
            },
            shape: model::Ellipse::default().at(0.0),
        };
        let plan = planned(
            vec![
                node("in", FusedKind::MediaIn),
                node("e", none),
                node("flip", flipped(0, Some(1))),
            ],
            Some(2),
        );
        let Pass::Lay { mask, .. } = &plan.passes[1] else {
            panic!("a transform");
        };
        assert_eq!(*mask, Reads::Pass(0));
    }

    #[test]
    fn the_shader_of_a_mask_has_the_edge_of_the_windows_once_and_the_numbers_of_the_maths() {
        let shader = mask_shader();
        assert_eq!(shader.matches("fn window_feather(").count(), 1);
        assert_eq!(shader.matches("fn window_distance(").count(), 1);
        // Nothing of the windows that reads the parameters of a grade.
        assert!(!shader.contains("fn window_outline(") && !shader.contains("param("));
        assert_eq!(shader.matches("@vertex").count(), 1);
        assert_eq!(shader.matches("@fragment").count(), 1);
        // The numbers of the maths, which the shader is held to.
        let steps = format!("const MASK_ELLIPSE_STEPS: u32 = {}u;", mask::ELLIPSE_STEPS);
        assert!(shader.contains(&steps), "{steps}");
        assert!(shader.contains("const MASK_FAR: f32 = 1.0e30;"));
        assert_eq!(mask::FAR, 1.0e30);
        // The shader that lays reads its effect mask, and no side of a polygon.
        let lays = super::shader();
        assert_eq!(lays.matches("var effect_mask: texture_2d<f32>;").count(), 1);
        assert!(!lays.contains("sides"));
    }

    #[test]
    fn the_uniform_of_a_pass_has_its_map_and_its_settings_as_the_maths_takes_them() {
        let settings = Merged {
            apply: Composite::Screen,
            // Read in the Normal mode alone: Additive in any other.
            subtractive_additive: 0.25,
            alpha_gain: 0.5,
            burn_in: 0.75,
            blend: 0.125,
            size: 0.25,
            ..merge()
        };
        let plan = planned(
            vec![
                node("in", FusedKind::MediaIn),
                node(
                    "over",
                    FusedKind::Merge {
                        background: None,
                        foreground: Some(0),
                        mask: None,
                        settings,
                    },
                ),
            ],
            Some(1),
        );
        let Pass::Lay { uniform, .. } = &plan.passes[0] else {
            panic!("a merge");
        };
        // The foreground a quarter as large: a pixel reads four times as far from the middle,
        // at eight points across and down.
        assert_eq!(uniform[..4], [4.0, 0.0, -24.0, 0.0]);
        assert_eq!(uniform[4..8], [0.0, 4.0, -12.0, 0.0]);
        assert_eq!(uniform[8..12], [0.25, 8.0, 23.0, 0.0]);
        assert_eq!(uniform[12..16], [1.0, 0.5, 0.75, 0.125]);
        assert_eq!(uniform[16..19], REC709_LUMA);
        assert_eq!(u64::try_from(uniform.len() * 4).unwrap(), PARAMS_SIZE);
        // As it lies, by the settings as they are held.
        let resting = merge_uniform(SIZE, &Settings::default(), REC709_LUMA);
        assert_eq!(
            resting[..12],
            [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0]
        );
        assert_eq!(resting[12..16], [1.0, 1.0, 0.0, 1.0]);
    }

    #[test]
    fn the_shader_of_a_pass_has_the_maths_of_the_modes_once_by_names_of_its_own() {
        let shader = shader();
        assert_eq!(shader.matches("fn layer_composite(").count(), 1);
        assert_eq!(
            shader
                .matches("var<private> LAYER_LUMA: vec3<f32>;")
                .count(),
            1
        );
        assert!(!shader.contains("grade_") && !shader.contains("GRADE_"));
        assert_eq!(shader.matches("fn fusion_merged(").count(), 1);
        assert_eq!(shader.matches("@fragment").count(), 1);
    }
}
