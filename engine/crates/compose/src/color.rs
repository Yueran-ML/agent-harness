//! Colour on the card: how the Y'CbCr of a picture is made R'G'B' ([`YuvCoding`]), and the stages
//! of colour that a frame goes through ([`Stage`]), in the order of `DaVinci Resolve` (Reference
//! Manual of Resolve 20, pp. 3329–3331):
//!
//! ```text
//! each layer, from the bottom up:
//!   planes of Y'CbCr ─▶ R'G'B' (its matrix and range)          decode, in the layer's shader
//!                    ─▶ input transform     LayerColor::input   ┐ a point each, fused into the
//!                    ─▶ grade               LayerColor::grade   ┘ same shader, after decoding
//!                    ─▶ transform, opacity, laid over what is under it
//! the canvas, Rgba16Float (the timeline's working space), preview and export alike:
//!   ─▶ timeline grade     Scene::grade      a pass over the canvas
//!   ─▶ subtitles          (a preview only)
//!   ─▶ output transform   Scene::output     a pass over the canvas
//!   ─▶ out: the texture of a preview (Rgba16Float, which a view draws into its own format),
//!           or NV12/P010 for an encoder (BT.709 of limited range, dithered in 8 bits)
//! ```
//!
//! # Design
//!
//! A stage is a list of [`ColorOp`]s: operations on the colour of one pixel at a time, each a
//! piece of WGSL that changes `rgb` (a `var` of `vec3<f32>`), with the parameters and textures
//! it reads. The compositor joins the operations of the input transform and of the grade of a
//! layer into one function, which the shader of the layer calls on each pixel once it is
//! decoded (and, for a picture drawn smaller, once its points are averaged): so a grade is not a
//! pass of its own, and a layer graded is drawn at the cost of one that is not, but for the
//! operations. The timeline grade and the output transform are passes over the canvas, from it
//! into a second texture of the target and back, each made of its operations the same way; and
//! a stage with no operations is no pass at all, so that a frame without colour is composed as
//! it was in phase 1 (the tests hold it to frames kept from then, `tests/golden.rs`).
//!
//! Shaders are made for each list of operations, by the text of their code: two layers graded
//! alike share a pipeline, whatever their parameters, which are read from a buffer at the
//! layer's own place. So the parameters of a grade can change from frame to frame (keyframes,
//! a slider dragged) without a shader being made again; only another shape of graph does that.
//!
//! What the later tasks of phase 2 plug in here:
//! - **The corrector (T9)** turns a node graph into one operation, or one per node, in the order
//!   of the manual (p. 3331); its parameters are uniforms in `params`, and parallel and layer
//!   mixers are pointwise too (each branch reads the same `rgb`). Operations that need pixels
//!   about the one they make (Midtone Detail; blur, sharpen and mist, T25) read them through
//!   [`Neighbourhood`]s, below.
//! - **LUTs (T10)** are textures of an operation (`textures`, a 3D one for a cube, read with
//!   `textureLoad` for a tetrahedral interpolation of its own, or sampled), with the code that
//!   reads them as its `library`, shared by every operation that names the same.
//! - **Scopes (T11)** read the texture of a preview ([`crate::Target::texture`]): the frame after
//!   the output transform, as it is exported, before a view maps it into its window.
//! - **Colour management (T6, T7, T12)** are operations of the input transform (the space of the
//!   media into the working space) and of the output transform ([`crate::ocio`], [`crate::tone`]).
//!   Decoding holds R'G'B' to 0..1, as phase 1 did, unless [`LayerColor::unclamped`] (a managed
//!   project); the convert of an export takes its matrix from the output space
//!   ([`crate::Target::set_matrix`]).
//!
//! # Neighbourhoods
//!
//! An operation may read, at each pixel, a filtered picture of what came into it: each of its
//! [`ColorOp::neighbourhoods`] is the colour that its `body` makes of the operation's input (the
//! picture as the operations before it left it), taken through its [`Filter`], and read in the
//! operation's code as `$N0`, `$N1`, …: the filtered colour at the pixel. A neighbourhood's body
//! may read those before it (`$N` of a lower number), as a node of a grade may come after
//! another that reads its own.
//!
//! A layer whose operations have neighbourhoods is drawn so (`compose.rs`): for each
//! neighbourhood, in order, the layer is drawn into a texture of its own of the size of its
//! picture (decoded, its input transform, and the operations before the one that reads it, and
//! then the neighbourhood's body), alone and as it is shown, with no transform; that texture is
//! filtered by two passes, across and down (a separable Gaussian or box, `neighbourhood.wgsl`),
//! into a smaller one where the filter is wide enough to lose nothing by it
//! ([`crate::neighbourhood::reduction`]); and then the layer is drawn onto the canvas as any
//! other, in one pass, its operations reading the filtered textures where each of its pixels
//! lies in its picture. So what a neighbourhood needs of the operations before it is done on the
//! card twice, once for the filter and once on the canvas, and the layer is graded as precisely
//! as one without a neighbourhood: only what is filtered goes through textures of half floats.
//!
//! The texture is of the picture at its own size, whatever the size it is drawn at, as `DaVinci`
//! takes a picture at "the full native resolution of the source media" (p. 284). How far a filter
//! reaches is said in pixels of a frame [`REFERENCE_HEIGHT`] high, the frame being the timeline
//! as the layer is fitted into it: so a blur looks the same in a preview of any size and in an
//! export, and on media of any resolution, as `DaVinci`'s effects "readjust themselves to match
//! the sizing of each new timeline resolution" (p. 283). A layer whose operations have none (a
//! grade whose Midtone Detail is 0 and not keyframed has none) is drawn in its one pass as
//! before, to the bit.
//!
//! The timeline grade and the output transform read neighbourhoods of the canvas the same way:
//! the canvas is drawn by the operations before the one that reads it into a texture, and that
//! is filtered.

