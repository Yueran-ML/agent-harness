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
//! - a Background is a texture cleared to its colour (`Pass::Fill`);
//! - a Merge and a Transform are one pass each of `fusion.wgsl` (`Pass::Lay`): the foreground
//!   moved, made larger or smaller and turned (and, of a Transform, flipped), read between its
//!   texels and smoothed at its edges as a layer is, then merged with the background by the
//!   mathematics of [`crate::merge`], step by step. A Transform is its picture laid over
//!   nothing by a merge that changes nothing;
//! - a node that changes nothing is no pass: its picture is that of its input. So is a
//!   Transform as it is at first, a Merge whose foreground gives no picture or whose Blend is
//!   0 (its background), and a Merge as it is at first over no background (its foreground). A
//!   node whose kind is not known here hands on the picture of its first input.
//!
//! A composition whose `MediaOut` shows its `MediaIn`, as the one that a clip has at first
//! does, has no pass at all: its layer is drawn as that of a clip without a composition, to
//! the bit. One of `n` nodes costs `n` passes at the most, and one more for its source.
//!
//! The textures are those of the neighbourhoods of grades ([`crate::neighbourhood`]), kept from
//! frame to frame by their size: a pass draws into one that no pass after it reads
//! (`Plan::slots`), so that a chain of nodes takes two or three, however long it is.

use columbina_model::fusion::{self as model, Fused, FusedKind, Merged, Transformed};

use crate::geometry::{fit, raster_scale, samples, Affine};
use crate::gpu::Gpu;
use crate::grade::layout;
use crate::merge::{Operator, Settings};
use crate::Source;

/// The shader of a pass that lays a foreground over a background.
const SHADER: &str = include_str!("fusion.wgsl");

/// The format of the pictures of a composition: that of the canvas, half floats, which keep
/// what is above 1 and below 0.
pub(crate) const FORMAT: wgpu::TextureFormat = crate::WORKING_FORMAT;

/// How many values the uniform of `fusion.wgsl` has.
pub(crate) const PARAMS: usize = 20;

