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
//! # The picture of one node
//!
//! A viewer of a node shows the picture of that node alone, as Fusion's viewers do. Nothing
//! is drawn otherwise for it: the composition is made for the node
//! ([`columbina_model::fusion::Composition::at_node`]), so that its `output` is the node and
//! its nodes are those that the node depends on, and it is the [`Fusion`] of a layer that is
//! not moved ([`crate::Transform::NONE`]), whole ([`crate::Crop::NONE`]), opaque, laid as it
//! is, with no grade, alone in its scene. `plan` then makes of it the passes that it makes of
//! those nodes within the whole composition, so that the picture is the texture of the node
//! as the whole composition draws it; and the picture is fitted whole to the canvas, in its
//! middle, where [`crate::geometry::fitted`] says, over the black of the canvas: what is
//! transparent in it shows black. A mask node shows white premultiplied by its mask, which
//! over black is the mask itself in R, G and B.
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
//! - a Blur (`Pass::Blur`) and a Mosaic (`Pass::Mosaic`) are each a picture of the
//!   composition made through textures of their own, in three passes of the card, or five:
//!   see Blur and Mosaic below;
//! - a Delta Keyer (`Pass::Key`) is a picture of the composition made by one pass of
//!   `fusion_keyer.wgsl`, and through textures of its own before it where it reads the
//!   pixels about each one: see The Delta Keyer below;
//! - a Tracker is the passes of a Transform and of a Merge (`Pass::Lay`), one or two, or
//!   none: see The Tracker below;
//! - a Planar Tracker is one pass of `fusion.wgsl` (`Pass::Lay`) whose map may be projective,
//!   or none: see The Planar Tracker below;
//! - a node that changes nothing is no pass: its picture is that of its input. So is a
//!   Transform as it is at first, a Merge whose foreground gives no picture or whose Blend is
//!   0 (its background), and a Merge as it is at first over no background (its foreground):
//!   each where it has no mask, and Masks below tells of those that have one. A node whose
//!   kind is not known here hands on the picture of its first input.
//!
//! A composition whose `MediaOut` shows its `MediaIn`, as the one that a clip has at first
//! does, has no pass at all: its layer is drawn as that of a clip without a composition, to
//! the bit. One of `n` nodes, none of them a Blur, a Mosaic or a Delta Keyer, costs `n`
//! passes at the most, and one more for its source; and a composition without a Blur,
//! without a Mosaic and without a Delta Keyer is drawn by the passes that it was drawn by
//! before there were any, to the bit.
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
//!
//! # Blur and Mosaic
//!
//! What a Blur and a Mosaic are is told with the model ([`columbina_model::fusion`], Blur
//! and Mosaic). Each makes one picture of the composition, of the size of the others, and
//! makes it through textures of other sizes, which are of no node: those that the passes of
//! a frame share, drawn into and read at once (the spares of the neighbourhoods).
//!
//! **A Blur** is blurred by the filters of the neighbourhoods of grades
//! ([`crate::neighbourhood`]), which are not written a second time: its picture goes through
//! the two passes of `neighbourhood.wgsl`, across and down, a Gaussian of the Blur Size as its
//! standard deviation or a box of it as its radius, the pixels beyond the edges being those
//! at the edges; into a texture that is smaller than the picture where the filter is wide
//! ([`crate::neighbourhood::reduction`]). A third pass, of `fusion_blur.wgsl`, reads that
//! between its texels, as an operation of colour reads a neighbourhood
//! ([`crate::neighbourhood::sample`]), and mixes it with the picture as it came in:
//! `input + (blurred - input) * blend * mask`, the mask 1 where the node has none. Where
//! `blend * mask` is 0 the pixel is that of the picture as it came in, to the bit, and where
//! it is 1 that of the blurred picture. The processor does the same
//! ([`crate::neighbourhood::filtered`]), and the card is held to it as it is for a grade.
//!
//! The Blur Size is in pixels of a picture [`crate::color::REFERENCE_HEIGHT`] high, as the
//! reach of a [`Filter`] is: as many pixels of the picture as it is drawn as that is higher
//! or lower than 1080, so that a number is the same share of the picture at whatever size
//! it is drawn, and a preview at half the size looks as the frame does.
//!
//! A Blur is no pass, and is its input, where both its Blur Sizes are 0 (with Lock X/Y, its
//! `size.x`) or its Blend is 0, whatever its mask; and one of no picture is no picture.
//!
//! **A Mosaic** is drawn as [`crate::mosaic`] lays it out, and is held to it
//! (`fusion_mosaic.wgsl`): the colours of the cells are made in two passes, across, into a
//! texture of a texel for each cell across and each row of the picture, and down, into one
//! of a texel for each cell; and a third pass draws each pixel from the texel of its cell,
//! mixed with the picture as it came in by the key at the pixel, which is the effect mask of
//! the node. With Whole Cells Only, where the node has a mask, two more passes make the least
//! of the key over each cell the same way, and a cell is shown where that is 1. The grid goes
//! to the card in whole numbers of steps ([`crate::mosaic::Grid`]), and the card counts the
//! cell of a pixel in whole numbers, so that the two put every pixel in the same cell.
//!
//! The textures are of half floats. Which pixels are of which cell is exact; a pixel under a
//! key of 0 is the picture as it came in, to the bit; which cells are whole is exact, the
//! least of half floats being one of them; and the colour of a cell is the mean that the
//! processor makes but for the rounding of the sum in floats of 32 bits and of two writes of
//! half floats, each to the nearest or towards 0 as the card has it: within three thousandths
//! of a colour from 0 to 1.
//!
//! The grid is said in pixels of the picture of the composition. Where the pictures are
//! drawn smaller, the grid is that of the settings for a frame of the size that they are
//! drawn at, with the offsets made as many times smaller: the same cells cover the same
//! parts of the picture, their sides at the nearest 256th of a pixel as it is drawn, and no
//! cell is less than a pixel as it is drawn. At the size of the composition the grid is that
//! of the processor exactly.
//!
//! A Mosaic is no pass, and is its input, where its cells are a pixel wide and a pixel high,
//! whatever its mask; and one of no picture is no picture.
//!
//! # The Delta Keyer
//!
//! What a Delta Keyer is is told with the model ([`columbina_model::fusion`], The Delta
//! Keyer), and its mathematics is [`crate::keyer`], which the shader is held to
//! (`fusion_keyer.wgsl`): what the node makes of a pixel is [`crate::keyer::node_pixel`], and
//! of a picture [`crate::keyer::node_picture`].
//!
//! The pictures are keyed **as they are**, in the working space of the timeline: where the
//! project is not managed they are coded with their gamma, and nothing here makes them of
//! linear light, which the mathematics of the keyer speaks of. No colour is converted in a
//! pass of a keyer.
//!
//! A keyer that reads no pixel about another is one pass, which reads its picture, its
//! garbage matte, its solid matte and its effect mask at the pixel: more pictures than a
//! pass that lays reads, so that its passes bind a layout of their own, of a uniform and six
//! pictures. Its uniform is of the size of the others, and lies with them: the four rows of
//! [`crate::keyer::Keyer::uniform`], what the pass reads in the place that those leave free,
//! and a fifth row of what the pass makes, the view, and how a filtered texture is read.
//!
//! The controls that read the pixels about each one are done with the filters of the
//! neighbourhoods of grades ([`crate::neighbourhood`]), as a Blur is, through textures that
//! the passes of a frame share:
//!
//! - with a Pre-Blur, the picture goes through the two passes of a Gaussian first, and the key
//!   is taken of what they made, read between its texels (two passes more);
//! - with an Erode/Dilate, a Matte Blur or a Clean, the matte of the key is drawn alone into
//!   a texture (one pass more), white premultiplied by it; and each of the three, in that
//!   order, filters that texture by two passes and draws the matte anew from it and from what
//!   the filter made (three passes more each: a box for Erode/Dilate, a Gaussian for the
//!   Matte Blur, and the Gaussian of the Cleans of a qualifier). The pass that makes the
//!   picture then reads the matte that the last of them drew.
//!
//! So a keyer costs from one pass to thirteen. A control at 0 costs none; and a keyer that
//! keys nothing (a Gain of 0, or a screen colour that keys have left with no channel above
//! its other two) reads no pixel about another, its matte being one level all over, which
//! those controls leave as it is. The lengths are in pixels of a picture
//! [`crate::color::REFERENCE_HEIGHT`] high, as a Blur Size is, so that a keyer drawn smaller
//! is the same share of its picture.
//!
//! The matte is kept in half floats between two of those controls, and the card writes a
//! half float to the nearest or towards 0: a matte that went through them is within a step
//! of a half float of the one of the processor at each, but for the pixels where an
//! Erode/Dilate takes it through its edge, a twentieth of the box wide, where such a step of
//! its mean is twenty of the matte.
//!
//! A keyer is no pass, and is its input, where it changes nothing: its view is the final
//! result, it keys nothing, its thresholds leave a solid matte solid (the lower is below 1),
//! it has no garbage matte, and it suppresses no spill (its method is None, or its Spill
//! Suppression 0); whatever its solid matte, which has nothing to fill, and whatever its
//! effect mask. Any other is a pass: with a garbage matte it takes from the alpha, with a
//! spill method it changes the colours though it keys nothing, and a view of a matte is a
//! picture of its own. One of no picture is no picture, whatever its view.
//!
//! A `garbage_matte` or a `solid_matte` that names a node which gives no picture here is not
//! there, as a `mask` that does is no mask.
//!
//! # The Tracker
//!
//! What a Tracker is is told with the model ([`columbina_model::fusion`], The Tracker). The
//! model finds the motion of its patterns, and hands the node on as the numbers by which a
//! Transform puts its picture and a Merge its foreground: where the middle of the picture
//! that is moved is put, how large it is made about its middle and how far it is turned
//! there ([`columbina_model::fusion::Tracked`]). So nothing is drawn for a Tracker that is
//! not drawn for those two, by the same passes of `fusion.wgsl`, and no shader is its own:
//!
//! - with an Operation of None it is its `input`, and no pass;
//! - BG Only is the pass of a Transform of its `input`: held still, the picture that was
//!   tracked is read through the inverse of the motion. Where it lies as it is (at the
//!   reference frame, and where there is no motion to be had) it is its `input`, and no
//!   pass, whatever its mask;
//! - FG Only is the pass of a Transform of its `foreground`, over nothing;
//! - FG Over BG is the pass of a Merge as it is at first, the plain over, of its
//!   `foreground`, put so, on its `input`;
//! - BG Over FG is two: the pass of a Transform of its `foreground`, and that of a Merge as
//!   it is at first of its `input`, as it lies, on what the first made.
//!
//! Its effect mask limits it as it limits every node that hands a picture on, the picture
//! of its `input` being the first: `input + (own - input) * mask`. With BG Only and FG Over
//! BG that is the mask of the pass, as of a Transform and of a Merge. With FG Only the
//! picture that the node makes is not made of its `input`, so that the two are mixed by a
//! pass more, which lays nothing over the picture that was made and is limited to its
//! `input` as it came in; and with BG Over FG the pass that lays the `input` is limited to
//! it as it came in. Where its `input` gives no picture, what it makes is multiplied by the
//! mask, as a Merge over no background is.
//!
//! A turn that the model found is no whole number of quarter turns but by chance, and is
//! drawn by its sine and its cosine: a picture that is brought back by a quarter turn is
//! read within a rounding of the middles of its texels.
//!
//! # The Planar Tracker
//!
//! What a Planar Tracker is is told with the model ([`columbina_model::fusion`], The Planar
//! Tracker). The model hands it on as its operation and a matrix of shares of the picture from
//! its top left corner, where each point of the picture that is moved is put
//! ([`columbina_model::fusion::PlanarTracked`]), as the stabilizer hands on its own; and it is
//! drawn as the stabilizer is, by one pass of `fusion.wgsl` that reads the picture through the
//! inverse of that matrix made a map of the pixels that the pictures are drawn at (`steadied`),
//! projective where it is, with no shader of its own:
//!
//! - with an operation of None it is its `input`, and no pass;
//! - Steady is the pass of a Transform of its `input`, laid over nothing: a pixel that the
//!   picture is not moved over is transparent. Where the matrix moves nothing (at the reference
//!   frame, and with no path) or cannot be undone, it is its `input`, and no pass, whatever its
//!   mask;
//! - Corner Pin is the pass of a Merge as it is at first, the plain over, of its `foreground`
//!   put so, on its `input`. With no `foreground` it is its `input`, and no pass.
//!
//! Its effect mask limits it as it limits every node that hands a picture on, the picture of
//! its `input` being the first: that is the mask of the pass, as of a Transform and of a
//! Merge. Where a pixel is made of several points, they lie as the affine part of the map puts
//! them, as of the stabilizer.
//!
//! # The stabilizer
//!
//! The stabilizer of a clip ([`columbina_model::stabilize`]) is given in the composition of its
//! layer as a node that the model puts after its last node, between the node that the
//! `MediaOut` read and the `MediaOut` ([`FusedKind::Stabilize`], see
//! [`columbina_model::fusion::Fused::stabilized`]): a clip that is stabilized and has no
//! composition of its own has the one that a clip has at first. So it is done to what comes out
//! of the composition, and before the grade of the layer. It is the pass of a Transform of that
//! picture (of the source, drawn alone, where the composition is the source alone), laid over
//! nothing, by the inverse of its matrix made a map of the pixels that the pictures are drawn
//! at (`steadied`): so a pixel that the picture is not moved over is transparent, and the
//! picture is cut at the edges of the frame, as a Transform cuts it. The map of a Similarity or
//! a Translation is affine, and is drawn as the map of a Transform is, to the bit where it is
//! that map; that of Perspective may be projective, and the place that the affine part gives is
//! then divided by the third row of the map in the shader (`row0.w` and `row1.w` of its
//! uniform, which are 0 of every other pass). Where a pixel is made of several points, they lie
//! as the affine part puts them. A matrix that moves no point is no pass, and so is one that
//! cannot be undone: the node is then its input.