use std::fmt::Write as _;
use std::sync::Arc;

use columbina_media::{ColorFacts, NativePicture};
pub use columbina_media::{ColorRange, YuvMatrix};

/// How the planes of a picture are made R'G'B': its matrix, and the range of its codes (one
/// that is not said is taken as limited, the range of video).
///
/// The matrix and range are those of the media crate, which says them of a file
/// ([`YuvCoding::of_facts`]) and of each picture of its own sampling
/// ([`YuvCoding::of_native`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YuvCoding {
    pub matrix: YuvMatrix,
    pub range: ColorRange,
}

impl YuvCoding {
    /// The coding of the pictures of a file of `width` × `height` whose colours are `color`,
    /// as the file says or players take it ([`YuvMatrix::of`], [`columbina_media::levels`]).
    #[must_use]
    pub fn of_facts(color: &ColorFacts, width: u32, height: u32) -> YuvCoding {
        YuvCoding {
            matrix: YuvMatrix::of(color, width, height),
            range: columbina_media::levels(color),
        }
    }

    /// The coding of a picture of its own sampling, which keeps the codes of its file.
    #[must_use]
    pub fn of_native(picture: &NativePicture) -> YuvCoding {
        YuvCoding {
            matrix: picture.matrix,
            range: picture.range,
        }
    }

    /// What Cr and Cb (from -0.5 to 0.5) are multiplied by for R', G' and B': R' from Cr, G' from
    /// Cb, G' from Cr (both taken away), B' from Cb.
    #[must_use]
    pub fn coefficients(self) -> [f64; 4] {
        let (kr, kb) = self.matrix.coefficients();
        let kg = 1.0 - kr - kb;
        [
            2.0 * (1.0 - kr),
            2.0 * kb * (1.0 - kb) / kg,
            2.0 * kr * (1.0 - kr) / kg,
            2.0 * (1.0 - kb),
        ]
    }

