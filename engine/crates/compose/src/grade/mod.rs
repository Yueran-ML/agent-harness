//! Grades on the card: the graph of nodes of a grade (`columbina_model::grade`), at a frame,
//! made into an operation of colour ([`crate::ColorOp`]) of a [`Stage`], as the Color page of
//! `DaVinci Resolve` does it (Reference Manual of Resolve 20, Chapters 131–147).
//!
//! # The operation of a grade
//!
//! A grade is one operation: its body is the WGSL of its graph, a few lines for each node in an
//! order in which each comes after what it reads, and its library the maths of the nodes
//! (`corrector.wgsl`). The **shape** of the graph (the kinds of its nodes, its links, which nodes
//! are enabled, the composite mode of a layer mixer, which LUTs there are) makes the body; the
//! **settings** of its nodes, each [`columbina_model::grade::Param`] taken at the frame, are its
//! parameters. So a slider dragged or a keyframe played changes only numbers, and the shader is
//! the same; a node added, removed, linked, enabled or disabled makes another. The points of the
//! curves are parameters too (a block says where they are and how many), so a curve can be edited
//! without a shader being made again.
//!
//! A grade that is not complete ([`Grade::is_complete`]: a node not linked so that the picture
//! runs through it to the output) changes nothing, as `DaVinci` disables it (pp. 3312, 3319);
//! nor does a grade without nodes. Both are a stage of no operations, which costs nothing.
//!
//! # Nodes
//!
//! - **Corrector** (p. 3311): its palettes, in the order of p. 3331, on its RGB input; then its
//!   key: the key at its key input (if one is linked) through the Key Input settings, or a full
//!   key, through the Key Output settings (p. 3362); the node gives its input mixed with what it
//!   made by that key. Key Output Gain is so "the strength of that node's contribution" (p. 3362):
//!   0 leaves the picture as it came. A node has no key of its own yet (qualifiers and windows
//!   are 2B), so Matte and Mask modes of its key input both give the key input as it is; they
//!   differ once the node has a key of its own to meet or add to.
//! - **Disabled** nodes give their first input as it is (a key mixer, the key of its first input),
//!   with a full key.
//! - **Parallel mixer** (pp. 3334–3335): "combined equally": the mean of its inputs. A node
//!   before it with a lower Key Output Gain gives less of its change to the mean (p. 3339).
//! - **Layer mixer** (pp. 3336–3339): the first input at the back, and each lower input laid
//!   over what is above it, "the lowest input on top", in the mixer's composite mode, by the key
//!   of the node that gives it: so a lower Key Output Gain lets more of the layers above through
//!   (p. 3339). What is laid is what that node made before its own key mix (its key does that
//!   here instead). The composite modes are those of the timeline (pp. 1055–1058), with the
//!   formulas of the W3C's Compositing and Blending (separable modes; Hue, Saturation, Color and
//!   Luminosity with its `SetLum`, `SetSat` and `ClipColor`, luma by the weights of the working
//!   space), Linear Light `b + 2f - 1`, Vivid Light by Color Burn and Color Dodge of `2f`, Pin
//!   Light, Hard Mix `b + f >= 1`, Divide `b / f` (white where `f` is 0 and `b` above it),
//!   Subtract `b - f`, and Darker and Lighter Color choosing the colour of lower or higher luma.
//! - **Key mixer** (pp. 3358–3363): each input's key through its settings (invert, offset, gain),
//!   the first taken as it is and each next one met (Matte mode: multiplied) or added (Mask mode,
//!   the default: summed, at most 1), then the output's settings.
//! - A key through settings (both a corrector's and a key mixer's): inverted if set, then its
//!   offset `o` moves its contrast with white held (`k + o (1 - k)`, so that a solid key is not
//!   changed, p. 3362), then it is multiplied by the gain and kept within 0 and 1 (a gain of 2
//!   "increases the intensity of any part of the key output that's less than 100 percent white").
//!
//! # The corrector, in the order of p. 3331
//!
//! The maths of `DaVinci` is not published; where the manual does not give it, these are common,
//! well-behaved formulas chosen to behave as its pages describe. Every one is the identity, to
//! the bit, at the neutral settings of a new node (the tests hold it), which is why many are
//! written `x + change × (setting - neutral)`. Values are R'G'B' of the stage's space, 0 black and
//! 1 white, and may go beyond; `Y` is luma by the weights of the working space
//! ([`GradeOptions::luma`]).
//!
//! 1. **Shadows and Highlights** (pp. 3063–3064): add to R, G and B `0.25 (s (1 - smoothstep(0,
//!    0.5, Y)) + h smoothstep(0.5, 1, Y))`, `s` and `h` the settings over 100: shadows raised or
//!    lowered "while leaving the midtones alone", highlights brought down with "a smooth blend"
//!    into the midtones. `DaVinci`'s may work on the neighbourhood of a pixel; these are tone
//!    curves of the pixel alone.
//! 2. **Midtone Detail** (p. 3063): raised, "the contrast of regions of the image with high edge
//!    detail is raised to increase the perception of image sharpness"; lowered, "regions of the
//!    image with low amounts of detail are softened while areas of high-detail are left alone".
//!    It reads the pixels about each one, a neighbourhood (`color.rs`): the mean `m` of luma
//!    about the pixel and its spread `σ` (standard deviation), by a Gaussian of
//!    [`layout::MIDTONE_RADIUS`] pixels of a frame 1080 high, of the luma after Shadows and
//!    Highlights. With `Y` the luma of the pixel, `w = 4 t (1 - t)` (`t` = `Y` held within 0
//!    and 1: 1 at mid grey, 0 at black and white, so that the midtones are worked on) and
//!    `e = smoothstep(0, E, σ)` (how much the region is one of detail, `E` =
//!    [`layout::MIDTONE_EDGE`]), it adds to R, G and B `a G e w (Y - m)` for a setting `a`
//!    (over 100) above 0, a local contrast that makes the detail `1 + G` times as large at +100
//!    (`G` = [`layout::MIDTONE_GAIN`]), and `a (1 - e) w (Y - m)` below 0, which takes the
//!    regions of little detail towards their mean, all the way at -100, and leaves the
//!    detailed ones. `DaVinci` publishes none of it: the shape is the manual's, and the
//!    constants are guesses to be set against `DaVinci` itself. A setting of 0 that is not
//!    keyframed reads no neighbourhood, and is the grade of a pixel alone, to the bit; one that
//!    is keyframed reads one at every frame (the shape of the grade is the same), and does
//!    nothing at a frame where it is 0. On a pixel alone ([`evaluate`]) it is left out, as it
//!    leaves a picture of one colour; [`evaluate_picture`] takes it with the pixels about each.
//! 3. **Color Boost** (p. 3063, "a non-uniform saturation … sometimes referred to as a vibrance
//!    operation"): `x + (x - Y) b (1 - S)`, `b` the setting over 100 and `S` the saturation
//!    `(max - min) / max`: low saturation is raised (or lowered) more.
//! 4. **Temp and Tint** (p. 3063, "a specifically constrained Gain color balance"): gains of R, G
//!    and B of `2^(0.5u, 0, -0.5u)` for a Temp of `4000u` (warm towards orange, cool towards blue)
//!    and `2^(0.25v, -0.5v, 0.25v)` for a Tint of `100v` (towards magenta, or green), divided by
//!    what they make of the luma of a grey, so that a grey keeps its luma.
//! 5. **Lift, Gamma and Gain** with **Lum Mix** (pp. 3064–3068). On each of R, G and B,
//!    `gain · (x + lift (1 - x))^e` with `e = 2^(-2 gamma)` (of the sign of the base): Gain
//!    scales from black; Lift raises black and keeps white (p. 3066, "starts at black, and then
//!    falls off … to no influence at white"); Gamma bends the midtones and keeps black and white
//!    ("most Gamma contrast adjustments have a minimal effect on the black and white point").
//!    With `o` the result of the R, G and B settings, `Y` the luma of the input and `lgg_Y` the
//!    same formula with the Y settings, the step gives `o + m (lgg_Y(Y) - Y(o))` for a Lum Mix
//!    `m` (over 100): at 100 the luma of the result is what the Y settings make of the luma, so
//!    that a change of R, G or B apart is matched by "automatic adjustments … to the other two
//!    color channels in order to maintain constant Luma levels" (p. 3064); at 0 the channels are
//!    their own and "Y-only contrast adjustments are turned off" (p. 3064). The master ring of a
//!    wheel moves Y, R, G and B together ("the YRGB parameters located underneath all change
//!    together", p. 3061; Ctrl and a drag "adjusts YRGB contrast identically as if you were
//!    dragging that control's master ring"): then `lgg_Y(Y)` is `Y(o)` as near as the channels'
//!    formula is linear, so the move is done once, whatever Lum Mix is. The Y settings alone add
//!    the change of luma to R, G and B alike, so that "an increase in contrast results in
//!    perceptually diminished color saturation" (p. 3068). Offset has no Y (p. 3067).
//! 6. **Contrast and Pivot** (p. 3063). With "Use S-curve for contrast" on (the default, and
//!    [`GradeOptions::s_curve_contrast`]): `p (x/p)^k` below the pivot `p` and
//!    `1 - (1 - p)((1 - x)/(1 - p))^k` above, which has the slope `k` at the pivot and never
//!    clips ("the shadows and highlights … will not be clipped"); beyond black and white the
//!    values are kept. Off: `x + (x - p)(k - 1)`, linear.
//! 7. **Custom curves** (pp. 3111–3116): a cubic of Hermite through the points, with the slope
//!    at each point that of the line through its two neighbours (Catmull–Rom for points not
//!    evenly spaced), and at an end that of its segment, carried on along it beyond the ends
//!    (data beyond black and white is kept, p. 3119). It is not made monotone: p. 3114 warns
//!    that points too close "can introduce unwanted contouring … flattening or solarization",
//!    which a curve that may overshoot does, and `DaVinci`'s curves are smooth through their
//!    control points. Its intensity `I` mixes: `x + I (curve(x) - x)` (p. 3116). Ganged, the Y
//!    curve works on R, G and B, each by its own intensity (p. 3115). Not ganged, the R, G and B
//!    curves work on their channels with the luma kept as Lum Mix says (adding `m (Y(n) -
//!    Y(o))`, `n` from the luma-weighted mean of their changes, so that what is common to the
//!    three changes luma), and then the Y curve on luma alone (p. 3116,
//!    "increasing luma contrast results in a perceptual decrease of image saturation"), by Lum
//!    Mix as p. 3064 says.
//! 8. **Offset and the Log wheels** (pp. 3068–3074). Offset, which "is in fact processed as part
//!    of the Log controls" (p. 3073), adds `(offset - 25) / 100` to each channel, outside Lum Mix
//!    ("the Offset controls do not use the Lum Mix parameter", p. 3074). Then the wheels:
//!    Shadow, Midtone and Highlight each add their R, G and B (`0.25` at the edge of the wheel)
//!    over their zone of luma: below Low Range, between, and above High Range, with blends of a
//!    width of `0.1` that does not change ("the amount of overlap between each range cannot"
//!    be customized, p. 3073), so that the zones meet without a step. Pushed hard they
//!    solarize, as p. 3073 warns ("raising the shadows to be higher than the Midtones"). In a log
//!    working space (Q2/Q3) these zones are those of the log signal, as the manual means them.
//! 9. **Hue and Saturation** (p. 3064): Hue turns colour about the grey axis by
//!    `(hue - 50) / 50 × 180°` (Rodrigues' formula; red goes towards yellow as hue rises);
//!    Saturation `x + (x - Y)(sat/50 - 1)`: 0 grey, 100 twice.
//! 10. **LUT** (p. 3331 puts it after Hue and Saturation and before the HSL curves): an operation
//!     that a [`Luts`] makes of the LUT's name, put in place in the node. [`CardLuts`] reads the
//!     `.cube` files of the LUT crate and puts them on the card, cubes read trilinearly as
//!     `DaVinci`'s projects do at first (p. 152); a LUT that cannot be read leaves its node without
//!     one (its error kept for the Color page, p. 3413), as does [`NoLuts`] for every node.
//! 11. **HSL curves** (pp. 3121–3126), read at the hue, saturation and luma of the picture that
//!     came into the node ("always relative to the RGB input connected to that node", p. 3122):
//!     hue 0 red, 1/3 green, 2/3 blue; saturation `(max - min) / max`; luma held within 0
//!     and 1. The curves are the same cubics, flat beyond their ends, the curves of hue going round
//!     so that their ends meet smoothly (p. 3122). Hue vs Hue turns by `out × 180°`; Hue vs Sat,
//!     Lum vs Sat and Sat vs Sat multiply saturation by `1 + out` each; Hue vs Lum (times the
//!     saturation, as the hue of a grey is nothing) and Sat vs Lum multiply luma by `1 + out`,
//!     adding `out × Y` to R, G and B, so that black stays black and the colour is kept. The
//!     manual says that they "increase or decrease the lightness" (p. 3124), "darken specific
//!     hues to add richness and depth", and darken the most saturated parts as "highly saturated
//!     colors block more light from passing through the emulsion" of film (p. 3127): a light
//!     let through in a share, as the curves of saturation do, rather than a level added, which
//!     would raise black.
//! 12. **Soft Clip** (pp. 3117–3121): High at 100 (1023 of 10 bits, 1.0) and High Soft at 0
//!     clip nothing ("image data that you push above 1023 … is preserved"); High below 100 clips
//!     there, "even by a single digit"; High Soft `s` compresses from `H - w` towards `H`
//!     (`w = 0.5 s / 100`) along `H - w + w (1 - e^-((x - H + w)/w))`, of slope 1 at the knee,
//!     and Low and Low Soft do the same below.
//! 13. **Shot Match** (p. 3048), "applied invisibly, as the very last adjustment to the node …
//!     similar to a LUT": on each channel `S(gain · x^gamma + offset)`, the power of the sign of
//!     `x`, and `S` the S-curve of step 6 with the match's own contrast and pivot, whatever
//!     [`GradeOptions::s_curve_contrast`] says (`columbina_model::grade::ShotMatch`). A node
//!     without one, or with one at its neutral settings, leaves it out.
//! 14. **The Blur palette** (pp. 3454–3456): Blur, Sharpen and Mist of what the node made, before
//!     its key, through neighbourhoods of it ([`blur`]). At rest it reads none and has no
//!     parameters: the node is as it was, to the bit.
//!
//! The formulas are kept apart from the graph (the maths in `corrector.wgsl` and [`maths`],
//! their units in [`layout`]) so that they can be replaced: colour-managed working spaces (Q2,
//! Q3) will change the luma weights ([`GradeOptions::luma`]), and may move the Log wheels and
//! Contrast into a log space of their own.