use columbina_model::fusion::{
    self as model, Blurred, DeltaKeyed, Fused, FusedKind, Merged, Mosaicked, PlanarTracked,
    Tracked, Transformed,
};

use crate::color::Filter;
use crate::geometry::{fit, raster_scale, samples, Affine};
use crate::gpu::Gpu;
use crate::grade::layout;
use crate::keyer::{self, Finesse, Keyer, View};
use crate::mask::{self, Common, FillMethod, PaintMode, Placed, Shape};
use crate::merge::{Operator, Settings};
use crate::mosaic::{self, Grid};
use crate::neighbourhood;
use crate::Source;

/// The shader of a pass that lays a foreground over a background.
const SHADER: &str = include_str!("fusion.wgsl");

/// The shader of a pass that draws a mask, or a Background times its effect mask.
const MASK_SHADER: &str = include_str!("fusion_mask.wgsl");

/// The shader of the pass that mixes a blurred picture with the picture as it came in.
const BLUR_SHADER: &str = include_str!("fusion_blur.wgsl");

/// The shader of the passes of a Mosaic.
const MOSAIC_SHADER: &str = include_str!("fusion_mosaic.wgsl");

/// The shader of the passes of a Delta Keyer.
const KEYER_SHADER: &str = include_str!("fusion_keyer.wgsl");

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

/// What the uniform of `fusion_mosaic.wgsl` has as what the pass makes (`kind.x`) for the
/// pass that makes the cells down, from what the pass across made, which has 0.
const CELLS_DOWN: f32 = 1.0;

/// What the uniform of `fusion_mosaic.wgsl` has as what the pass makes (`kind.x`) for the
/// pass that draws the picture from the colours of its cells.
const CELLS_SHOWN: f32 = 2.0;

/// The place in the uniform of `fusion_keyer.wgsl` of what a pass of a keyer reads (`key.w`),
/// a sum of the `READS_…` below: the one that the four rows of
/// [`crate::keyer::Keyer::uniform`] leave free.
const READS_AT: usize = 7;

/// The place in the uniform of `fusion_keyer.wgsl` of its fifth row (`made`): what the pass
/// makes, the view of the node, and the texels of a filtered texture to a pixel of the
/// picture, across and down.
const MADE_AT: usize = 16;

/// A pass of a keyer reads a garbage matte.
const READS_GARBAGE: f32 = 1.0;

/// It reads the garbage matte the other way round.
const READS_GARBAGE_INVERTED: f32 = 2.0;

/// It reads a solid matte.
const READS_SOLID: f32 = 4.0;

/// It reads an effect mask.
const READS_MASK: f32 = 8.0;

/// It reads the matte that the passes before it drew.
const READS_MATTE: f32 = 16.0;

/// It keys the picture blurred by a Pre-Blur.
const READS_BLURRED: f32 = 32.0;

/// What the uniform of `fusion_keyer.wgsl` has as what the pass makes (`made.x`) for the pass
/// that draws the matte of the key alone; the pass that makes the picture has 0.
const KEY_MATTE: f32 = 1.0;

/// The same for the pass that draws the matte anew through Erode/Dilate.
const KEY_ERODED: f32 = 2.0;

/// The same for the pass that draws the matte anew through the Matte Blur.
const KEY_SOFTENED: f32 = 3.0;

/// The same for the pass that draws the matte anew through Clean Foreground and Clean
/// Background.
const KEY_CLEANED: f32 = 4.0;

/// How the pass of a Mosaic that draws the picture is keyed (`kind.z` of the uniform of
/// `fusion_mosaic.wgsl`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Keyed {
    /// By nothing: every cell is shown.
    Whole,
    /// By the alpha, at each pixel, of the picture that is the effect mask of the node.
    Pixels,
    /// By the least of the key over each cell, a texel each: a cell is shown where it is 1.
    Cells,
}

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
            | FusedKind::Blur { .. }
            | FusedKind::Mosaic { .. }
            | FusedKind::DeltaKeyer { .. }
            | FusedKind::Tracker { .. }
            | FusedKind::PlanarTracker { .. }
            | FusedKind::Stabilize { .. }
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
    /// A Blur: its picture through the two passes of a filter of the neighbourhoods
    /// (`neighbourhood.wgsl`), into textures of their own, and that mixed with the picture as
    /// it came in by a pass of `fusion_blur.wgsl`: three passes of the card, which make one
    /// picture of the composition.
    Blur {
        /// The picture that is blurred: never nothing.
        input: Reads,
        /// The picture whose alpha is the effect mask of the node; nothing where it has none.
        mask: Reads,
        /// The two passes of its filter, across and down, over a picture of the size that
        /// the pictures are drawn at.
        filter: [neighbourhood::Pass; 2],
        /// The uniform of `fusion_blur.wgsl`.
        uniform: [f32; PARAMS],
    },
    /// A Mosaic: the colours of its cells made in two passes of `fusion_mosaic.wgsl`, across
    /// and down, into textures of their own; with `whole`, the least of its key over each
    /// cell made in two more; and the picture drawn from them by one more, which makes one
    /// picture of the composition.
    Mosaic {
        /// The picture that is laid out in cells: never nothing.
        input: Reads,
        /// The picture whose alpha is the key, the effect mask of the node; nothing where it
        /// has none, and every cell is shown.
        key: Reads,
        /// The grid, over a picture of the size that the pictures are drawn at.
        grid: Grid,
        /// Whole Cells Only, of a node that has a key: a cell is shown only where the key is
        /// 1 over all of it. Never where there is no key.
        whole: bool,
    },
    /// A Delta Keyer: its picture keyed by a pass of `fusion_keyer.wgsl`, which makes one
    /// picture of the composition; and before it, where the keyer reads the pixels about
    /// each one, the passes of filters of the neighbourhoods and those that draw its matte,
    /// into textures of their own ([`Keying`]).
    Key {
        /// The picture that is keyed: never nothing.
        input: Reads,
        /// The picture whose alpha is the garbage matte; nothing where there is none.
        garbage: Reads,
        /// The picture whose alpha is the solid matte; nothing where there is none.
        solid: Reads,
        /// The picture whose alpha is the effect mask of the node; nothing where it has none.
        mask: Reads,
        /// What its passes are drawn with.
        keying: Box<Keying>,
    },
}

/// A control of the matte of a keyer that reads the pixels about each one (Erode/Dilate, the
/// Matte Blur, or the Cleans): the two passes of its filter over the matte, across and down,
/// and the uniform of the pass of `fusion_keyer.wgsl` that draws the matte anew from the
/// matte and from what they made.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Stage {
    pub filter: [neighbourhood::Pass; 2],
    pub uniform: [f32; PARAMS],
}

/// What the passes of a Delta Keyer are drawn with, over a picture of the size that the
/// pictures are drawn at.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Keying {
    /// The two passes of the filter of its Pre-Blur over its picture; none where it has
    /// none, and the key is of the picture as it is.
    pub pre_blur: Option<[neighbourhood::Pass; 2]>,
    /// The controls of its matte that read the pixels about each one, in the order in which
    /// they are done; none where every one of them is at 0, and the matte is that of the key.
    pub stages: Vec<Stage>,
    /// The uniform of the pass that draws the matte of the key alone, which is drawn where
    /// there are stages.
    pub matte: [f32; PARAMS],
    /// The uniform of the pass that makes the picture.
    pub uniform: [f32; PARAMS],
}