    /// The codes of black, of the range of luma from black to white, of no colour, and of the
    /// range of chroma, in codes of 8 bits or of 10 (`ten`).
    #[must_use]
    pub fn levels(self, ten: bool) -> [f64; 4] {
        match (self.range == ColorRange::Full, ten) {
            (false, false) => [16.0, 219.0, 128.0, 224.0],
            (false, true) => [64.0, 876.0, 512.0, 896.0],
            (true, false) => [0.0, 255.0, 128.0, 255.0],
            (true, true) => [0.0, 1023.0, 512.0, 1023.0],
        }
    }
}

/// What a texture of an operation is, for its binding.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextureKind {
    /// `texture_1d<f32>`: a 1D LUT, a curve.
    D1,
    /// `texture_2d<f32>`.
    D2,
    /// `texture_3d<f32>`: a 3D LUT.
    D3,
}

impl TextureKind {
    fn wgsl(self) -> &'static str {
        match self {
            TextureKind::D1 => "texture_1d<f32>",
            TextureKind::D2 => "texture_2d<f32>",
            TextureKind::D3 => "texture_3d<f32>",
        }
    }

    pub(crate) fn dimension(self) -> wgpu::TextureViewDimension {
        match self {
            TextureKind::D1 => wgpu::TextureViewDimension::D1,
            TextureKind::D2 => wgpu::TextureViewDimension::D2,
            TextureKind::D3 => wgpu::TextureViewDimension::D3,
        }
    }
}

/// A texture that an operation reads, of the compositor's card, of floats that can be filtered.
#[derive(Clone, Debug)]
pub struct OpTexture {
    pub kind: TextureKind,
    pub view: wgpu::TextureView,
}

/// The rows of the frame that the reach of a [`Filter`] is said in: a filter of a standard
/// deviation of 1 is of a pixel of the timeline where it is 1080 high, of 2 pixels where it is
/// 2160 high, and of as many pixels of a picture as make a pixel of such a timeline.
pub const REFERENCE_HEIGHT: f32 = 1080.0;

/// How the pixels about each one are taken: two passes, across and down, each reaching as far
/// as its own value (`[across, down]`), in pixels of a frame [`REFERENCE_HEIGHT`] high. The
/// pixels beyond the edges are those at the edges.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Filter {
    /// A Gaussian of these standard deviations.
    Gaussian { sigma: [f32; 2] },
    /// The mean of the pixels whose centres are within these distances of the pixel's.
    Box { radius: [f32; 2] },
}

/// What an operation reads of the pixels about each one: the colour that `body` makes of the
/// operation's input, through `filter` (see the module's documentation). `body` is WGSL as the
/// operation's is, reading the operation's parameters and textures, and the neighbourhoods
/// before it.
#[derive(Clone, Debug)]
pub struct Neighbourhood {
    pub body: Arc<str>,
    pub filter: Filter,
}

/// An operation on the colour of one pixel at a time.
///
/// `body` is WGSL: statements that change `rgb`, a `var` of `vec3<f32>` (R'G'B' or RGB of the
/// space of the stage, 0 to 1 for black to white, and beyond). In it:
/// - `param($P + k)` is the `k`th of the operation's `params` (a `vec4<f32>`);
/// - `$T0`, `$T1`, … are its `textures`, and `$S` a linear sampler that clamps to the edges;
/// - `$N0`, `$N1`, … are its `neighbourhoods`, each a `vec3<f32>`: the colour filtered, at the
///   pixel.
///
/// `library` is WGSL put once before every operation, for functions that the bodies call: the
/// same library, by its text, is put once however many operations name it. Names in it are to
/// be of the operation's kind (`lut_tetrahedral`), so as not to meet others.
///
/// The text of `body` and `library`, the kinds of `textures`, and how many `neighbourhoods` there
/// are and their bodies make the shader: keep them the same from frame to frame, and change only
/// `params`, the textures themselves, and the filters.
#[derive(Clone, Debug, Default)]
pub struct ColorOp {
    pub body: Arc<str>,
    pub library: Arc<str>,
    pub params: Vec<[f32; 4]>,
    pub textures: Vec<OpTexture>,
    pub neighbourhoods: Vec<Neighbourhood>,
}