pub mod blur;
pub mod curve;
mod graph;
pub mod layout;
mod luts;
pub mod maths;
mod windows;

use std::sync::Arc;

use columbina_model::grade::Grade;

use crate::neighbourhood::{self, Image};
use crate::{ColorOp, Stage};
/// The cache and the folders that [`CardLuts`] reads LUTs by, from the LUT crate.
pub use columbina_lut::{LutCache, Roots};
use graph::Plan;
pub use luts::{CardLuts, TEXTURE_BUDGET};
use maths::Constants;

/// The maths of the nodes, on the card.
const LIBRARY: &str = include_str!("corrector.wgsl");

/// The weights of R, G and B in the luma of BT.709.
pub const REC709_LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];

/// What a grade is made with besides its nodes: the space it works in. The options of all the
/// grades of one stage are to be the same (their maths is put into the shader once).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GradeOptions {
    /// The weights of R, G and B in luma, in the working space (BT.709 until the project says
    /// another, Q2/Q3).
    pub luma: [f32; 3],
    /// Whether Contrast is an S-curve, the project setting "Use S-curve for contrast", on by
    /// default (p. 3062).
    pub s_curve_contrast: bool,
}

impl Default for GradeOptions {
    fn default() -> Self {
        GradeOptions {
            luma: REC709_LUMA,
            s_curve_contrast: true,
        }
    }
}