impl Pass {
    /// The pictures that the pass reads: nothing in the places of those that it has not.
    fn reads(&self) -> [Reads; 4] {
        let none = Reads::Nothing;
        match self {
            Pass::Fill(_) => [none; 4],
            Pass::Lay {
                background,
                foreground,
                mask,
                ..
            } => [*background, *foreground, *mask, none],
            Pass::Mask { incoming, .. } => [*incoming, none, none, none],
            Pass::Blur { input, mask, .. } => [*input, *mask, none, none],
            Pass::Mosaic { input, key, .. } => [*input, *key, none, none],
            Pass::Key {
                input,
                garbage,
                solid,
                mask,
                ..
            } => [*input, *garbage, *solid, *mask],
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
            Pass::Blur {
                input,
                mask,
                filter,
                uniform,
            } => Pass::Blur {
                input: renumbered(input),
                mask: renumbered(mask),
                filter,
                uniform,
            },
            Pass::Mosaic {
                input,
                key,
                grid,
                whole,
            } => Pass::Mosaic {
                input: renumbered(input),
                key: renumbered(key),
                grid,
                whole,
            },
            Pass::Key {
                input,
                garbage,
                solid,
                mask,
                keying,
            } => Pass::Key {
                input: renumbered(input),
                garbage: renumbered(garbage),
                solid: renumbered(solid),
                mask: renumbered(mask),
                keying,
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

    /// Whether a pass of it is a Blur, which needs the pipeline of `fusion_blur.wgsl` and
    /// that of the filters of neighbourhoods.
    pub(crate) fn blurs(&self) -> bool {
        self.passes
            .iter()
            .any(|pass| matches!(pass, Pass::Blur { .. }))
    }

    /// Whether a pass of it is a Mosaic, which needs the pipeline of `fusion_mosaic.wgsl`.
    pub(crate) fn mosaics(&self) -> bool {
        self.passes
            .iter()
            .any(|pass| matches!(pass, Pass::Mosaic { .. }))
    }

    /// Whether a pass of it is a Delta Keyer, which needs the pipeline of
    /// `fusion_keyer.wgsl`.
    pub(crate) fn keys(&self) -> bool {
        self.passes
            .iter()
            .any(|pass| matches!(pass, Pass::Key { .. }))
    }

    /// Whether a pass of it goes through a filter of the neighbourhoods, which needs their
    /// pipeline: a Blur, and a Delta Keyer that reads the pixels about each one.
    pub(crate) fn filters(&self) -> bool {
        self.passes.iter().any(|pass| match pass {
            Pass::Blur { .. } => true,
            Pass::Key { keying, .. } => keying.pre_blur.is_some() || !keying.stages.is_empty(),
            Pass::Fill(_) | Pass::Lay { .. } | Pass::Mask { .. } | Pass::Mosaic { .. } => false,
        })
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

/// The passes of `composition`, whose pictures are of `nominal` pixels, drawn `size` pixels
/// large; `luma` the weights of R, G and B in the luma of the working space, which some Apply
/// Modes read. `source` says whether the source of the clip gives a picture: a title that is
/// not seen gives none, and a `MediaIn` is then no picture.
pub(crate) fn plan(
    composition: &Fused,
    size: (u32, u32),
    nominal: (u32, u32),
    luma: [f32; 3],
    source: bool,
) -> Plan {
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
            FusedKind::Blur {
                input,
                mask,
                settings,
            } => blurred(read(*input), read(*mask), settings, size, &mut passes),
            FusedKind::Mosaic {
                input,
                mask,
                settings,
            } => tiled(
                read(*input),
                read(*mask),
                settings,
                (size, nominal),
                &mut passes,
            ),
            // A matte that names a node which gives no picture is not there, as a mask is
            // no mask.
            FusedKind::DeltaKeyer {
                input,
                garbage_matte,
                solid_matte,
                mask,
                settings,
            } => keyed(
                [
                    read(*input),
                    read(*garbage_matte),
                    read(*solid_matte),
                    read(*mask),
                ],
                settings,
                size,
                &mut passes,
            ),
            tracker @ FusedKind::Tracker { .. } => tracked(tracker, read, size, luma, &mut passes),
            node @ FusedKind::PlanarTracker { .. } => planar(node, read, size, luma, &mut passes),
            kind @ FusedKind::Stabilize { .. } => stabilized(kind, read, size, luma, &mut passes),
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

/// The picture of the Tracker `node`, `read` giving the picture that an input of it reads,
/// drawn `size` pixels large: what its Operation and its Merge make of its `input` and of
/// its `foreground`, the one that is moved put as the model says, and that limited by its
/// effect mask to its `input` as it came in. None of a node that is no Tracker.
///
/// It is drawn by the passes of a Transform and of a Merge, and is no pass where those are
/// none: see The Tracker of the [module](self).
fn tracked(
    node: &FusedKind,
    read: impl Fn(Option<usize>) -> Reads,
    size: (u32, u32),
    luma: [f32; 3],
    passes: &mut Vec<Pass>,
) -> Reads {
    let FusedKind::Tracker {
        input,
        foreground,
        mask,
        settings,
    } = node
    else {
        return Reads::Nothing;
    };
    let (input, foreground, limit) = (read(*input), read(*foreground), read(*mask));
    if settings.operation == model::TrackerOperation::None {
        return input;
    }
    // The picture that is moved, as a Transform puts its picture and as a Merge as it is at
    // first puts its foreground; and a Merge as it is at first, which moves nothing.
    let Tracked {
        center,
        size: scale,
        angle,
        ..
    } = *settings;
    let put = Transformed {
        center,
        size: scale,
        angle,
        flip_horizontal: false,
        flip_vertical: false,
    };
    let plain = model::Merge::default().at(0.0);
    let laid = Merged {
        center,
        size: scale,
        angle,
        ..plain
    };
    // The foreground as it follows the patterns, alone.
    let moved =
        |passes: &mut Vec<Pass>| transformed(foreground, Reads::Nothing, &put, size, luma, passes);
    match settings.merge {
        model::TrackerMerge::BgOnly => transformed(input, limit, &put, size, luma, passes),
        model::TrackerMerge::FgOverBg => {
            merged(input, foreground, limit, &laid, size, luma, passes)
        }
        model::TrackerMerge::FgOnly => {
            // With no mask, and over no `input`, what is made is the foreground as it
            // follows, laid over nothing by a Merge as it is at first, which its mask limits
            // to nothing.
            if input == Reads::Nothing || limit == Reads::Nothing {
                return merged(Reads::Nothing, foreground, limit, &laid, size, luma, passes);
            }
            let own = moved(passes);
            if own == input {
                return input;
            }
            mixed(input, own, limit, &NOWHERE, luma, passes)
        }
        model::TrackerMerge::BgOverFg => {
            let under = moved(passes);
            if input == Reads::Nothing {
                // Nothing over the foreground as it follows: that, limited to nothing.
                return merged(Reads::Nothing, under, limit, &plain, size, luma, passes);
            }
            if limit == Reads::Nothing {
                return merged(under, input, limit, &plain, size, luma, passes);
            }
            if under == Reads::Nothing {
                return input;
            }
            let as_it_lies = reading(size, [0.5, 0.5], 1.0, 0.0, (false, false));
            mixed(input, under, limit, &as_it_lies, luma, passes)
        }
    }
}

/// The picture of the stabilizer of a clip, `node`, `read` giving the picture that its input
/// reads, drawn `size` pixels large: what comes out of the composition put where the matrix of
/// the node says, laid over nothing as a Transform lays its picture, by a pass whose map may be
/// projective (see The stabilizer of the [module](self)). None where its input gives no
/// picture; its input where the matrix moves no point, or cannot be undone. None of a node
/// that is not the stabilizer.
fn stabilized(
    node: &FusedKind,
    read: impl Fn(Option<usize>) -> Reads,
    size: (u32, u32),
    luma: [f32; 3],
    passes: &mut Vec<Pass>,
) -> Reads {
    let FusedKind::Stabilize { input, matrix } = node else {
        return Reads::Nothing;
    };
    let input = read(*input);
    if input == Reads::Nothing || !columbina_model::stabilize::moves(matrix) {
        return input;
    }
    let Some(params) = projective(matrix, size, &Settings::default(), luma) else {
        return input;
    };
    drawn(
        passes,
        Pass::Lay {
            background: Reads::Nothing,
            foreground: input,
            mask: Reads::Nothing,
            uniform: params,
        },
    )
}

/// The uniform of `fusion.wgsl` of a pass that lays a picture put where `matrix` puts each of
/// its points, in shares of the picture from its top left corner, both drawn `size` pixels
/// large, merged by `settings`: the inverse of the matrix made a map of pixels ([`steadied`]),
/// its affine part and the two numbers of its third row by which a place is divided. None where
/// the matrix cannot be undone.
fn projective(
    matrix: &[[f64; 3]; 3],
    size: (u32, u32),
    settings: &Settings,
    luma: [f32; 3],
) -> Option<[f32; PARAMS]> {
    let (map, scale) = steadied(matrix, size)?;
    let mut params = uniform(&map.affine, scale, settings, luma);
    #[allow(
        clippy::cast_possible_truncation,
        reason = "a map, as the card takes it"
    )]
    let (across, down) = (map.across as f32, map.down as f32);
    params[PROJECTIVE_ACROSS_AT] = across;
    params[PROJECTIVE_DOWN_AT] = down;
    Some(params)
}

/// The picture of the Planar Tracker `node`, `read` giving the picture that an input of it
/// reads, drawn `size` pixels large: its `input` held still (Steady), or its `foreground` laid
/// on the plane over its `input` (Corner Pin), the picture that is moved put where the matrix
/// of the model puts it ([`projective`]); and that limited by its effect mask to its `input` as
/// it came in. None of a node that is no Planar Tracker.
///
/// It is one pass, or none: see The Planar Tracker of the [module](self).
fn planar(
    node: &FusedKind,
    read: impl Fn(Option<usize>) -> Reads,
    size: (u32, u32),
    luma: [f32; 3],
    passes: &mut Vec<Pass>,
) -> Reads {
    let FusedKind::PlanarTracker {
        input,
        foreground,
        mask,
        settings,
    } = node
    else {
        return Reads::Nothing;
    };
    let (input, foreground, limit) = (read(*input), read(*foreground), read(*mask));
    let PlanarTracked { operation, matrix } = *settings;
    let moves = columbina_model::stabilize::moves(&matrix);
    match operation {
        model::PlanarOperation::None => input,
        model::PlanarOperation::Steady => {
            if input == Reads::Nothing || !moves {
                return input;
            }
            let Some(params) = projective(&matrix, size, &Settings::default(), luma) else {
                return input;
            };
            drawn(
                passes,
                Pass::Lay {
                    background: Reads::Nothing,
                    foreground: input,
                    mask: limit,
                    uniform: limited(params, limit, LIMITED_TO_FOREGROUND),
                },
            )
        }
        model::PlanarOperation::CornerPin => {
            if foreground == Reads::Nothing {
                return input;
            }
            // Laid as it lies over nothing, with no mask, it is its foreground.
            if !moves && input == Reads::Nothing && limit == Reads::Nothing {
                return foreground;
            }
            let plain = held(&model::Merge::default().at(0.0));
            let Some(params) = projective(&matrix, size, &plain, luma) else {
                return input;
            };
            drawn(
                passes,
                Pass::Lay {
                    background: input,
                    foreground,
                    mask: limit,
                    uniform: limited(params, limit, LIMITED_TO_BACKGROUND),
                },
            )
        }
    }
}

/// How a pixel of the picture that the stabilizer draws reads the picture that comes into it,
/// both drawn `size` pixels large, in pixels from their top left corners: the affine part of the
/// map, and the two numbers of its third row by which the place that the affine part gives is
/// divided (`across · x + down · y + 1`; both 0 of an affine map).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Steadied {
    /// The affine part.
    pub affine: Affine,
    /// What the divisor gains a pixel across.
    pub across: f64,
    /// What the divisor gains a pixel down.
    pub down: f64,
}

/// The place in the uniform of `fusion.wgsl` of what the divisor of a projective map gains a
/// pixel across (`row0.w`): 0 of every pass but those of the stabilizer and of a Planar
/// Tracker whose map is projective.
const PROJECTIVE_ACROSS_AT: usize = 3;

/// The place of what it gains a pixel down (`row1.w`).
const PROJECTIVE_DOWN_AT: usize = 7;

/// How a picture that the stabilizer puts by `matrix`, in shares of the picture, is read where
/// it is drawn `size` pixels large ([`Steadied`]), and how many pixels of the picture drawn are
/// a pixel of the picture read, about its middle: the inverse of the matrix made a map of
/// pixels. None where the matrix flattens the picture, or is not of finite numbers.
pub(crate) fn steadied(matrix: &[[f64; 3]; 3], size: (u32, u32)) -> Option<(Steadied, f64)> {
    let (width, height) = (f64::from(size.0), f64::from(size.1));
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    // A share is so many pixels: the matrix of pixels is `S · matrix · S⁻¹`.
    let scale = [width, height, 1.0];
    let mut forward = [[0.0; 3]; 3];
    for (row, (cells, own)) in forward.iter_mut().zip(matrix).enumerate() {
        for (column, cell) in cells.iter_mut().enumerate() {
            *cell = own[column] * scale[row] / scale[column];
        }
    }
    let back = inverse(&forward)?;
    // The divisor of the map is made 1 at the corner.
    let corner = back[2][2];
    if !corner.is_finite() || corner.abs() < 1e-12 {
        return None;
    }
    let map = back.map(|row| row.map(|cell| cell / corner));
    // How large the picture is drawn: the square root of the area that the affine part of the
    // matrix gives a pixel.
    let area = forward[0][0] * forward[1][1] - forward[0][1] * forward[1][0];
    let grown = area.abs().sqrt();
    if !(grown.is_finite() && grown > 0.0) || map.iter().flatten().any(|cell| !cell.is_finite()) {
        return None;
    }
    let affine = Affine {
        a: map[0][0],
        b: map[0][1],
        c: map[1][0],
        d: map[1][1],
        tx: map[0][2],
        ty: map[1][2],
    };
    Some((
        Steadied {
            affine,
            across: map[2][0],
            down: map[2][1],
        },
        grown,
    ))
}

/// The inverse of a 3×3 matrix, or none where it is singular.
fn inverse(m: &[[f64; 3]; 3]) -> Option<[[f64; 3]; 3]> {
    let minor =
        |r0: usize, r1: usize, c0: usize, c1: usize| m[r0][c0] * m[r1][c1] - m[r0][c1] * m[r1][c0];
    let det =
        m[0][0] * minor(1, 2, 1, 2) - m[0][1] * minor(1, 2, 0, 2) + m[0][2] * minor(1, 2, 0, 1);
    if !det.is_finite() || det.abs() < 1e-300 {
        return None;
    }
    let adjugate = [
        [minor(1, 2, 1, 2), -minor(0, 2, 1, 2), minor(0, 1, 1, 2)],
        [-minor(1, 2, 0, 2), minor(0, 2, 0, 2), -minor(0, 1, 0, 2)],
        [minor(1, 2, 0, 1), -minor(0, 2, 0, 1), minor(0, 1, 0, 1)],
    ];
    Some(adjugate.map(|row| row.map(|cell| cell / det)))
}

/// The picture `first` laid by `map` over the picture `under`, as a Merge as it is at first
/// lays a foreground, and that limited by the effect mask `limit` to `first` as it came in:
/// `first + (own - first) * mask`, `own` being what is laid. Neither `first` nor `limit` is
/// nothing.
///
/// It is the pass by which a Tracker with a mask mixes what it makes with the picture of its
/// `input`, where what it makes is not that picture moved: with a map by which nothing of
/// `first` is laid ([`NOWHERE`]), `own` is `under` as it is, to the bit, which is nothing
/// where `under` is; and with one by which `first` lies as it is, `own` is `first` over
/// `under`.
fn mixed(
    first: Reads,
    under: Reads,
    limit: Reads,
    map: &Affine,
    luma: [f32; 3],
    passes: &mut Vec<Pass>,
) -> Reads {
    drawn(
        passes,
        Pass::Lay {
            background: under,
            foreground: first,
            mask: limit,
            uniform: limited(
                uniform(map, 1.0, &Settings::default(), luma),
                limit,
                LIMITED_TO_FOREGROUND,
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

/// A number of a setting as the card takes it, where one below 0 is none: 0 where it is no
/// number.
#[allow(
    clippy::cast_possible_truncation,
    reason = "settings of nodes, as the card takes them"
)]
fn positive(value: f64) -> f32 {
    if value > 0.0 {
        value as f32
    } else {
        0.0
    }
}

/// How many pixels of a picture drawn `size` pixels large make a pixel of a picture 1080
/// pixels high ([`crate::color::REFERENCE_HEIGHT`]): what the reach of the filter of a Blur
/// is multiplied by, as that of a neighbourhood of a stage over a canvas of that size is.
fn units(size: (u32, u32)) -> f32 {
    let height = f64::from(size.1);
    neighbourhood::units(height, height, height)
}

/// The picture of a Blur of `input`, limited by the effect mask `limit` (nothing: it has
/// none), drawn `size` pixels large: none where it is given none; that of its input where
/// it blurs nothing (both its Blur Sizes are 0) or its Blend is 0, whatever its mask; and
/// else a pass, which blurs it by the filter of the node and mixes that with the picture as
/// it came in.
fn blurred(
    input: Reads,
    limit: Reads,
    settings: &Blurred,
    size: (u32, u32),
    passes: &mut Vec<Pass>,
) -> Reads {
    if input == Reads::Nothing {
        return Reads::Nothing;
    }
    let reach = settings.size.map(positive);
    let blend = positive(settings.blend).min(1.0);
    if blend <= 0.0 || (reach[0] <= 0.0 && reach[1] <= 0.0) {
        return input;
    }
    let taken = match settings.filter {
        model::BlurFilter::Gaussian => Filter::Gaussian { sigma: reach },
        model::BlurFilter::Box => Filter::Box { radius: reach },
    };
    let filter = neighbourhood::passes(taken, units(size), size);
    let limited = limit != Reads::Nothing;
    let uniform = blur_uniform(blend, limited, filter[1].target, size);
    drawn(
        passes,
        Pass::Blur {
            input,
            mask: limit,
            filter,
            uniform,
        },
    )
}

/// The uniform of `fusion_blur.wgsl`: the Blend, within 0 and 1; whether the node has an
/// effect mask; and how many texels of the blurred picture, which has `filtered` of them,
/// lie to a pixel of the picture that comes out, of `size` pixels, across and down.
#[allow(
    clippy::cast_precision_loss,
    reason = "sizes of textures, which a float holds"
)]
fn blur_uniform(
    blend: f32,
    limited: bool,
    filtered: (u32, u32),
    size: (u32, u32),
) -> [f32; PARAMS] {
    let mut uniform = [0.0; PARAMS];
    uniform[0] = blend;
    uniform[1] = f32::from(limited);
    uniform[4] = filtered.0 as f32 / size.0.max(1) as f32;
    uniform[5] = filtered.1 as f32 / size.1.max(1) as f32;
    uniform
}

/// The picture of a Mosaic of `input`, keyed by the effect mask `key` (nothing: it has
/// none, and every cell is shown), drawn at the first of `sizes`, the second being the size
/// of the composition: none where it is given none; that of its input where its cells are a
/// pixel wide and a pixel high, whatever its key; and else a pass.
fn tiled(
    input: Reads,
    key: Reads,
    settings: &Mosaicked,
    sizes: ((u32, u32), (u32, u32)),
    passes: &mut Vec<Pass>,
) -> Reads {
    if input == Reads::Nothing {
        return Reads::Nothing;
    }
    let (size, nominal) = sizes;
    let grid = Grid::new(size.0, size.1, &scaled(settings, size, nominal));
    if grid.size == [mosaic::STEPS; 2] {
        return input;
    }
    let whole = settings.whole_cells_only && key != Reads::Nothing;
    drawn(
        passes,
        Pass::Mosaic {
            input,
            key,
            grid,
            whole,
        },
    )
}

/// The settings of a Mosaic as the maths takes them for a picture drawn `size` pixels large,
/// of the `nominal` pixels of the composition: its offsets, which are pixels of the
/// composition, made pixels of the picture as it is drawn, so that the grid lies over the
/// same parts of the picture. At the size of the composition they are as they are.
#[allow(
    clippy::cast_possible_truncation,
    reason = "offsets in pixels, as the maths takes them"
)]
fn scaled(settings: &Mosaicked, size: (u32, u32), nominal: (u32, u32)) -> mosaic::Settings {
    let per = [
        f64::from(size.0) / f64::from(nominal.0.max(1)),
        f64::from(size.1) / f64::from(nominal.1.max(1)),
    ];
    let mut held = mosaic::Settings::from(settings);
    held.offset = [0, 1].map(|axis: usize| (settings.offset[axis] * per[axis]) as f32);
    held
}

/// The grid of a Mosaic as the uniform of `fusion_mosaic.wgsl` has it: the size of a cell
/// and the lead of the grid, in steps, and the size of the picture, in pixels. They are
/// whole numbers of at most 2^24, which a float keeps apart ([`mosaic::MOST_CELL`]).
#[allow(
    clippy::cast_precision_loss,
    reason = "whole numbers of at most 2^24, which a float keeps apart"
)]
fn grid_uniform(grid: &Grid) -> [f32; PARAMS] {
    let mut uniform = [0.0; PARAMS];
    uniform[4] = grid.size[0] as f32;
    uniform[5] = grid.size[1] as f32;
    uniform[6] = grid.lead[0] as f32;
    uniform[7] = grid.lead[1] as f32;
    uniform[8] = grid.frame[0] as f32;
    uniform[9] = grid.frame[1] as f32;
    uniform
}