/// A stage of colour: its operations, in the order they are done. None is the identity: nothing
/// is done, and nothing costs anything.
#[derive(Clone, Debug, Default)]
pub struct Stage {
    pub ops: Vec<ColorOp>,
}

impl Stage {
    /// Whether it does nothing.
    #[must_use]
    pub fn is_identity(&self) -> bool {
        self.ops.is_empty()
    }

    /// Whether an operation of it reads the pixels about each one.
    #[must_use]
    pub fn reads_neighbourhoods(&self) -> bool {
        self.ops.iter().any(|op| !op.neighbourhoods.is_empty())
    }
}

/// The colour of a layer: how its planes are decoded (a picture's; titles and colours are R'G'B'
/// already), its input transform, and its grade.
#[derive(Clone, Debug, Default)]
pub struct LayerColor {
    pub coding: YuvCoding,
    /// From the space of the media into the working space of the timeline.
    pub input: Stage,
    /// The grade of the clip: its group's pre-clip grade, its own, and its group's post-clip
    /// grade, one after the other.
    pub grade: Stage,
    /// Whether R'G'B' decoded keeps what is beyond 0..1 (superwhites and superblacks of
    /// Y'CbCr, and the values of integer R'G'B' of limited range beyond its ends), as a
    /// project whose colour is managed keeps them (RM20 p. 221: kept "internally", and its
    /// input transforms "done without clipping", p. 239); or holds it to 0..1, as phase 1 did
    /// and `DaVinci YRGB` here still does, so that a project that is not managed is drawn as it
    /// was, to the bit (the default). R'G'B' of floats is never held.
    pub unclamped: bool,
}

/// The WGSL of the operations of stages done one after another, as a function
/// `fn colour(rgb: vec3<f32>) -> vec3<f32>` and what it reads (bindings of group 1); the kinds of
/// the textures bound, in order; and the parameters, laid out as the code reads them.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Program {
    pub code: String,
    pub textures: Vec<TextureKind>,
}

/// What a program reads for one draw.
pub(crate) struct Inputs {
    pub params: Vec<[f32; 4]>,
    pub textures: Vec<wgpu::TextureView>,
}

/// The function that does nothing, in the shaders as they are written, between these lines.
pub(crate) const IDENTITY_BEGIN: &str = "// colour: begin";
pub(crate) const IDENTITY_END: &str = "// colour: end";

/// The bindings of group 1, before the textures: the parameters, then the sampler.
pub(crate) const FIRST_TEXTURE_BINDING: u32 = 2;

/// The filters of the neighbourhoods that the operations of `stages` read, in order: the
/// textures that [`program_of`] is given are these, filtered.
pub(crate) fn filters(stages: &[&Stage]) -> Vec<Filter> {
    stages
        .iter()
        .flat_map(|stage| &stage.ops)
        .flat_map(|op| op.neighbourhoods.iter().map(|n| n.filter))
        .collect()
}

/// Which colour a program gives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Part {
    /// That of all the operations.
    Whole,
    /// The colour that neighbourhood `n` (of all those of the stages, in order) filters: that
    /// of the operations before the one it is of, and then of its body.
    Neighbourhood(usize),
}

/// The program of `stages`, done one after another, and what it reads; none where they do
/// nothing. They are to read no neighbourhoods (see [`program_of`]).
pub(crate) fn program(stages: &[&Stage]) -> Option<(Program, Inputs)> {
    program_of(stages, Part::Whole, &[])
}