impl GradeOptions {
    fn constants(&self) -> Constants {
        Constants {
            luma: self.luma,
            s_curve: self.s_curve_contrast,
        }
    }

    fn luma64(&self) -> [f64; 3] {
        self.luma.map(f64::from)
    }
}

/// The LUTs of nodes, by the name a node gives (a path, or a name in the library of LUTs): what
/// the LUT crate reads and puts on the card, as an operation of colour whose body changes `rgb`.
///
/// The body of the operation is put into the grade's, in the node, between its Hue and
/// Saturation and its HSL curves; its `$P` and `$T0`, `$T1`, … are made to read its own
/// parameters and textures there, and its library is put into the stage once. The same LUT
/// (the same body, library and kinds of textures) in another node, or another LUT of the same
/// kind, makes the same shader.
pub trait Luts {
    /// The operation of the LUT named `name`, or none where there is none (the node then has
    /// no LUT).
    fn lut(&self, name: &str) -> Option<ColorOp>;
}

/// No LUTs: a node that names one has none.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoLuts;

impl Luts for NoLuts {
    fn lut(&self, _name: &str) -> Option<ColorOp> {
        None
    }
}

/// The library of the maths of the nodes, with the constants of `options`.
#[must_use]
pub fn library(options: &GradeOptions) -> String {
    let [r, g, b] = options.luma;
    format!(
        "const GRADE_LUMA: vec3<f32> = vec3<f32>({r:?}, {g:?}, {b:?});\n\
         const GRADE_S_CURVE: bool = {};\n{LIBRARY}",
        options.s_curve_contrast
    )
}