/// The uniform of `fusion_mosaic.wgsl` for a pass that makes the cells of `grid`: across,
/// from the picture, or `down`, from what the pass across made; the means of the colours of
/// the cells, or, with `least`, the least of the key over each.
pub(crate) fn cells_uniform(grid: &Grid, down: bool, least: bool) -> [f32; PARAMS] {
    let mut uniform = grid_uniform(grid);
    if down {
        uniform[0] = CELLS_DOWN;
    }
    uniform[1] = f32::from(least);
    uniform
}

/// The uniform of `fusion_mosaic.wgsl` for the pass that draws the picture of a Mosaic of
/// `grid` from the colours of its cells, keyed as `keyed` says.
pub(crate) fn shown_uniform(grid: &Grid, keyed: Keyed) -> [f32; PARAMS] {
    let mut uniform = grid_uniform(grid);
    uniform[0] = CELLS_SHOWN;
    uniform[2] = match keyed {
        Keyed::Whole => 0.0,
        Keyed::Pixels => 1.0,
        Keyed::Cells => 2.0,
    };
    uniform
}

/// The picture of a Delta Keyer of the first of `pictures`, the others being the pictures
/// whose alphas are its garbage matte, its solid matte and its effect mask (nothing: it has
/// none), drawn `size` pixels large: none where it is given none; that of its input where it
/// changes nothing ([`keying`]); and else a pass.
fn keyed(
    pictures: [Reads; 4],
    settings: &DeltaKeyed,
    size: (u32, u32),
    passes: &mut Vec<Pass>,
) -> Reads {
    let [input, garbage, solid, mask] = pictures;
    if input == Reads::Nothing {
        return Reads::Nothing;
    }
    let linked = [garbage, solid, mask].map(|reads| reads != Reads::Nothing);
    match keying(settings, linked, size) {
        None => input,
        Some(keying) => drawn(
            passes,
            Pass::Key {
                input,
                garbage,
                solid,
                mask,
                keying: Box::new(keying),
            },
        ),
    }
}

/// Whether a keyer of `k` that shows `view`, with a garbage matte or without one, is its
/// picture as it came in: it shows the final result, it keys nothing, its thresholds leave a
/// solid matte solid, it suppresses no spill, and no garbage matte takes from it. Its solid
/// matte has then nothing to fill, and its effect mask mixes its picture with itself.
#[allow(
    clippy::float_cmp,
    reason = "a Gain and a spill that are 0 themselves, and the solid end of a matte"
)]
fn keys_nothing(k: &Keyer, view: View, garbage: bool) -> bool {
    view == View::FinalResult
        && !garbage
        && k.gain == 0.0
        && k.spill == 0.0
        && keyer::thresholds(k, 1.0) == 1.0
}

/// How many texels of a filtered texture, which has `filtered` of them, lie to a pixel of a
/// picture of `size` pixels, across and down.
#[allow(
    clippy::cast_precision_loss,
    reason = "sizes of textures, which a float holds"
)]
fn texels(filtered: (u32, u32), size: (u32, u32)) -> [f32; 2] {
    [
        filtered.0 as f32 / size.0.max(1) as f32,
        filtered.1 as f32 / size.1.max(1) as f32,
    ]
}

/// A control of the matte of a keyer that is drawn by the pass that makes `made`
/// ([`KEY_ERODED`], [`KEY_SOFTENED`] or [`KEY_CLEANED`]), with the two `numbers` that the pass
/// reads and the two passes of its `filter` over a matte of `size` pixels.
fn stage(
    made: f32,
    numbers: [f32; 2],
    filter: [neighbourhood::Pass; 2],
    size: (u32, u32),
) -> Stage {
    let mut uniform = [0.0; PARAMS];
    uniform[..2].copy_from_slice(&numbers);
    uniform[MADE_AT] = made;
    uniform[MADE_AT + 2..].copy_from_slice(&texels(filter[1].target, size));
    Stage { filter, uniform }
}