/// The program of `part` of `stages`, and what it reads, `neighbours` being the textures of
/// their neighbourhoods, filtered ([`filters`]): as many as come before the neighbourhood of
/// `part`, or all of them. None where it does nothing.
///
/// # Panics
/// Fewer `neighbours` are given than the program reads.
#[allow(
    clippy::too_many_lines,
    reason = "the parts of a program, one after another"
)]
pub(crate) fn program_of(
    stages: &[&Stage],
    part: Part,
    neighbours: &[wgpu::TextureView],
) -> Option<(Program, Inputs)> {
    let all: Vec<&ColorOp> = stages.iter().flat_map(|stage| &stage.ops).collect();
    // The number of the first neighbourhood of each operation.
    let mut first = Vec::with_capacity(all.len());
    let mut count = 0;
    for op in &all {
        first.push(count);
        count += op.neighbourhoods.len();
    }
    // The operations done; the body of the last, where it is a neighbourhood's; and how many
    // neighbourhoods are read.
    let (ops, last_body, reads) = match part {
        Part::Whole => (&all[..], None, count),
        Part::Neighbourhood(n) => {
            let at = (0..all.len())
                .find(|&i| n >= first[i] && n - first[i] < all[i].neighbourhoods.len())?;
            let body = Arc::clone(&all[at].neighbourhoods[n - first[at]].body);
            (&all[..=at], Some(body), n)
        }
    };
    if ops.is_empty() {
        return None;
    }
    assert!(
        neighbours.len() >= reads,
        "a program reads {reads} neighbourhoods, and {} are given",
        neighbours.len()
    );
    let mut code = String::new();
    let mut libraries: Vec<&str> = Vec::new();
    for op in ops {
        if !op.library.is_empty() && !libraries.contains(&&*op.library) {
            libraries.push(&op.library);
        }
    }
    for library in &libraries {
        code.push_str(library);
        code.push('\n');
    }
    code.push_str(
        "const colour_identity: bool = false;\n\
         var<private> colour_at: vec2<f32>;\n\
         @group(1) @binding(0) var<storage, read> colour_params: array<vec4<f32>>;\n\
         @group(1) @binding(1) var colour_sampler: sampler;\n\
         fn param(index: u32) -> vec4<f32> {\n    return colour_params[index];\n}\n",
    );
    let mut kinds = Vec::new();
    let mut inputs = Inputs {
        params: Vec::new(),
        textures: Vec::new(),
    };
    for (index, op) in ops.iter().enumerate() {
        let body = match &last_body {
            Some(body) if index + 1 == ops.len() => body,
            _ => &op.body,
        };
        let mut body = body.replace("$P", &format!("{}u", inputs.params.len()));
        // From the last, so that $T1 is not taken for the start of $T10.
        for number in (0..op.textures.len()).rev() {
            let binding = kinds.len() + number;
            body = body.replace(&format!("$T{number}"), &format!("colour_texture_{binding}"));
        }
        for number in (0..op.neighbourhoods.len()).rev() {
            body = body.replace(
                &format!("$N{number}"),
                &format!("colour_neighbour_{}()", first[index] + number),
            );
        }
        body = body.replace("$S", "colour_sampler");
        for texture in &op.textures {
            let binding = kinds.len();
            let _ = writeln!(
                code,
                "@group(1) @binding({}) var colour_texture_{binding}: {};",
                FIRST_TEXTURE_BINDING as usize + binding,
                texture.kind.wgsl()
            );
            kinds.push(texture.kind);
            inputs.textures.push(texture.view.clone());
        }
        let _ = write!(
            code,
            "fn colour_op_{index}(colour_in: vec3<f32>) -> vec3<f32> {{\n    var rgb = colour_in;\n    \
             {{\n{body}\n    }}\n    return rgb;\n}}\n"
        );
        inputs.params.extend(&op.params);
    }
    for (number, view) in neighbours[..reads].iter().enumerate() {
        code.push_str(&neighbour_code(number, kinds.len()));
        kinds.push(TextureKind::D2);
        inputs.textures.push(view.clone());
    }
    code.push_str("fn colour(rgb: vec3<f32>) -> vec3<f32> {\n    var out = rgb;\n");
    for index in 0..ops.len() {
        let _ = writeln!(code, "    out = colour_op_{index}(out);");
    }
    code.push_str("    return out;\n}\n");
    if inputs.params.is_empty() {
        // A binding of storage is never empty.
        inputs.params.push([0.0; 4]);
    }
    Some((
        Program {
            code,
            textures: kinds,
        },
        inputs,
    ))
}