/// The bytes of the uniform of `fusion.wgsl`.
const PARAMS_SIZE: u64 = 80;

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
    /// A colour over the whole picture, premultiplied: a Background.
    Fill([f32; 4]),
    /// A foreground moved and merged with a background (`fusion.wgsl`): a Merge, or a
    /// Transform, whose background is nothing. The foreground is never nothing.
    Lay {
        background: Reads,
        foreground: Reads,
        /// The uniform of `fusion.wgsl`.
        uniform: [f32; PARAMS],
    },
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
            || self.passes.iter().any(|pass| match pass {
                Pass::Fill(_) => false,
                Pass::Lay {
                    background,
                    foreground,
                    ..
                } => [*background, *foreground].contains(&Reads::Source),
            })
    }

    /// Whether a pass of it is one of `fusion.wgsl`, which needs its pipeline.
    pub(crate) fn lays(&self) -> bool {
        self.passes
            .iter()
            .any(|pass| matches!(pass, Pass::Lay { .. }))
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
            if let Pass::Lay {
                background,
                foreground,
                ..
            } = pass
            {
                read(*background, by);
                read(*foreground, by);
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
            let Pass::Lay {
                background,
                foreground,
                ..
            } = pass
            else {
                continue;
            };
            // What this pass is the last to read is free for the passes after it.
            let both = if background == foreground {
                [*background, Reads::Nothing]
            } else {
                [*background, *foreground]
            };
            for reads in both {
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
        let reads = match &node.kind {
            FusedKind::MediaIn if source => Reads::Source,
            FusedKind::MediaIn => Reads::Nothing,
            FusedKind::Background { rgba } => match filled(*rgba) {
                Some(colour) => {
                    passes.push(Pass::Fill(colour));
                    Reads::Pass(passes.len() - 1)
                }
                None => Reads::Nothing,
            },
            FusedKind::Transform { input, settings } => {
                transformed(read(*input), settings, size, luma, &mut passes)
            }
            FusedKind::Merge {
                background,
                foreground,
                settings,
            } => merged(
                read(*background),
                read(*foreground),
                settings,
                size,
                luma,
                &mut passes,
            ),
            // A kind that is not known here: the picture of its first input, as a node that is
            // passed through hands it on.
            other => read(other.first_input()),
        };
        made.push(reads);
    }
    let output = composition
        .output
        .and_then(|at| made.get(at).copied())
        .unwrap_or_default();
    needed(passes, output)
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

/// The picture of a Transform of `input`: none where it is given none or leaves nothing of it,
/// that of its input where it changes nothing, and else a pass that lays it over nothing.
fn transformed(
    input: Reads,
    settings: &Transformed,
    size: (u32, u32),
    luma: [f32; 3],
    passes: &mut Vec<Pass>,
) -> Reads {
    let scale = settings.size;
    if input == Reads::Nothing || scale.is_nan() || scale <= 0.0 {
        return Reads::Nothing;
    }
    let flips = (settings.flip_horizontal, settings.flip_vertical);
    if flips == (false, false) && lies_as_it_is(settings.center, scale, settings.angle) {
        return input;
    }
    let map = reading(size, settings.center, scale, settings.angle, flips);
    passes.push(Pass::Lay {
        background: Reads::Nothing,
        foreground: input,
        uniform: uniform(&map, scale, &Settings::default(), luma),
    });
    Reads::Pass(passes.len() - 1)
}

/// The picture of a Merge: its background where its foreground gives nothing, or nothing is
/// left of it, or its Blend is 0; its foreground where that lies as it is over no background
/// and the merge is the plain over of it; and else a pass.
fn merged(
    background: Reads,
    foreground: Reads,
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
    if background == Reads::Nothing && placed && adds_as_it_is(&held) {
        return foreground;
    }
    let map = reading(size, settings.center, scale, settings.angle, (false, false));
    passes.push(Pass::Lay {
        background,
        foreground,
        uniform: uniform(&map, scale, &held, luma),
    });
    Reads::Pass(passes.len() - 1)
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
        if let Some(Pass::Lay {
            background,
            foreground,
            ..
        }) = passes.get(at)
        {
            next.extend([*background, *foreground]);
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
        .map(|(pass, _)| match pass {
            Pass::Lay {
                background,
                foreground,
                uniform,
            } => Pass::Lay {
                background: renumbered(background),
                foreground: renumbered(foreground),
                uniform,
            },
            fill @ Pass::Fill(_) => fill,
        })
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

/// The pipeline of the passes that lay, of the shader `source` ([`shader`]): it writes each
/// pixel as it makes it, into a texture of [`FORMAT`].
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

/// What the passes that lay are drawn with on a card, but for their pipeline, which the
/// compositor keeps with its others: made the first time a composition has such a pass, so
/// that a compositor that draws none has nothing of it.
pub(crate) struct Card {
    /// What binds the uniform of a pass, its background and its foreground.
    pub layout: wgpu::BindGroupLayout,
    /// A texel of nothing: the background of a pass that has none.
    nothing: wgpu::TextureView,
    /// The uniforms of the passes of a frame, a slot each.
    uniform: wgpu::Buffer,
}

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
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("columbina composition"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(PARAMS_SIZE),
                    },
                    count: None,
                },
                picture(1),
                picture(2),
            ],
        });
        let nothing = device.create_texture(&wgpu::TextureDescriptor {
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
        Card {
            layout,
            nothing: nothing.create_view(&wgpu::TextureViewDescriptor::default()),
            uniform: uniform_buffer(device, u64::from(slot) * 4),
        }
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
    /// into `target`, by `pipeline` and the uniform at `offset`.
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
        target: &wgpu::Texture,
    ) {
        let whole = wgpu::TextureViewDescriptor::default();
        let under =
            background.map_or_else(|| self.nothing.clone(), |under| under.create_view(&whole));
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("columbina composition"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &self.uniform,
                        offset: 0,
                        size: wgpu::BufferSize::new(PARAMS_SIZE),
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&under),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&foreground.create_view(&whole)),
                },
            ],
        });
        let view = target.create_view(&whole);
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
        pass.set_bind_group(0, &bind, &[offset]);
        pass.draw(0..3, 0..1);
    }
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
        FusedKind::Background { rgba }
    }

    fn planned(nodes: Vec<FusedNode>, output: Option<usize>) -> Plan {
        plan(&Fused { nodes, output }, SIZE, REC709_LUMA, true)
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