/// The operations of `grade` at `frame` (of the clip, from its first frame at 0; of the
/// timeline, for its grade): an operation for the libraries of its LUTs, each once and doing
/// nothing, and the operation of the grade. None where it changes nothing.
#[must_use]
pub fn grade_ops(
    grade: &Grade,
    frame: f64,
    options: &GradeOptions,
    luts: &dyn Luts,
) -> Vec<ColorOp> {
    Prepared::new(grade, options, luts).ops_at(frame).collect()
}

/// A grade made ready for its frames: the code of its operations (its graph, its LUTs and the
/// library of `options`) made once, so that each frame takes only its parameters
/// ([`Prepared::ops_at`]), as [`grade_ops`] would give them. It keeps the grade it was made of
/// ([`Prepared::grade`]), to be made again once the grade is another, or its LUTs are.
#[derive(Clone, Debug)]
pub struct Prepared {
    grade: Grade,
    luma: [f64; 3],
    plan: Option<Plan>,
    /// The operations of the libraries of its LUTs, and last that of the grade, its parameters
    /// left empty; none where it changes nothing.
    ops: Vec<ColorOp>,
}

impl Prepared {
    /// `grade` made ready, in the space of `options`, with the LUTs that `luts` has.
    #[must_use]
    pub fn new(grade: &Grade, options: &GradeOptions, luts: &dyn Luts) -> Prepared {
        let plan = Plan::of(grade, luts);
        let mut ops = Vec::new();
        if let Some(plan) = &plan {
            let mut textures = Vec::with_capacity(plan.textures);
            for lut in plan.luts() {
                textures.extend(lut.op.textures.iter().cloned());
                let shared = ops.iter().any(|op: &ColorOp| op.library == lut.op.library);
                if !lut.op.library.is_empty() && !shared {
                    ops.push(ColorOp {
                        library: Arc::clone(&lut.op.library),
                        ..ColorOp::default()
                    });
                }
            }
            if plan.spatial() {
                ops.push(ColorOp {
                    library: windows::LIBRARY.into(),
                    ..ColorOp::default()
                });
            }
            if plan.blurred() {
                ops.push(ColorOp {
                    library: blur::LIBRARY.into(),
                    ..ColorOp::default()
                });
            }
            ops.push(ColorOp {
                body: plan.body().into(),
                library: library(options).into(),
                spatial: plan.spatial(),
                params: Vec::new(),
                textures,
                neighbourhoods: plan.neighbourhoods(grade, 0.0),
            });
        }
        Prepared {
            grade: grade.clone(),
            luma: options.luma64(),
            plan,
            ops,
        }
    }