/// The WGSL of neighbourhood `number`, filtered, its texture the `binding`th of the program:
/// `colour_neighbour_{number}()`, its colour where the pixel lies in the picture (`colour_at`,
/// from 0 to 1 across and down), between its four texels as the shader of pictures reads its
/// planes, unpremultiplied.
fn neighbour_code(number: usize, binding: usize) -> String {
    let texture = format!("colour_texture_{binding}");
    format!(
        "@group(1) @binding({}) var {texture}: texture_2d<f32>;\n\
         fn colour_neighbour_{number}() -> vec3<f32> {{\n    \
         let size = vec2<f32>(textureDimensions({texture}));\n    \
         let place = colour_at * size - vec2<f32>(0.5);\n    \
         let base = floor(place);\n    \
         let blend = place - base;\n    \
         let last = size - vec2<f32>(1.0);\n    \
         let low = vec2<i32>(clamp(base, vec2<f32>(0.0), last));\n    \
         let high = vec2<i32>(clamp(base + vec2<f32>(1.0), vec2<f32>(0.0), last));\n    \
         let top = mix(textureLoad({texture}, low, 0), \
         textureLoad({texture}, vec2<i32>(high.x, low.y), 0), blend.x);\n    \
         let bottom = mix(textureLoad({texture}, vec2<i32>(low.x, high.y), 0), \
         textureLoad({texture}, high, 0), blend.x);\n    \
         let filtered = mix(top, bottom, blend.y);\n    \
         if (filtered.a <= 0.0) {{\n        return vec3<f32>(0.0);\n    }}\n    \
         return filtered.rgb / filtered.a;\n}}\n",
        FIRST_TEXTURE_BINDING as usize + binding,
    )
}