/// What the passes of a Delta Keyer of `settings` are drawn with, for pictures drawn `size`
/// pixels large; `linked` says whether it has a garbage matte, a solid matte and an effect
/// mask. None where the keyer changes nothing, and is its picture as it came in.
///
/// The settings are taken as [`crate::keyer::Node`] and [`Keyer::of`] take them. The controls
/// that read the pixels about each one are each there where they are not at 0, and none of
/// them where the keyer keys nothing: their lengths are in pixels of a picture 1080 high.
#[allow(
    clippy::float_cmp,
    reason = "an Erode/Dilate that is 0 itself, which does nothing"
)]
pub(crate) fn keying(settings: &DeltaKeyed, linked: [bool; 3], size: (u32, u32)) -> Option<Keying> {
    let node = keyer::Node::from(settings);
    let k = node.keyer();
    let [garbage, solid, limited] = linked;
    if keys_nothing(&k, node.view, garbage) {
        return None;
    }
    // A matte of one level all over is left as it is by whatever reads the pixels about one.
    let finesse = if k.gain > 0.0 {
        node.finesse.held()
    } else {
        Finesse::default()
    };
    let per = units(size);
    let gaussian = |sigma: f32| {
        let filter = Filter::Gaussian { sigma: [sigma; 2] };
        neighbourhood::passes(filter, per, size)
    };
    let pre_blur = (finesse.pre_blur > 0.0).then(|| gaussian(finesse.pre_blur));
    let mut stages = Vec::new();
    if finesse.erode_dilate != 0.0 {
        let radius = [finesse.erode_dilate.abs(); 2];
        let filter = neighbourhood::passes(Filter::Box { radius }, per, size);
        let grow = f32::from(finesse.erode_dilate > 0.0);
        let numbers = [grow, keyer::MORPH_EDGE];
        stages.push(stage(KEY_ERODED, numbers, filter, size));
    }
    if finesse.matte_blur > 0.0 {
        let filter = gaussian(finesse.matte_blur);
        stages.push(stage(KEY_SOFTENED, [0.0; 2], filter, size));
    }
    if finesse.clean_foreground > 0.0 || finesse.clean_background > 0.0 {
        let filter = neighbourhood::passes(layout::clean_filter(), per, size);
        let numbers = [finesse.clean_background, finesse.clean_foreground];
        stages.push(stage(KEY_CLEANED, numbers, filter, size));
    }
    let reads = [
        (garbage, READS_GARBAGE),
        (garbage && node.garbage_invert, READS_GARBAGE_INVERTED),
        (solid, READS_SOLID),
        (limited, READS_MASK),
        (!stages.is_empty(), READS_MATTE),
        (pre_blur.is_some(), READS_BLURRED),
    ];
    let mut uniform = [0.0; PARAMS];
    uniform[..MADE_AT].copy_from_slice(&k.uniform());
    uniform[READS_AT] = reads
        .into_iter()
        .filter_map(|(read, flag)| read.then_some(flag))
        .sum();
    uniform[MADE_AT + 1] = match node.view {
        View::PreMatte => 0.0,
        View::Matte => 1.0,
        View::Status => 2.0,
        View::FinalResult => 3.0,
    };
    if let Some(filter) = &pre_blur {
        uniform[MADE_AT + 2..].copy_from_slice(&texels(filter[1].target, size));
    }
    let mut matte = uniform;
    matte[MADE_AT] = KEY_MATTE;
    Some(Keying {
        pre_blur,
        stages,
        matte,
        uniform,
    })
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

/// The shader of the pass that mixes the blurred picture of a Blur with the picture as it
/// came in: `fusion_blur.wgsl`, as it is. The passes that blur are those of
/// `neighbourhood.wgsl`, by the pipeline of the filters of neighbourhoods.
pub(crate) fn blur_shader() -> String {
    BLUR_SHADER.to_owned()
}

/// The shader of the passes of a Mosaic: `fusion_mosaic.wgsl`, as it is.
pub(crate) fn mosaic_shader() -> String {
    MOSAIC_SHADER.to_owned()
}

/// The shader of the passes of a Delta Keyer: `fusion_keyer.wgsl`, as it is. The passes
/// that filter its picture and its matte are those of `neighbourhood.wgsl`, by the pipeline
/// of the filters of neighbourhoods.
pub(crate) fn keyer_shader() -> String {
    KEYER_SHADER.to_owned()
}

/// The pipeline of the passes that lay, of the shader `source` ([`shader`]), of those that
/// draw masks ([`mask_shader`]), of that which mixes a blurred picture ([`blur_shader`]), of
/// those of a Mosaic ([`mosaic_shader`]) or of those of a Delta Keyer ([`keyer_shader`]),
/// `layout` binding what they read: it writes each pixel as it makes it, into a texture of
/// [`FORMAT`].
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
    /// effect mask; and the uniform and the three pictures of the pass of a Blur that mixes
    /// and of the passes of a Mosaic.
    pub layout: wgpu::BindGroupLayout,
    /// What binds the uniform of a pass that draws a mask, the mask that comes into it and
    /// the sides of the polygons of the frame.
    pub mask_layout: wgpu::BindGroupLayout,
    /// What binds the uniform of a pass of a Delta Keyer and the six pictures that it may
    /// read: its picture, what a filter made, the matte that the passes before it drew, and
    /// the pictures of its garbage matte, of its solid matte and of its effect mask.
    pub keyer_layout: wgpu::BindGroupLayout,
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
        let keyer_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("columbina keyer of a composition"),
            entries: &[
                buffer(0, wgpu::BufferBindingType::Uniform, true, PARAMS_SIZE),
                picture(1),
                picture(2),
                picture(3),
                picture(4),
                picture(5),
                picture(6),
            ],
        });
        Card {
            layout,
            mask_layout,
            keyer_layout,
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

    /// Draw a pass that reads up to three pictures into `target`, by `pipeline` and the
    /// uniform at `offset`: the pass of a Blur that mixes, and the passes of a Mosaic, whose
    /// shaders bind what a pass that lays binds. `pictures` are bound in their order, a
    /// texel of nothing in the place of one that the pass has not, which it does not read.
    pub(crate) fn read(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::RenderPipeline,
        offset: u32,
        pictures: [Option<&wgpu::Texture>; 3],
        target: &wgpu::Texture,
    ) {
        let [first, second, third] = pictures.map(|picture| self.view(picture));
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("columbina composition"),
            layout: &self.layout,
            entries: &[
                self.params(),
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&first),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&second),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&third),
                },
            ],
        });
        draw(encoder, pipeline, &bind, offset, target);
    }

    /// Draw a pass of a Delta Keyer into `target`, by `pipeline` and the uniform at `offset`.
    /// `pictures` are bound in their order: the picture as it came in (or, of a pass that
    /// draws the matte anew, the matte so far), what a filter made, the matte that the passes
    /// before drew, and the pictures whose alphas are the garbage matte, the solid matte and
    /// the effect mask of the node; a texel of nothing in the place of one that the pass has
    /// not, which it does not read.
    pub(crate) fn key(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::RenderPipeline,
        offset: u32,
        pictures: [Option<&wgpu::Texture>; 6],
        target: &wgpu::Texture,
    ) {
        let views = pictures.map(|picture| self.view(picture));
        let mut entries = vec![self.params()];
        for (binding, view) in (1..).zip(&views) {
            entries.push(wgpu::BindGroupEntry {
                binding,
                resource: wgpu::BindingResource::TextureView(view),
            });
        }
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("columbina keyer of a composition"),
            layout: &self.keyer_layout,
            entries: &entries,
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

    /// A keyer that is a pass: what makes it one, the mattes and the mask that it reads, and
    /// what is set of a keyer that changes nothing.
    type KeyerPass = (&'static str, [Option<usize>; 3], fn(&mut DeltaKeyed));

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
        plan(&Fused { nodes, output }, size, size, REC709_LUMA, source)
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

    /// A Gaussian Blur of the picture at `input`, with the effect mask `mask`, shown whole:
    /// its Blur Size `size`, across and down.
    fn blur(input: Option<usize>, mask: Option<usize>, size: [f64; 2]) -> FusedKind {
        FusedKind::Blur {
            input,
            mask,
            settings: Blurred {
                filter: model::BlurFilter::Gaussian,
                size,
                blend: 1.0,
            },
        }
    }

    /// A Mosaic of the picture at `input`, keyed by the mask `mask`, of `frequency` square
    /// cells across, its grid where it lies.
    fn cells(input: Option<usize>, mask: Option<usize>, frequency: f64) -> FusedKind {
        FusedKind::Mosaic {
            input,
            mask,
            settings: Mosaicked {
                frequency,
                aspect: 1.0,
                offset: [0.0, 0.0],
                whole_cells_only: false,
            },
        }
    }

    /// The settings of a mosaic at first, but with `frequency` cells across and the grid moved
    /// by `offset`.
    fn grid_of(frequency: f32, offset: [f32; 2]) -> mosaic::Settings {
        mosaic::Settings {
            frequency,
            offset,
            ..mosaic::Settings::default()
        }
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "a blur at two sizes, and one of the other filter with a mask"
    )]
    fn a_blur_is_its_picture_through_a_filter_of_the_neighbourhoods_and_a_pass_that_mixes() {
        assert!(is_drawn(&blur(None, None, [5.0, 5.0])));
        let full = (1920, 1080);
        let graph = || {
            vec![
                node("in", FusedKind::MediaIn),
                node("soft", blur(Some(0), None, [5.0, 5.0])),
            ]
        };
        // In a picture 1080 high a Blur Size is so many pixels: a Gaussian of 5, which is
        // wide enough to be taken at a pixel of two.
        let plan = plan_of(graph(), Some(1), full, true);
        assert_eq!(plan.output, Reads::Pass(0));
        assert!(plan.blurs() && plan.reads_source());
        assert!(!plan.lays() && !plan.masks() && !plan.mosaics());
        let [Pass::Blur {
            input,
            mask,
            filter,
            uniform,
        }] = plan.passes.as_slice()
        else {
            panic!("a blur");
        };
        assert_eq!((*input, *mask), (Reads::Source, Reads::Nothing));
        let gaussian = Filter::Gaussian { sigma: [5.0; 2] };
        assert_eq!(*filter, neighbourhood::passes(gaussian, 1.0, full));
        assert_eq!((filter[0].reach, filter[0].taps), (5.0, 15.0));
        assert_eq!(filter[0].target, (960, 1080));
        assert_eq!(filter[1].target, (960, 540));
        // Shown whole, with no mask; read at half a texel to a pixel, across and down.
        assert_eq!(uniform[..4], [1.0, 0.0, 0.0, 0.0]);
        assert_eq!(uniform[4..8], [0.5, 0.5, 0.0, 0.0]);
        assert_eq!(uniform[8..], [0.0; 12]);
        // One picture of the composition: the textures of its filter are of no slot.
        assert_eq!(
            plan.slots(),
            Slots {
                source: Some(0),
                passes: vec![1],
                count: 2
            }
        );
        // Drawn at half the size, it reaches half as many pixels: the same share of the
        // picture, which is then read texel for pixel.
        let half = plan_of(graph(), Some(1), (960, 540), true);
        let [Pass::Blur {
            filter, uniform, ..
        }] = half.passes.as_slice()
        else {
            panic!("a blur");
        };
        assert_eq!(*filter, neighbourhood::passes(gaussian, 0.5, (960, 540)));
        assert_eq!((filter[0].reach, filter[0].taps), (2.5, 8.0));
        assert_eq!(filter[1].target, (960, 540));
        assert_eq!(uniform[4..6], [1.0, 1.0]);
        // A Box of another radius across than down, half blended in, with a mask: the mask
        // is read by the pass, after it is drawn.
        let boxed = FusedKind::Blur {
            input: Some(1),
            mask: Some(0),
            settings: Blurred {
                filter: model::BlurFilter::Box,
                size: [16.0, 2.0],
                blend: 0.5,
            },
        };
        let plan = plan_of(
            vec![
                node("e", ellipse(None)),
                node("in", FusedKind::MediaIn),
                node("soft", boxed),
            ],
            Some(2),
            full,
            true,
        );
        assert!(plan.blurs() && plan.masks());
        let [Pass::Mask { .. }, Pass::Blur {
            input,
            mask,
            filter,
            uniform,
        }] = plan.passes.as_slice()
        else {
            panic!("a mask and a blur");
        };
        assert_eq!((*input, *mask), (Reads::Source, Reads::Pass(0)));
        let radii = Filter::Box {
            radius: [16.0, 2.0],
        };
        assert_eq!(*filter, neighbourhood::passes(radii, 1.0, full));
        assert!(!filter[0].gaussian && filter[0].across && !filter[1].across);
        assert_eq!((filter[0].reach, filter[1].reach), (16.0, 2.0));
        assert_eq!(filter[1].target, (480, 1080));
        assert_eq!(uniform[..2], [0.5, 1.0]);
        assert_eq!(uniform[4..6], [0.25, 1.0]);
        assert_eq!(
            plan.slots(),
            Slots {
                source: Some(0),
                passes: vec![1, 2],
                count: 3
            }
        );
        assert_eq!(u64::try_from(uniform.len() * 4).unwrap(), PARAMS_SIZE);
    }

    #[test]
    fn a_blur_that_blurs_nothing_or_is_not_blended_in_is_its_input_and_no_pass() {
        // In each graph an Ellipse, which may be the mask, is at 0 and the `MediaIn` at 1.
        let with = |kind: FusedKind| {
            planned(
                vec![
                    node("e", ellipse(None)),
                    node("in", FusedKind::MediaIn),
                    node("soft", kind),
                ],
                Some(2),
            )
        };
        let blended = |blend: f64, mask: Option<usize>| FusedKind::Blur {
            input: Some(1),
            mask,
            settings: Blurred {
                filter: model::BlurFilter::Box,
                size: [40.0, 40.0],
                blend,
            },
        };
        // A Blur Size of 0 across and down, and one that is no number; and a Blend of 0:
        // whatever the mask, which nothing reads then, and is not drawn.
        for mask in [None, Some(0)] {
            for kind in [
                blur(Some(1), mask, [0.0, 0.0]),
                blur(Some(1), mask, [f64::NAN, -1.0]),
                blended(0.0, mask),
                blended(f64::NAN, mask),
            ] {
                let plan = with(kind.clone());
                assert_eq!(plan.passes, [] as [Pass; 0], "{kind:?}");
                assert_eq!(plan.output, Reads::Source, "{kind:?}");
            }
            // A Blur of no picture is no picture.
            assert_eq!(with(blur(None, mask, [40.0, 40.0])), Plan::default());
        }
        // A Blur across alone, or down alone, is a pass; and a Blend above 1 is 1.
        for size in [[40.0, 0.0], [0.0, 40.0]] {
            let plan = with(blur(Some(1), None, size));
            assert_eq!(plan.passes.len(), 1, "{size:?}");
            assert!(plan.blurs(), "{size:?}");
        }
        let plan = with(blended(1.5, None));
        let [Pass::Blur { uniform, .. }] = plan.passes.as_slice() else {
            panic!("a blur");
        };
        assert_eq!(uniform[0], 1.0);
        // A Blur of a Blur: two pictures, each drawn into a texture that the other does not
        // read.
        let chain = planned(
            vec![
                node("in", FusedKind::MediaIn),
                node("first", blur(Some(0), None, [40.0, 40.0])),
                node("second", blur(Some(1), None, [40.0, 40.0])),
            ],
            Some(2),
        );
        assert_eq!(chain.passes.len(), 2);
        let Pass::Blur { input, .. } = &chain.passes[1] else {
            panic!("a blur");
        };
        assert_eq!(*input, Reads::Pass(0));
        // The second is drawn into the texture of the source, which the first was the last
        // to read.
        assert_eq!(
            chain.slots(),
            Slots {
                source: Some(0),
                passes: vec![1, 0],
                count: 2
            }
        );
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "a mosaic with and without a key, and what its passes are given"
    )]
    fn a_mosaic_is_a_pass_of_its_grid_in_whole_steps_keyed_by_its_mask() {
        assert!(is_drawn(&cells(None, None, 4.0)));
        // Four cells across a picture of 16 by 8: cells of four pixels, 1024 steps.
        let plan = planned(
            vec![
                node("in", FusedKind::MediaIn),
                node("tiles", cells(Some(0), None, 4.0)),
            ],
            Some(1),
        );
        assert_eq!(plan.output, Reads::Pass(0));
        assert!(plan.mosaics() && plan.reads_source());
        assert!(!plan.blurs() && !plan.lays() && !plan.masks());
        let [Pass::Mosaic {
            input,
            key,
            grid,
            whole,
        }] = plan.passes.as_slice()
        else {
            panic!("a mosaic");
        };
        assert_eq!(
            (*input, *key, *whole),
            (Reads::Source, Reads::Nothing, false)
        );
        assert_eq!(*grid, Grid::new(16, 8, &grid_of(4.0, [0.0, 0.0])));
        assert_eq!((grid.size, grid.cells()), ([1024, 1024], [4, 2]));
        // What its passes are given: the grid, in whole numbers; and what each makes.
        let across = cells_uniform(grid, false, false);
        assert_eq!(across[..4], [0.0, 0.0, 0.0, 0.0]);
        assert_eq!(across[4..8], [1024.0, 1024.0, 128.0, 128.0]);
        assert_eq!(across[8..12], [16.0, 8.0, 0.0, 0.0]);
        assert_eq!(across[12..], [0.0; 8]);
        assert_eq!(cells_uniform(grid, true, false)[..4], [1.0, 0.0, 0.0, 0.0]);
        assert_eq!(cells_uniform(grid, false, true)[..4], [0.0, 1.0, 0.0, 0.0]);
        assert_eq!(cells_uniform(grid, true, true)[4..], across[4..]);
        for (keyed, number) in [
            (Keyed::Whole, 0.0),
            (Keyed::Pixels, 1.0),
            (Keyed::Cells, 2.0),
        ] {
            let shown = shown_uniform(grid, keyed);
            assert_eq!(shown[..4], [2.0, 0.0, number, 0.0], "{keyed:?}");
            assert_eq!(shown[4..], across[4..], "{keyed:?}");
        }
        assert_eq!(u64::try_from(across.len() * 4).unwrap(), PARAMS_SIZE);
        assert_eq!(
            plan.slots(),
            Slots {
                source: Some(0),
                passes: vec![1],
                count: 2
            }
        );
        // With a mask it is keyed by it; and with Whole Cells Only the least of the key over
        // each cell is made too. Without a mask Whole Cells Only is nothing.
        let keyed = |mask: Option<usize>, whole_cells_only: bool| {
            let kind = FusedKind::Mosaic {
                input: Some(1),
                mask,
                settings: Mosaicked {
                    frequency: 4.0,
                    aspect: 2.0,
                    offset: [1.5, -2.25],
                    whole_cells_only,
                },
            };
            planned(
                vec![
                    node("e", ellipse(None)),
                    node("in", FusedKind::MediaIn),
                    node("tiles", kind),
                ],
                Some(2),
            )
        };
        let moved = mosaic::Settings {
            aspect: 2.0,
            ..grid_of(4.0, [1.5, -2.25])
        };
        for (mask, whole_cells_only, wanted) in [
            (Some(0), false, false),
            (Some(0), true, true),
            (None, true, false),
        ] {
            let plan = keyed(mask, whole_cells_only);
            let Some(Pass::Mosaic {
                input,
                key,
                grid,
                whole,
            }) = plan.passes.last()
            else {
                panic!("a mosaic");
            };
            assert_eq!(*whole, wanted, "{mask:?}, {whole_cells_only}");
            assert_eq!(*input, Reads::Source);
            assert_eq!(*grid, Grid::new(16, 8, &moved));
            // The mask is drawn where it is read alone.
            let reads = mask.map_or(Reads::Nothing, |_| Reads::Pass(0));
            assert_eq!(*key, reads);
            assert_eq!(plan.masks(), mask.is_some());
        }
    }

    #[test]
    fn a_mosaic_of_cells_of_a_pixel_is_its_input_and_one_drawn_smaller_has_its_grid_smaller() {
        let with = |kind: FusedKind| {
            planned(
                vec![
                    node("e", ellipse(None)),
                    node("in", FusedKind::MediaIn),
                    node("tiles", kind),
                ],
                Some(2),
            )
        };
        // As many cells across as the picture has pixels, or more: each is a pixel, and the
        // picture is as it came in, whatever the mask, which is not drawn then.
        for mask in [None, Some(0)] {
            for frequency in [16.0, 100.0, 16_384.0] {
                let plan = with(cells(Some(1), mask, frequency));
                assert_eq!(plan.passes, [] as [Pass; 0], "{frequency}");
                assert_eq!(plan.output, Reads::Source, "{frequency}");
            }
            // A Mosaic of no picture is no picture.
            assert_eq!(with(cells(None, mask, 4.0)), Plan::default());
        }
        // Cells a pixel wide and two high are cells: a pass.
        let tall = FusedKind::Mosaic {
            input: Some(1),
            mask: None,
            settings: Mosaicked {
                frequency: 16.0,
                aspect: 0.5,
                offset: [0.0, 0.0],
                whole_cells_only: false,
            },
        };
        let upright = with(tall);
        let [Pass::Mosaic { grid, .. }] = upright.passes.as_slice() else {
            panic!("a mosaic");
        };
        assert_eq!(grid.size, [mosaic::STEPS, 2 * mosaic::STEPS]);
        // A composition of 32 by 16 drawn at half its size: as many cells across, and the
        // offsets, which are pixels of the composition, half as many pixels as it is drawn.
        let moved = FusedKind::Mosaic {
            input: Some(0),
            mask: None,
            settings: Mosaicked {
                frequency: 4.0,
                aspect: 1.0,
                offset: [4.0, -2.0],
                whole_cells_only: false,
            },
        };
        let drawn_at = |size: (u32, u32)| {
            let composition = Fused {
                nodes: vec![node("in", FusedKind::MediaIn), node("tiles", moved.clone())],
                output: Some(1),
            };
            let made = super::plan(&composition, size, (32, 16), REC709_LUMA, true);
            let [Pass::Mosaic { grid, .. }] = made.passes.as_slice() else {
                panic!("a mosaic");
            };
            *grid
        };
        assert_eq!(
            drawn_at((32, 16)),
            Grid::new(32, 16, &grid_of(4.0, [4.0, -2.0]))
        );
        let half = drawn_at((16, 8));
        assert_eq!(half, Grid::new(16, 8, &grid_of(4.0, [2.0, -1.0])));
        assert_eq!(half.cells(), drawn_at((32, 16)).cells());
    }

    #[test]
    fn the_shaders_of_a_blur_and_of_a_mosaic_bind_what_a_pass_that_lays_binds() {
        for shader in [blur_shader(), mosaic_shader()] {
            assert_eq!(shader.matches("@vertex").count(), 1);
            assert_eq!(shader.matches("@fragment").count(), 1);
            assert_eq!(shader.matches("var<uniform> params:").count(), 1);
            for binding in 0..4 {
                let bound = format!("@group(0) @binding({binding}) var");
                assert_eq!(shader.matches(&bound).count(), 1, "{bound}");
            }
            assert!(!shader.contains("@binding(4)"));
            // No sampler: every picture is read texel by texel.
            assert!(!shader.contains("textureSample"));
        }
        // The steps of the grid are those of the maths, which the shader is held to.
        let steps = format!("const MOSAIC_STEPS: u32 = {}u;", mosaic::STEPS);
        assert!(mosaic_shader().contains(&steps), "{steps}");
        // The shader that lays is as it was: it has nothing of either.
        let lays = super::shader();
        assert!(!lays.contains("fn blur_at(") && !lays.contains("MOSAIC_STEPS"));
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

    /// A Delta Keyer of the picture at `input`, as it is at first but for what `set` sets,
    /// with the nodes at `mattes` linked into its garbage matte, its solid matte and its
    /// effect mask.
    fn delta(
        input: Option<usize>,
        mattes: [Option<usize>; 3],
        set: impl FnOnce(&mut DeltaKeyed),
    ) -> FusedKind {
        let mut settings = model::DeltaKeyer::default().at(0.0);
        set(&mut settings);
        let [garbage_matte, solid_matte, mask] = mattes;
        FusedKind::DeltaKeyer {
            input,
            garbage_matte,
            solid_matte,
            mask,
            settings,
        }
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "a keyer alone, and one with its mattes and its mask, and what each is given"
    )]
    fn a_delta_keyer_is_a_pass_of_its_numbers_that_reads_its_picture_its_mattes_and_its_mask() {
        assert!(is_drawn(&delta(None, [None; 3], |_| {})));
        let plan = planned(
            vec![
                node("in", FusedKind::MediaIn),
                node("key", delta(Some(0), [None; 3], |_| {})),
            ],
            Some(1),
        );
        assert_eq!(plan.output, Reads::Pass(0));
        assert!(plan.keys() && plan.reads_source() && !plan.filters());
        assert!(!plan.blurs() && !plan.lays() && !plan.masks() && !plan.mosaics());
        let [Pass::Key {
            input,
            garbage,
            solid,
            mask,
            keying,
        }] = plan.passes.as_slice()
        else {
            panic!("a keyer");
        };
        assert_eq!(
            [*input, *garbage, *solid, *mask],
            [
                Reads::Source,
                Reads::Nothing,
                Reads::Nothing,
                Reads::Nothing
            ]
        );
        // It reads no pixel about another: no filter, and no matte drawn before it.
        assert_eq!(keying.pre_blur, None);
        assert_eq!(keying.stages, [] as [Stage; 0]);
        // The four rows of the keyer of its settings, with what it reads in the place that
        // they leave free: nothing but its picture. And what it makes: the picture, of the
        // final result.
        let uniform = keying.uniform;
        let rows = Keyer::of(&keyer::Settings::default()).uniform();
        assert_eq!(uniform[..READS_AT], rows[..READS_AT]);
        assert_eq!(uniform[READS_AT + 1..MADE_AT], rows[READS_AT + 1..]);
        assert_eq!(uniform[..4], [0.0, 1.0, 0.0, 1.0]);
        assert_eq!(uniform[4..8], [1.0, 1.0, 0.5, 0.0]);
        assert_eq!(uniform[8..12], [0.0, 1.0, 0.5, 1.0]);
        assert_eq!(uniform[12..16], [0.5, 0.5, 0.5, 3.0]);
        assert_eq!(uniform[MADE_AT..], [0.0, 3.0, 0.0, 0.0]);
        assert_eq!(u64::try_from(uniform.len() * 4).unwrap(), PARAMS_SIZE);
        // One picture of the composition.
        assert_eq!(
            plan.slots(),
            Slots {
                source: Some(0),
                passes: vec![1],
                count: 2
            }
        );
        // With a garbage matte read the other way round, a solid matte and a mask: each is
        // read by the pass, after it is drawn; and the view of the node goes with it.
        let plan = planned(
            vec![
                node("g", rectangle(None)),
                node("s", ellipse(None)),
                node("e", ellipse(None)),
                node("in", FusedKind::MediaIn),
                node(
                    "key",
                    delta(Some(3), [Some(0), Some(1), Some(2)], |keyer| {
                        keyer.garbage_invert = true;
                        keyer.view = model::KeyerView::Status;
                    }),
                ),
            ],
            Some(4),
        );
        assert_eq!(plan.passes.len(), 4);
        assert!(plan.keys() && plan.masks() && !plan.filters());
        let Pass::Key {
            input,
            garbage,
            solid,
            mask,
            keying,
        } = &plan.passes[3]
        else {
            panic!("a keyer");
        };
        assert_eq!(
            [*input, *garbage, *solid, *mask],
            [
                Reads::Source,
                Reads::Pass(0),
                Reads::Pass(1),
                Reads::Pass(2)
            ]
        );
        let all = READS_GARBAGE + READS_GARBAGE_INVERTED + READS_SOLID + READS_MASK;
        assert_eq!(keying.uniform[READS_AT], all);
        assert_eq!(keying.uniform[MADE_AT..], [0.0, 2.0, 0.0, 0.0]);
        assert_eq!(
            plan.slots(),
            Slots {
                source: Some(0),
                passes: vec![1, 2, 3, 4],
                count: 5
            }
        );
        // The invert of a garbage matte that is not there is nothing; and a matte that names
        // a node which gives no picture is not there, as a mask is no mask.
        let plan = planned(
            vec![
                node("in", FusedKind::MediaIn),
                node("clear", background([1.0, 1.0, 1.0, 0.0])),
                node(
                    "key",
                    delta(Some(0), [Some(1); 3], |keyer| keyer.garbage_invert = true),
                ),
            ],
            Some(2),
        );
        let [Pass::Key {
            garbage,
            solid,
            mask,
            keying,
            ..
        }] = plan.passes.as_slice()
        else {
            panic!("a keyer");
        };
        assert_eq!([*garbage, *solid, *mask], [Reads::Nothing; 3]);
        assert_eq!(keying.uniform[READS_AT], 0.0);
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "each keyer that changes nothing, and each that seems to and does not"
    )]
    fn a_delta_keyer_that_changes_nothing_is_its_input_and_no_pass() {
        // In each graph an Ellipse, which may be a matte or the mask, is at 0 and the
        // `MediaIn` at 1.
        let with = |kind: FusedKind| {
            planned(
                vec![
                    node("e", ellipse(None)),
                    node("in", FusedKind::MediaIn),
                    node("key", kind),
                ],
                Some(2),
            )
        };
        let idle: fn(&mut DeltaKeyed) = |keyer| {
            keyer.gain = 0.0;
            keyer.spill_method = model::SpillMethod::None;
        };
        // A Gain of 0 and no spill method, the final result shown: the picture as it came
        // in, whatever its solid matte, which has nothing to fill, and whatever its mask;
        // neither is drawn then.
        for mattes in [
            [None; 3],
            [None, Some(0), None],
            [None, None, Some(0)],
            [None, Some(0), Some(0)],
        ] {
            let plan = with(delta(Some(1), mattes, idle));
            assert_eq!(plan.passes, [] as [Pass; 0], "{mattes:?}");
            assert_eq!(plan.output, Reads::Source, "{mattes:?}");
        }
        // The same with a Spill Suppression of 0 in place of no method; with every control
        // that reads the pixels about each one set, and any replace mode; with a screen
        // colour that has no channel above its other two, which keys nothing; and with a
        // Gain and a Spill Suppression that are no numbers.
        let others: [fn(&mut DeltaKeyed); 4] = [
            |keyer| {
                keyer.gain = 0.0;
                keyer.spill_suppression = 0.0;
            },
            |keyer| {
                keyer.gain = 0.0;
                keyer.spill_method = model::SpillMethod::None;
                keyer.pre_blur = 5.0;
                keyer.erode_dilate = -3.0;
                keyer.matte_blur = 4.0;
                keyer.clean_foreground = 0.5;
                keyer.clean_background = 0.5;
                keyer.replace = model::ReplaceMode::Source;
            },
            |keyer| {
                keyer.screen = [0.5; 3];
                keyer.spill_method = model::SpillMethod::None;
            },
            |keyer| {
                keyer.gain = f64::NAN;
                keyer.spill_suppression = f64::NAN;
            },
        ];
        for (at, set) in others.into_iter().enumerate() {
            let plan = with(delta(Some(1), [None; 3], set));
            assert_eq!(plan.passes, [] as [Pass; 0], "{at}");
            assert_eq!(plan.output, Reads::Source, "{at}");
        }
        // But it is a pass with a garbage matte, which takes from the alpha; where it
        // suppresses spill, which changes the colours though nothing is keyed; where it
        // shows a matte, a picture of its own; where its thresholds are both 1, which leave
        // nothing solid; and where it keys.
        let passes: [KeyerPass; 5] = [
            ("a garbage matte", [Some(0), None, None], idle),
            ("a spill method", [None; 3], |keyer| keyer.gain = 0.0),
            ("a view of the matte", [None; 3], |keyer| {
                keyer.gain = 0.0;
                keyer.spill_method = model::SpillMethod::None;
                keyer.view = model::KeyerView::Matte;
            }),
            ("thresholds that are both 1", [None; 3], |keyer| {
                keyer.gain = 0.0;
                keyer.spill_method = model::SpillMethod::None;
                keyer.low = 1.0;
                keyer.high = 1.0;
            }),
            ("a gain", [None; 3], |keyer| {
                keyer.spill_method = model::SpillMethod::None;
            }),
        ];
        for (what, mattes, set) in passes {
            let plan = with(delta(Some(1), mattes, set));
            assert!(plan.keys(), "{what}");
            assert_eq!(plan.output, Reads::Pass(plan.passes.len() - 1), "{what}");
            // A keyer that keys nothing reads no pixel about another.
            assert!(!plan.filters(), "{what}");
        }
        // A keyer of no picture is no picture, whatever its view and its mattes.
        for view in [
            model::KeyerView::PreMatte,
            model::KeyerView::Matte,
            model::KeyerView::Status,
            model::KeyerView::FinalResult,
        ] {
            let nothing = with(delta(None, [Some(0); 3], |keyer| keyer.view = view));
            assert_eq!(nothing, Plan::default(), "{view:?}");
        }
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "each control of a keyer that reads the pixels about each one, and all of them"
    )]
    fn the_controls_of_a_keyer_about_a_pixel_are_filters_of_the_neighbourhoods_and_none_at_0() {
        let full = (1920, 1080);
        let drawn_at = |size: (u32, u32), set: fn(&mut DeltaKeyed)| {
            let plan = plan_of(
                vec![
                    node("in", FusedKind::MediaIn),
                    node("key", delta(Some(0), [None; 3], set)),
                ],
                Some(1),
                size,
                true,
            );
            let [Pass::Key { keying, .. }] = plan.passes.as_slice() else {
                panic!("a keyer");
            };
            let reads = keying.pre_blur.is_some() || !keying.stages.is_empty();
            assert_eq!(plan.filters(), reads);
            // One picture of the composition, whatever textures of its own it is made
            // through.
            assert_eq!(
                plan.slots(),
                Slots {
                    source: Some(0),
                    passes: vec![1],
                    count: 2
                }
            );
            (**keying).clone()
        };
        let with = |set: fn(&mut DeltaKeyed)| drawn_at(full, set);
        let gaussian = |sigma: f32| {
            let filter = Filter::Gaussian { sigma: [sigma; 2] };
            neighbourhood::passes(filter, 1.0, full)
        };
        // As it is at first: none of them. The pass that would draw the matte of the key
        // alone has the numbers of the pass that makes the picture, and makes the matte.
        let first = with(|_| {});
        assert_eq!((first.pre_blur, first.stages.len()), (None, 0));
        assert_eq!(first.uniform[READS_AT], 0.0);
        assert_eq!(first.matte[..MADE_AT], first.uniform[..MADE_AT]);
        assert_eq!(first.matte[MADE_AT..], [KEY_MATTE, 3.0, 0.0, 0.0]);
        // A Pre-Blur: in a picture 1080 high it is so many pixels, a Gaussian of 5, which is
        // taken at a pixel of two and read at half a texel to a pixel.
        let soft = with(|keyer| keyer.pre_blur = 5.0);
        assert_eq!(soft.pre_blur, Some(gaussian(5.0)));
        assert_eq!(soft.stages.len(), 0);
        assert_eq!(soft.uniform[READS_AT], READS_BLURRED);
        assert_eq!(soft.uniform[MADE_AT..], [0.0, 3.0, 0.5, 0.5]);
        assert_eq!(soft.matte[MADE_AT..], [KEY_MATTE, 3.0, 0.5, 0.5]);
        // Every control of the matte: Erode/Dilate, the Matte Blur and the Cleans, in that
        // order, each with the two passes of its filter and what the pass after them reads.
        let every = with(|keyer| {
            keyer.erode_dilate = -3.0;
            keyer.matte_blur = 2.0;
            keyer.clean_foreground = 0.25;
            keyer.clean_background = 0.5;
        });
        assert_eq!(every.pre_blur, None);
        assert_eq!(every.uniform[READS_AT], READS_MATTE);
        assert_eq!(every.uniform[MADE_AT..], [0.0, 3.0, 0.0, 0.0]);
        let [eroded, softened, cleaned] = every.stages.as_slice() else {
            panic!("three controls");
        };
        let boxed = neighbourhood::passes(Filter::Box { radius: [3.0; 2] }, 1.0, full);
        assert_eq!(eroded.filter, boxed);
        assert_eq!(eroded.uniform[..2], [0.0, keyer::MORPH_EDGE]);
        assert_eq!(eroded.uniform[MADE_AT..], [KEY_ERODED, 0.0, 1.0, 1.0]);
        assert_eq!(softened.filter, gaussian(2.0));
        assert_eq!(softened.uniform[..2], [0.0, 0.0]);
        assert_eq!(softened.uniform[MADE_AT..], [KEY_SOFTENED, 0.0, 1.0, 1.0]);
        let clean = neighbourhood::passes(layout::clean_filter(), 1.0, full);
        assert_eq!(cleaned.filter, clean);
        assert_eq!(cleaned.uniform[..2], [0.5, 0.25]);
        assert_eq!(cleaned.uniform[MADE_AT..], [KEY_CLEANED, 0.0, 1.0, 1.0]);
        for stage in &every.stages {
            assert_eq!(stage.uniform[2..MADE_AT], [0.0; 14]);
            assert_eq!(u64::try_from(stage.uniform.len() * 4).unwrap(), PARAMS_SIZE);
        }
        // Above 0 Erode/Dilate grows the matte; a box of 12 pixels is taken at a pixel of
        // two.
        let grown = with(|keyer| keyer.erode_dilate = 12.0);
        assert_eq!(grown.stages.len(), 1);
        assert_eq!(grown.stages[0].uniform[..2], [1.0, keyer::MORPH_EDGE]);
        assert_eq!(grown.stages[0].uniform[MADE_AT + 2..], [0.5, 0.5]);
        // Either Clean alone is the control.
        let specks = with(|keyer| keyer.clean_background = 0.5);
        assert_eq!(specks.stages.len(), 1);
        assert_eq!(specks.stages[0].uniform[..2], [0.5, 0.0]);
        // All of them: a keyer of thirteen passes of the card.
        let all = with(|keyer| {
            keyer.pre_blur = 5.0;
            keyer.erode_dilate = 2.0;
            keyer.matte_blur = 2.0;
            keyer.clean_foreground = 0.5;
        });
        assert_eq!((all.pre_blur.is_some(), all.stages.len()), (true, 3));
        assert_eq!(all.uniform[READS_AT], READS_MATTE + READS_BLURRED);
        // A keyer that keys nothing reads no pixel about another, whatever they are set to:
        // it is one pass, which suppresses the spill.
        let idle = with(|keyer| {
            keyer.gain = 0.0;
            keyer.pre_blur = 5.0;
            keyer.erode_dilate = -3.0;
            keyer.matte_blur = 2.0;
            keyer.clean_foreground = 1.0;
        });
        assert_eq!((idle.pre_blur, idle.stages.len()), (None, 0));
        assert_eq!(idle.uniform[READS_AT], 0.0);
        // Drawn at half the size, they reach half as many pixels: the same share of the
        // picture.
        let small = (960, 540);
        let half = drawn_at(small, |keyer| {
            keyer.pre_blur = 5.0;
            keyer.erode_dilate = -3.0;
        });
        let sigma = Filter::Gaussian { sigma: [5.0; 2] };
        assert_eq!(
            half.pre_blur,
            Some(neighbourhood::passes(sigma, 0.5, small))
        );
        let radius = Filter::Box { radius: [3.0; 2] };
        assert_eq!(
            half.stages[0].filter,
            neighbourhood::passes(radius, 0.5, small)
        );
        assert_eq!(half.uniform[MADE_AT + 2..], [1.0, 1.0]);
    }

    #[test]
    fn the_shader_of_a_keyer_binds_its_six_pictures_and_has_the_numbers_of_the_maths() {
        let shader = keyer_shader();
        assert_eq!(shader.matches("@vertex").count(), 1);
        assert_eq!(shader.matches("@fragment").count(), 1);
        assert_eq!(shader.matches("var<uniform> params:").count(), 1);
        for binding in 0..7 {
            let bound = format!("@group(0) @binding({binding}) var");
            assert_eq!(shader.matches(&bound).count(), 1, "{bound}");
        }
        assert!(!shader.contains("@binding(7)"));
        // No sampler: every picture is read texel by texel.
        assert!(!shader.contains("textureSample"));
        // The numbers of the maths, which the shader is held to.
        let largest = format!("const KEYER_LARGEST: f32 = {:.1};", keyer::LARGEST);
        assert!(shader.contains(&largest), "{largest}");
        assert!(shader.contains("const KEYER_STATUS_TOLERANCE: f32 = 0.00390625;"));
        assert_eq!(keyer::STATUS_TOLERANCE, 0.003_906_25);
        // The shaders of the other passes are as they were: they have nothing of it.
        for other in [
            super::shader(),
            mask_shader(),
            blur_shader(),
            mosaic_shader(),
        ] {
            assert!(!other.contains("keyer_") && !other.contains("KEYER_"));
        }
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

    /// Where the picture that a Tracker of the tests moves is put: two pixels to the right.
    const PUT: [f64; 2] = [0.625, 0.5];

    /// A Tracker that match moves and draws as `merge` says, its `input`, its `foreground`
    /// and its `mask` reading `reads`: the picture that it moves is put at [`PUT`].
    fn tracker(merge: model::TrackerMerge, reads: [Option<usize>; 3]) -> FusedKind {
        let [input, foreground, mask] = reads;
        FusedKind::Tracker {
            input,
            foreground,
            mask,
            settings: Tracked {
                operation: model::TrackerOperation::MatchMove,
                merge,
                center: PUT,
                size: 1.0,
                angle: 0.0,
            },
        }
    }

    /// The passes of the source of the clip, a Background and an Ellipse, and of `last`
    /// after them, which the `MediaOut` reads.
    fn after_three(last: FusedKind) -> Plan {
        planned(
            vec![
                node("in", FusedKind::MediaIn),
                node("bg", background([0.2, 0.4, 0.6, 1.0])),
                node("e", ellipse(None)),
                node("last", last),
            ],
            Some(3),
        )
    }

    /// What a pass that lays reads, as its background, its foreground and its mask, and
    /// its uniform.
    fn laid(pass: &Pass) -> ([Reads; 3], [f32; PARAMS]) {
        let Pass::Lay {
            background,
            foreground,
            mask,
            uniform,
        } = pass
        else {
            panic!("{pass:?} lays nothing");
        };
        ([*background, *foreground, *mask], *uniform)
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "each way of a tracker, with a mask and without"
    )]
    fn a_tracker_is_drawn_by_the_passes_of_a_transform_and_of_a_merge() {
        use model::TrackerMerge::{BgOnly, BgOverFg, FgOnly, FgOverBg};
        let put = Transformed {
            center: PUT,
            ..transform()
        };
        let over = Merged {
            center: PUT,
            ..merge()
        };
        for mask in [None, Some(2)] {
            // Held still: a Transform of the picture that was tracked.
            let held = after_three(tracker(BgOnly, [Some(0), Some(1), mask]));
            let transform = FusedKind::Transform {
                input: Some(0),
                mask,
                settings: put,
            };
            assert_eq!(held, after_three(transform), "{mask:?}");
            assert!(held.lays() && held.reads_source());
            // The foreground as it follows, over the picture that was tracked: a Merge.
            let follows = after_three(tracker(FgOverBg, [Some(0), Some(1), mask]));
            let merge = FusedKind::Merge {
                background: Some(0),
                foreground: Some(1),
                mask,
                settings: over,
            };
            assert_eq!(follows, after_three(merge), "{mask:?}");
        }
        // The foreground alone: a Transform of it, which reads nothing of the picture that
        // was tracked.
        let alone = after_three(tracker(FgOnly, [Some(0), Some(1), None]));
        let transform = FusedKind::Transform {
            input: Some(1),
            mask: None,
            settings: put,
        };
        assert_eq!(alone, after_three(transform));
        assert!(!alone.reads_source());
        // With a mask and no picture to be limited to: the foreground as it follows times
        // the mask, as a Merge over no background is.
        let masked = after_three(tracker(FgOnly, [None, Some(1), Some(2)]));
        let merge = FusedKind::Merge {
            background: None,
            foreground: Some(1),
            mask: Some(2),
            settings: over,
        };
        assert_eq!(masked, after_three(merge));
        // With a mask over the picture that was tracked, a pass more: the foreground as it
        // follows, alone; and that under the picture as it came in, of which nothing is
        // laid, limited to it.
        let moved = uniform(
            &reading(SIZE, PUT, 1.0, 0.0, (false, false)),
            1.0,
            &Settings::default(),
            REC709_LUMA,
        );
        let to_its_picture = |mut numbers: [f32; PARAMS]| {
            numbers[LIMITED_AT] = LIMITED_TO_FOREGROUND;
            numbers
        };
        let both = after_three(tracker(FgOnly, [Some(0), Some(1), Some(2)]));
        assert_eq!(both.passes.len(), 4);
        assert_eq!(both.output, Reads::Pass(3));
        assert!(matches!(both.passes[0], Pass::Fill(_)));
        assert!(matches!(both.passes[1], Pass::Mask { .. }));
        let nothing = Reads::Nothing;
        assert_eq!(
            laid(&both.passes[2]),
            ([nothing, Reads::Pass(0), nothing], moved)
        );
        let nowhere = uniform(&NOWHERE, 1.0, &Settings::default(), REC709_LUMA);
        assert_eq!(
            laid(&both.passes[3]),
            (
                [Reads::Pass(2), Reads::Source, Reads::Pass(1)],
                to_its_picture(nowhere)
            )
        );
        // The picture that was tracked over the foreground as it follows: the foreground
        // moved, and the picture laid over it as it lies by a Merge as it is at first.
        let as_it_lies = merge_uniform(SIZE, &Settings::default(), REC709_LUMA);
        let under = after_three(tracker(BgOverFg, [Some(0), Some(1), None]));
        assert_eq!(under.passes.len(), 3);
        assert_eq!(under.output, Reads::Pass(2));
        assert_eq!(
            laid(&under.passes[1]),
            ([nothing, Reads::Pass(0), nothing], moved)
        );
        assert_eq!(
            laid(&under.passes[2]),
            ([Reads::Pass(1), Reads::Source, nothing], as_it_lies)
        );
        // With a mask, that pass is limited to the picture as it came in.
        let under = after_three(tracker(BgOverFg, [Some(0), Some(1), Some(2)]));
        assert_eq!(under.passes.len(), 4);
        assert_eq!(
            laid(&under.passes[3]),
            (
                [Reads::Pass(2), Reads::Source, Reads::Pass(1)],
                to_its_picture(as_it_lies)
            )
        );
        // And over no picture it is the foreground as it follows, times its mask where it
        // has one.
        let bare = after_three(tracker(BgOverFg, [None, Some(1), None]));
        assert_eq!(bare, alone);
        let bare = after_three(tracker(BgOverFg, [None, Some(1), Some(2)]));
        assert_eq!(bare.passes.len(), 4);
        let (reads, by) = laid(&bare.passes[3]);
        assert_eq!(reads, [nothing, Reads::Pass(2), Reads::Pass(1)]);
        assert_eq!(by[LIMITED_AT], LIMITED_TO_BACKGROUND);
        // Every one of them is drawn as what it is.
        for merge in [BgOnly, FgOnly, FgOverBg, BgOverFg] {
            assert!(is_drawn(&tracker(merge, [Some(0), Some(1), Some(2)])));
        }
    }

    #[test]
    fn a_tracker_that_moves_nothing_is_no_pass() {
        use model::TrackerMerge::{BgOnly, BgOverFg, FgOnly, FgOverBg};
        let source = Plan {
            passes: vec![],
            output: Reads::Source,
        };
        // With an Operation of None it is the picture that was tracked, whatever else.
        for merge in [BgOnly, FgOnly, FgOverBg, BgOverFg] {
            for mask in [None, Some(2)] {
                let mut idle = tracker(merge, [Some(0), Some(1), mask]);
                if let FusedKind::Tracker { settings, .. } = &mut idle {
                    settings.operation = model::TrackerOperation::None;
                }
                assert_eq!(after_three(idle), source, "{merge:?}, {mask:?}");
            }
        }
        // The picture that it moves lying as it is: at the reference frame, and where there
        // is no motion to be had.
        let still = |merge: model::TrackerMerge, reads: [Option<usize>; 3]| {
            let mut kind = tracker(merge, reads);
            if let FusedKind::Tracker { settings, .. } = &mut kind {
                settings.center = [0.5, 0.5];
            }
            after_three(kind)
        };
        // Held still, it is the picture that was tracked, whatever its mask.
        for mask in [None, Some(2)] {
            assert_eq!(still(BgOnly, [Some(0), Some(1), mask]), source, "{mask:?}");
        }
        // Alone, it is its foreground.
        let alone = still(FgOnly, [Some(0), Some(1), None]);
        assert_eq!(alone.passes, vec![Pass::Fill([0.2, 0.4, 0.6, 1.0])]);
        assert_eq!(alone.output, Reads::Pass(0));
        // With no foreground it is the picture that was tracked, over which and under which
        // nothing lies, whatever its mask; and held still of no picture it is none.
        for merge in [FgOverBg, BgOverFg] {
            for mask in [None, Some(2)] {
                let bare = after_three(tracker(merge, [Some(0), None, mask]));
                assert_eq!(bare, source, "{merge:?}, {mask:?}");
            }
        }
        let none = after_three(tracker(BgOnly, [None, Some(1), None]));
        assert_eq!(none, Plan::default());
        // No foreground alone is nothing: with a mask, the picture that was tracked where
        // the mask is 0, which is a pass.
        let none = after_three(tracker(FgOnly, [Some(0), None, None]));
        assert_eq!(none, Plan::default());
        let left = after_three(tracker(FgOnly, [Some(0), None, Some(2)]));
        assert_eq!(left.passes.len(), 2);
        let (reads, by) = laid(&left.passes[1]);
        assert_eq!(reads, [Reads::Nothing, Reads::Source, Reads::Pass(0)]);
        assert_eq!(by[LIMITED_AT], LIMITED_TO_FOREGROUND);
        // One picture as what was tracked and as the foreground that lies as it is, with a
        // mask: the two that are mixed are one, and it is that.
        let same = still(FgOnly, [Some(0), Some(0), Some(2)]);
        assert_eq!(same, source);
    }

    #[test]
    fn a_picture_put_by_a_centre_a_size_and_a_turn_is_read_where_it_was() {
        // What the model takes a Transform to do where it hands a Tracker on as one: the
        // middle of the picture put at the centre, and about it a size and a turn
        // counter-clockwise. A pixel then reads the place that was put there.
        let (width, height) = (64.0, 32.0);
        let (center, scale, angle) = ([0.4, 0.7], 1.5, 30.0_f64);
        let map = reading((64, 32), center, scale, angle, (false, false));
        let (sin, cos) = angle.to_radians().sin_cos();
        let to = (center[0] * width, (1.0 - center[1]) * height);
        for point in [(0.0, 0.0), (64.0, 0.0), (10.5, 20.25), (32.0, 16.0)] {
            let (x, y) = (point.0 - width / 2.0, point.1 - height / 2.0);
            let put = (
                to.0 + scale * (x * cos + y * sin),
                to.1 + scale * (-x * sin + y * cos),
            );
            let read = map.apply(put);
            assert!(
                (read.0 - point.0).abs() < 1e-9 && (read.1 - point.1).abs() < 1e-9,
                "{point:?} is read at {read:?}"
            );
        }
    }

    /// The stabilizer of the source by `matrix`, in shares of the picture: a `MediaIn` and the
    /// node that the model puts after it.
    fn steady(matrix: [[f64; 3]; 3]) -> Plan {
        planned(
            vec![
                node("in", FusedKind::MediaIn),
                node(
                    "stabilized",
                    FusedKind::Stabilize {
                        input: Some(0),
                        matrix,
                    },
                ),
            ],
            Some(1),
        )
    }

    #[test]
    fn the_stabilizer_is_the_pass_of_a_transform_of_the_source_and_none_where_it_moves_nothing() {
        // Where it moves no point, and where it flattens the picture, it is the source.
        for matrix in [
            columbina_model::stabilize::IDENTITY,
            [[0.0, 0.0, 0.5], [0.0, 0.0, 0.5], [0.0, 0.0, 1.0]],
        ] {
            let plan = steady(matrix);
            assert_eq!(plan.passes, [] as [Pass; 0], "{matrix:?}");
            assert_eq!(plan.output, Reads::Source, "{matrix:?}");
        }
        // Twice as large about the middle: the pass of a Transform of a Size of 2, to the
        // number, but for the sign of a zero.
        let zoomed = steady([[2.0, 0.0, -0.5], [0.0, 2.0, -0.5], [0.0, 0.0, 1.0]]);
        let transform = planned(
            vec![
                node("in", FusedKind::MediaIn),
                node(
                    "t",
                    FusedKind::Transform {
                        input: Some(0),
                        mask: None,
                        settings: Transformed {
                            size: 2.0,
                            ..transform()
                        },
                    },
                ),
            ],
            Some(1),
        );
        assert_eq!(zoomed, transform);
        let [Pass::Lay {
            background,
            foreground,
            mask,
            uniform,
        }] = zoomed.passes.as_slice()
        else {
            panic!("one pass that lays: {zoomed:?}")
        };
        assert_eq!(
            [*background, *foreground, *mask],
            [Reads::Nothing, Reads::Source, Reads::Nothing]
        );
        assert_eq!(
            [uniform[PROJECTIVE_ACROSS_AT], uniform[PROJECTIVE_DOWN_AT]],
            [0.0, 0.0]
        );
        // Of no picture, no picture.
        let nothing = plan_of(
            vec![
                node("in", FusedKind::MediaIn),
                node(
                    "stabilized",
                    FusedKind::Stabilize {
                        input: Some(0),
                        matrix: [[2.0, 0.0, -0.5], [0.0, 2.0, -0.5], [0.0, 0.0, 1.0]],
                    },
                ),
            ],
            Some(1),
            SIZE,
            false,
        );
        assert_eq!(nothing, Plan::default());
    }

    #[test]
    fn the_map_of_the_stabilizer_reads_where_its_matrix_put_each_point() {
        // A turn, a zoom, a shift and a little perspective, in shares of a picture of 64 × 32.
        let matrix = [[1.05, -0.02, 0.01], [0.03, 0.98, -0.02], [0.04, -0.03, 1.0]];
        let (width, height) = (64.0, 32.0);
        let (map, scale) = steadied(&matrix, (64, 32)).unwrap();
        assert!(scale > 0.9 && scale < 1.1, "{scale}");
        assert!(map.across != 0.0 && map.down != 0.0);
        for point in [(0.0, 0.0), (64.0, 32.0), (10.5, 20.25), (32.0, 16.0)] {
            // Where the matrix puts the point, in pixels.
            let (u, v) = (point.0 / width, point.1 / height);
            let w = matrix[2][0] * u + matrix[2][1] * v + matrix[2][2];
            let put = (
                (matrix[0][0] * u + matrix[0][1] * v + matrix[0][2]) / w * width,
                (matrix[1][0] * u + matrix[1][1] * v + matrix[1][2]) / w * height,
            );
            // And where the pixel there reads, as the shader reads it.
            let affine = map.affine.apply(put);
            let divisor = map.across * put.0 + map.down * put.1 + 1.0;
            let read = (affine.0 / divisor, affine.1 / divisor);
            assert!(
                (read.0 - point.0).abs() < 1e-9 && (read.1 - point.1).abs() < 1e-9,
                "{point:?} is read at {read:?}"
            );
        }
        // A shift of an eighth of the width is one of 8 pixels, affine.
        let (shifted, scale) = steadied(
            &[[1.0, 0.0, 0.125], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            (64, 32),
        )
        .unwrap();
        assert_eq!(scale, 1.0);
        assert_eq!((shifted.across, shifted.down), (0.0, 0.0));
        assert_eq!(shifted.affine.apply((8.5, 3.5)), (0.5, 3.5));
        assert_eq!(steadied(&[[0.0; 3]; 3], (64, 32)), None);
        assert_eq!(steadied(&matrix, (0, 32)), None);
    }

    /// A Planar Tracker that does `operation` and puts the picture that it moves by `matrix`,
    /// in shares of the picture from its top left corner, its `input`, its `foreground` and its
    /// `mask` reading `reads`.
    fn planar_node(
        operation: model::PlanarOperation,
        matrix: [[f64; 3]; 3],
        reads: [Option<usize>; 3],
    ) -> FusedKind {
        let [input, foreground, mask] = reads;
        FusedKind::PlanarTracker {
            input,
            foreground,
            mask,
            settings: PlanarTracked { operation, matrix },
        }
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "each operation of a planar tracker, with a mask and without"
    )]
    fn a_planar_tracker_is_one_pass_that_reads_its_picture_through_its_matrix() {
        use model::PlanarOperation::{CornerPin, None as Idle, Steady};
        let identity = columbina_model::stabilize::IDENTITY;
        // Two pixels of 16 to the right; and that, with a little perspective across.
        let shift = [[1.0, 0.0, 0.125], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let tilted = [[1.0, 0.0, 0.125], [0.0, 1.0, 0.0], [0.05, 0.0, 1.0]];
        let source = Plan {
            passes: vec![],
            output: Reads::Source,
        };
        // With an operation of None, and held still where the matrix moves nothing or cannot
        // be undone, it is the picture that was tracked, whatever its mask; and so is a Corner
        // Pin with no foreground.
        for mask in [None, Some(2)] {
            let reads = [Some(0), Some(1), mask];
            assert_eq!(after_three(planar_node(Idle, shift, reads)), source);
            assert_eq!(after_three(planar_node(Steady, identity, reads)), source);
            let flat = [[0.0; 3]; 3];
            assert_eq!(after_three(planar_node(Steady, flat, reads)), source);
            let bare = [Some(0), None, mask];
            assert_eq!(after_three(planar_node(CornerPin, shift, bare)), source);
        }
        // Held still of no picture, it is none.
        let none = after_three(planar_node(Steady, shift, [None, Some(1), None]));
        assert_eq!(none, Plan::default());
        // Held still without a mask: the pass of the stabilizer of the picture, to the number.
        for matrix in [shift, tilted] {
            let still = after_three(planar_node(Steady, matrix, [Some(0), Some(1), None]));
            let stabilizer = FusedKind::Stabilize {
                input: Some(0),
                matrix,
            };
            assert_eq!(still, after_three(stabilizer), "{matrix:?}");
            assert_eq!(still.passes.len(), 1);
        }
        // With a mask, that pass limited to the picture as it came in.
        let mut wanted = projective(&tilted, SIZE, &Settings::default(), REC709_LUMA).unwrap();
        assert_ne!(wanted[PROJECTIVE_ACROSS_AT], 0.0);
        wanted[LIMITED_AT] = LIMITED_TO_FOREGROUND;
        let with_mask = after_three(planar_node(Steady, tilted, [Some(0), Some(1), Some(2)]));
        assert_eq!(with_mask.passes.len(), 2);
        assert!(matches!(with_mask.passes[0], Pass::Mask { .. }));
        assert_eq!(
            laid(&with_mask.passes[1]),
            ([Reads::Nothing, Reads::Source, Reads::Pass(0)], wanted)
        );
        assert_eq!(with_mask.output, Reads::Pass(1));
        // A Corner Pin: the foreground, put by the matrix, laid over the picture that was
        // tracked by a Merge as it is at first; with a mask, limited to that picture.
        let plain = held(&model::Merge::default().at(0.0));
        let laid_by = projective(&tilted, SIZE, &plain, REC709_LUMA).unwrap();
        let pinned = after_three(planar_node(CornerPin, tilted, [Some(0), Some(1), None]));
        assert_eq!(pinned.passes.len(), 2);
        assert_eq!(pinned.passes[0], Pass::Fill([0.2, 0.4, 0.6, 1.0]));
        assert_eq!(
            laid(&pinned.passes[1]),
            ([Reads::Source, Reads::Pass(0), Reads::Nothing], laid_by)
        );
        let mut limited_by = laid_by;
        limited_by[LIMITED_AT] = LIMITED_TO_BACKGROUND;
        let pinned = after_three(planar_node(CornerPin, tilted, [Some(0), Some(1), Some(2)]));
        assert_eq!(pinned.passes.len(), 3);
        assert_eq!(
            laid(&pinned.passes[2]),
            ([Reads::Source, Reads::Pass(0), Reads::Pass(1)], limited_by)
        );
        // Laid as it lies over nothing, with no mask, it is its foreground; over a picture, a
        // pass all the same.
        let lying = after_three(planar_node(CornerPin, identity, [None, Some(1), None]));
        assert_eq!(lying.passes, vec![Pass::Fill([0.2, 0.4, 0.6, 1.0])]);
        assert_eq!(lying.output, Reads::Pass(0));
        let over = after_three(planar_node(CornerPin, identity, [Some(0), Some(1), None]));
        assert_eq!(over.passes.len(), 2);
        // Every one of them is drawn as what it is.
        for operation in [Idle, Steady, CornerPin] {
            assert!(is_drawn(&planar_node(
                operation,
                shift,
                [Some(0), Some(1), None]
            )));
        }
    }
}