    /// The grade that it was made of.
    #[must_use]
    pub fn grade(&self) -> &Grade {
        &self.grade
    }

    /// Whether it changes nothing (a grade of no nodes, or not complete): it has no operations.
    #[must_use]
    pub fn is_identity(&self) -> bool {
        self.ops.is_empty()
    }

    /// Its operations at `frame`: the code as it was made, and the parameters of the frame and
    /// the filters of its neighbourhoods there.
    pub fn ops_at(&self, frame: f64) -> impl Iterator<Item = ColorOp> + '_ {
        let last = self.ops.len().saturating_sub(1);
        self.ops.iter().enumerate().map(move |(i, op)| {
            let mut op = op.clone();
            if i == last {
                if let Some(plan) = &self.plan {
                    op.params = plan.params(&self.grade, frame, self.luma);
                    for (kept, filter) in op
                        .neighbourhoods
                        .iter_mut()
                        .zip(plan.filters(&self.grade, frame))
                    {
                        kept.filter = filter;
                    }
                }
            }
            op
        })
    }

    /// Its operations at `frame` as [`Prepared::ops_at`] gives them, but showing what `look`
    /// asks of one of its nodes in place of its picture; none where the node is not one that
    /// the grade runs through (it is not there, or the grade changes nothing). The code of the
    /// grade's operation is another, made once for the node and what is shown; its parameters
    /// and neighbourhoods are the same.
    #[must_use]
    pub fn ops_showing(&self, frame: f64, look: &Look) -> Option<Vec<ColorOp>> {
        let plan = self.plan.as_ref()?;
        let node = self.grade.position(&look.node)?;
        let step = plan.step_of(node)?;
        let mut ops: Vec<ColorOp> = self.ops_at(frame).collect();
        let last = ops.last_mut()?;
        last.body = plan.body_showing(Some((step, look.show))).into();
        Some(ops)
    }
}