/// A shader with the function of colour that it is written with (the identity) put in place by
/// that of `program`.
pub(crate) fn with_program(shader: &str, program: &Program) -> String {
    let begin = shader
        .find(IDENTITY_BEGIN)
        .expect("a shader of layers has a function of colour");
    let end = shader[begin..]
        .find(IDENTITY_END)
        .map(|at| begin + at + IDENTITY_END.len())
        .expect("a shader of layers has a function of colour");
    format!("{}{}{}", &shader[..begin], program.code, &shader[end..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_matrices_are_those_of_their_recommendations() {
        let close = |a: [f64; 4], b: [f64; 4]| a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-4);
        let of = |matrix| {
            YuvCoding {
                matrix,
                ..YuvCoding::default()
            }
            .coefficients()
        };
        // As BT.709 and the shader of phase 1 wrote them.
        assert!(close(
            of(YuvMatrix::Bt709),
            [1.5748, 0.187_324, 0.468_124, 1.8556]
        ));
        // BT.601 as ITU-R BT.601 (and JPEG) give it: R = Y + 1.402 Cr, B = Y + 1.772 Cb.
        assert!(close(
            of(YuvMatrix::Bt601),
            [1.402, 0.344_136, 0.714_136, 1.772]
        ));
        // BT.2020 non-constant luminance.
        assert!(close(
            of(YuvMatrix::Bt2020),
            [1.4746, 0.164_553, 0.571_353, 1.8814]
        ));
        assert_eq!(YuvCoding::default().matrix, YuvMatrix::Bt709);
    }

    #[test]
    fn full_range_takes_every_code_and_any_other_leaves_the_ends() {
        let full = YuvCoding {
            range: ColorRange::Full,
            ..YuvCoding::default()
        };
        assert_eq!(full.levels(false), [0.0, 255.0, 128.0, 255.0]);
        assert_eq!(full.levels(true), [0.0, 1023.0, 512.0, 1023.0]);
        for range in [ColorRange::Limited, ColorRange::Unspecified] {
            let coding = YuvCoding {
                range,
                ..YuvCoding::default()
            };
            assert_eq!(coding.levels(false), [16.0, 219.0, 128.0, 224.0]);
            assert_eq!(coding.levels(true), [64.0, 876.0, 512.0, 896.0]);
        }
    }

    #[test]
    fn a_file_that_says_nothing_of_its_colours_is_coded_as_players_take_it() {
        let nothing = ColorFacts::default();
        assert_eq!(
            YuvCoding::of_facts(&nothing, 1920, 1080),
            YuvCoding {
                matrix: YuvMatrix::Bt709,
                range: ColorRange::Limited
            }
        );
        assert_eq!(
            YuvCoding::of_facts(&nothing, 720, 576).matrix,
            YuvMatrix::Bt601
        );
        let full_2020 = ColorFacts {
            matrix: 9,
            range: ColorRange::Full,
            ..ColorFacts::default()
        };
        assert_eq!(
            YuvCoding::of_facts(&full_2020, 3840, 2160),
            YuvCoding {
                matrix: YuvMatrix::Bt2020,
                range: ColorRange::Full
            }
        );
    }

    #[test]
    fn stages_that_do_nothing_make_no_program() {
        assert!(program(&[]).is_none());
        assert!(program(&[&Stage::default(), &Stage::default()]).is_none());
        assert!(Stage::default().is_identity());
    }

    #[test]
    fn operations_are_done_in_order_each_reading_its_own_parameters() {
        let gain = ColorOp {
            body: "rgb = rgb * param($P).rgb;".into(),
            params: vec![[0.5, 0.5, 0.5, 0.0]],
            ..ColorOp::default()
        };
        let lift = ColorOp {
            body: "rgb = rgb + param($P).rgb + param($P + 1u).rgb;".into(),
            library: "fn shared_helper() {}".into(),
            params: vec![[0.1; 4], [0.2; 4]],
            ..ColorOp::default()
        };
        let first = Stage {
            ops: vec![gain.clone()],
        };
        let second = Stage {
            ops: vec![lift.clone(), lift],
        };
        let (program, inputs) = program(&[&first, &second]).unwrap();
        assert_eq!(
            inputs.params,
            [[0.5, 0.5, 0.5, 0.0], [0.1; 4], [0.2; 4], [0.1; 4], [0.2; 4]]
        );
        assert!(program.code.contains("rgb = rgb * param(0u).rgb;"));
        assert!(program
            .code
            .contains("rgb = rgb + param(1u).rgb + param(1u + 1u).rgb;"));
        assert!(program
            .code
            .contains("rgb = rgb + param(3u).rgb + param(3u + 1u).rgb;"));
        // The library once, before the operations.
        assert_eq!(program.code.matches("fn shared_helper").count(), 1);
        let calls: Vec<usize> = (0..3)
            .map(|index| {
                program
                    .code
                    .find(&format!("out = colour_op_{index}(out);"))
                    .unwrap()
            })
            .collect();
        assert!(calls.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(program.textures, []);
        // The same graph with other values: the same shader.
        let mut other = first.clone();
        other.ops[0].params = vec![[2.0; 4]];
        let (again, _) = super::program(&[&other, &second]).unwrap();
        assert_eq!(again, program);
    }

    #[test]
    fn the_identity_of_a_shader_is_put_in_place_by_a_program() {
        let shader = "a\n// colour: begin\nfn colour(rgb: vec3<f32>) -> vec3<f32> { return rgb; }\n// colour: end\nb";
        let program = Program {
            code: "fn colour(x) {}".into(),
            textures: Vec::new(),
        };
        assert_eq!(with_program(shader, &program), "a\nfn colour(x) {}\nb");
    }
}
