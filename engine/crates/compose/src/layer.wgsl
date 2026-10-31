// A layer of R'G'B'A, premultiplied, put on the canvas: a title, a subtitle, or a colour (a
// texture of one white texel, tinted). The quad of the frame that it covers is put on the canvas
// by an affine map, as a picture is (`yuv.wgsl`), its edges smoothed over a pixel, and multiplied
// by its opacity.

struct Params {
    // A point (x, y) of the frame, in its pixels, is put at
    // (row0.x * x + row0.y * y + row0.z, row1.x * x + row1.y * y + row1.z) of the canvas, whose
    // width is row0.w and height row1.w.
    row0: vec4<f32>,
    row1: vec4<f32>,
    // The quad drawn, from (x0, y0) to (x1, y1) of the frame, which the texture covers.
    quad: vec4<f32>,
    // The opacity of the layer, and the pixels of the canvas to a pixel of the frame.
    extra: vec4<f32>,
    // What the colour of each texel is multiplied by, and its opacity.
    tint: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var picture: texture_2d<f32>;
@group(0) @binding(2) var bilinear: sampler;

struct Layer {
    @builtin(position) position: vec4<f32>,
    @location(0) local: vec2<f32>,
};

fn corner(index: u32) -> vec2<f32> {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
    );
    return corners[index];
}

@vertex
fn vs_layer(@builtin(vertex_index) index: u32) -> Layer {
    let grow = 1.0 / params.extra.y;
    let local = mix(params.quad.xy - vec2<f32>(grow), params.quad.zw + vec2<f32>(grow), corner(index));
    let x = params.row0.x * local.x + params.row0.y * local.y + params.row0.z;
    let y = params.row1.x * local.x + params.row1.y * local.y + params.row1.z;
    var out: Layer;
    out.position = vec4<f32>(x / params.row0.w * 2.0 - 1.0, 1.0 - y / params.row1.w * 2.0, 0.0, 1.0);
    out.local = local;
    return out;
}

fn coverage(local: vec2<f32>) -> f32 {
    let inside = min(
        min(local.x - params.quad.x, params.quad.z - local.x),
        min(local.y - params.quad.y, params.quad.w - local.y),
    ) * params.extra.y;
    return clamp(inside + 0.5, 0.0, 1.0);
}

// The input transform and the grade of the layer (`color.rs`): the lines from "colour: begin"
// to "colour: end" are put in place by those of a layer that has them; as they are written,
// they do nothing. `colour_at` is where the pixel lies in the texture, from 0 to 1 across and
// down, for the neighbourhoods that a grade reads there.
// colour: begin
const colour_identity: bool = true;
var<private> colour_at: vec2<f32>;
fn colour(rgb: vec3<f32>) -> vec3<f32> {
    return rgb;
}
// colour: end

// The colour of a texel premultiplied by `alpha`, graded as the colour it stands for.
fn graded(rgb: vec3<f32>, alpha: f32) -> vec3<f32> {
    if colour_identity || alpha <= 0.0 {
        return rgb;
    }
    return colour(rgb / alpha) * alpha;
}

@fragment
fn fs_layer(layer: Layer) -> @location(0) vec4<f32> {
    let size = params.quad.zw - params.quad.xy;
    let at = clamp((layer.local - params.quad.xy) / size, vec2<f32>(0.0), vec2<f32>(1.0));
    let texel = textureSample(picture, bilinear, at);
    colour_at = at;
    let alpha = coverage(layer.local) * params.extra.x * params.tint.a;
    return vec4<f32>(graded(texel.rgb * params.tint.rgb, texel.a), texel.a) * alpha;
}