/// What the viewer shows of a node of a grade in place of the picture, as the Highlight modes of
/// the viewer show the key of the node chosen (pp. 3170–3171).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Look {
    /// The node, by its id.
    pub node: String,
    /// What is shown of it.
    pub show: Show,
}

/// What is shown of a node (pp. 3170–3171).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Show {
    /// Highlight (Shift-H): the picture of the grade where the node's key is, and a flat grey
    /// ([`layout::HIGHLIGHT_GREY`]) where it is not, mixed by the key. A node with no key of its
    /// own (a corrector without a qualifier or a key linked into it, a mixer of pictures) is
    /// shown alone instead, its picture in place of the grade's ("if you turn on Highlight
    /// while you're selecting a node with no key, then only the currently selected node is
    /// output", p. 3171).
    Highlight,
    /// Highlight BW (Alt-Shift-H): the key of the node, white where it is full and black where
    /// it is none, a "high-contrast" highlight.
    Matte,
}

/// The stage of `grade` at `frame` (see [`grade_ops`]).
#[must_use]
pub fn stage_of(grade: &Grade, frame: f64, options: &GradeOptions, luts: &dyn Luts) -> Stage {
    Stage {
        ops: grade_ops(grade, frame, options, luts),
    }
}

/// The stage of grades done one after another, each at its own frame: for a layer, the pre-clip
/// grade of its group, its own and the post-clip grade of its group (p. 3330).
#[must_use]
pub fn stage_of_grades(grades: &[(&Grade, f64)], options: &GradeOptions, luts: &dyn Luts) -> Stage {
    Stage {
        ops: grades
            .iter()
            .flat_map(|(grade, frame)| grade_ops(grade, *frame, options, luts))
            .collect(),
    }
}

/// What `grade` makes of `rgb` at `frame`, on the processor: the reference that the card is held
/// to. LUTs are left out, as with [`NoLuts`]; and Midtone Detail, which has no pixels about
/// `rgb` to work on (it leaves a picture of one colour as it is), see [`evaluate_picture`].
/// Power Windows are ignored by this positionless API, not sampled at an invented position.
/// Use [`evaluate_picture`] for spatial keys; LUT export excludes the whole windowed node.
#[must_use]
pub fn evaluate(grade: &Grade, frame: f64, options: &GradeOptions, rgb: [f32; 3]) -> [f32; 3] {
    evaluate_with(grade, frame, options, &NoLuts, &|_, rgb| rgb, rgb)
}

/// What `grade` makes of `rgb` at `frame` on the processor, with the LUTs that `luts` has, each
/// done by `lut` (given its name and a colour). Power Windows are ignored, as in [`evaluate`].
#[must_use]
pub fn evaluate_with(
    grade: &Grade,
    frame: f64,
    options: &GradeOptions,
    luts: &dyn Luts,
    lut: &dyn Fn(&str, [f32; 3]) -> [f32; 3],
    rgb: [f32; 3],
) -> [f32; 3] {
    let Some(plan) = Plan::of(grade, luts) else {
        return rgb;
    };
    let params = plan.params(grade, frame, options.luma64());
    plan.run(&options.constants(), &params, rgb, lut)
}

/// What `grade` makes of a picture at `frame` on the processor, Midtone Detail and all, as the
/// compositor grades a layer drawn at the size of its picture: `pixels`, R'G'B' of `width` ×
/// `height`, row after row, `units` of them to a pixel of a frame 1080 high
/// ([`neighbourhood::units`]); with the LUTs that `luts` has, each done by `lut`.
///
/// The colour that each neighbourhood filters is kept in half floats, and filtered as the card
/// does it ([`neighbourhood::filtered`]), so that only the roundings of the card are left
/// between the two.
///
/// # Panics
/// `pixels` is not of `width` × `height`.
#[must_use]
#[allow(
    clippy::too_many_arguments,
    clippy::cast_precision_loss,
    reason = "a grade, its frame and options, its LUTs and a picture; places of pixels"
)]
pub fn evaluate_picture(
    grade: &Grade,
    frame: f64,
    options: &GradeOptions,
    luts: &dyn Luts,
    lut: &dyn Fn(&str, [f32; 3]) -> [f32; 3],
    (width, height): (u32, u32),
    pixels: &[[f32; 3]],
    units: f32,
) -> Vec<[f32; 3]> {
    evaluate_picture_showing(
        grade,
        frame,
        options,
        luts,
        lut,
        (width, height),
        pixels,
        units,
        None,
    )
}

/// What [`evaluate_picture`] gives, but showing what `look` asks of one of the grade's nodes in
/// place of its picture where there is one (see [`Prepared::ops_showing`]); a node that the grade
/// does not run through shows the picture of the grade.
///
/// # Panics
/// `pixels` is not of `width` × `height`.
#[must_use]
#[allow(
    clippy::too_many_arguments,
    clippy::cast_precision_loss,
    reason = "a grade, its frame and options, its LUTs, a picture and a look; places of pixels"
)]
pub fn evaluate_picture_showing(
    grade: &Grade,
    frame: f64,
    options: &GradeOptions,
    luts: &dyn Luts,
    lut: &dyn Fn(&str, [f32; 3]) -> [f32; 3],
    (width, height): (u32, u32),
    pixels: &[[f32; 3]],
    units: f32,
    look: Option<&Look>,
) -> Vec<[f32; 3]> {
    assert_eq!(pixels.len(), width as usize * height as usize);
    let Some(plan) = Plan::of(grade, luts) else {
        return pixels.to_vec();
    };
    let show = look.and_then(|look| {
        let step = plan.step_of(grade.position(&look.node)?)?;
        Some((step, look.show))
    });
    let params = plan.params(grade, frame, options.luma64());
    let constants = options.constants();
    let position = |at: usize| windows::Position {
        uv: [
            ((at % width as usize) as f32 + 0.5) / width as f32,
            ((at / width as usize) as f32 + 0.5) / height as f32,
        ],
        size: [width as f32, height as f32],
    };
    let mut around: Vec<Vec<[f32; 3]>> = vec![Vec::new(); pixels.len()];
    for (number, filter) in plan.filters(grade, frame).into_iter().enumerate() {
        let source = Image {
            width,
            height,
            pixels: pixels
                .iter()
                .zip(&around)
                .enumerate()
                .map(|(at, (rgb, around))| {
                    let [r, g, b] = plan.hood_source(
                        number,
                        &constants,
                        &params,
                        *rgb,
                        lut,
                        around,
                        position(at),
                    );
                    [r, g, b, 1.0].map(neighbourhood::half)
                })
                .collect(),
        };
        let filtered = neighbourhood::filtered(&source, filter, units);
        for (at, around) in around.iter_mut().enumerate() {
            let (x, y) = (at % width as usize, at / width as usize);
            let place = [
                (x as f32 + 0.5) / width as f32,
                (y as f32 + 0.5) / height as f32,
            ];
            around.push(neighbourhood::colour_at(&filtered, place));
        }
    }
    pixels
        .iter()
        .zip(&around)
        .enumerate()
        .map(|(at, (rgb, around))| {
            plan.run_showing(
                &constants,
                &params,
                *rgb,
                lut,
                Some(around),
                show,
                Some(position(at)),
            )
        })
        .collect()
}

/// What a grade gives of `rgb` at `frame` on the processor, showing what `look` asks of one of
/// its nodes (see [`Prepared::ops_showing`]), with the LUTs that `luts` has, each done by `lut`:
/// a pixel alone, a picture of one colour, as [`evaluate_with`] takes it.
#[must_use]
pub fn evaluate_showing(
    grade: &Grade,
    frame: f64,
    options: &GradeOptions,
    luts: &dyn Luts,
    lut: &dyn Fn(&str, [f32; 3]) -> [f32; 3],
    rgb: [f32; 3],
    look: &Look,
) -> [f32; 3] {
    let Some(plan) = Plan::of(grade, luts) else {
        return rgb;
    };
    let show = grade
        .position(&look.node)
        .and_then(|node| plan.step_of(node))
        .map(|step| (step, look.show));
    let params = plan.params(grade, frame, options.luma64());
    plan.run_showing(&options.constants(), &params, rgb, lut, None, show, None)
}

#[cfg(test)]
mod tests;
