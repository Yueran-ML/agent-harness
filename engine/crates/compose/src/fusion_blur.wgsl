// The last pass of a Blur node of the composition of a clip (`fusion.rs`): the picture as it
// came in mixed with the picture blurred, by the Blend of the node and by its effect mask.
//
// The picture is blurred before this pass by the two passes of a filter of the neighbourhoods
// (`neighbourhood.wgsl`), across and down, into a texture that is smaller than the picture where
// the filter is wide (`neighbourhood.rs`): it is read here between its four texels about the
// middle of each pixel, as the operations of colour read a neighbourhood and as
// `neighbourhood::sample` does on the processor, those beyond its edges being those at its
// edges. At the middle of a texel it is that texel, to the bit.
//
// Each pixel is `input + (blurred - input) * share`, channel by channel, `share` being the Blend
// times the effect mask at the pixel (the Blend alone where the node has no mask). Where the
// share is 0 the pixel is that of the picture as it came in, to the bit, and where it is 1 that
// of the blurred picture.
//
// The pictures hold R, G, B and A premultiplied by their alpha, in half floats: the alpha is
// blurred with the colour.

struct Blur {
    // x: Blend, within 0 and 1; y: 1 where the node has an effect mask, 0 where it has none.
    share: vec4<f32>,
    // xy: the texels of the blurred picture to a pixel of the picture that comes out, across
    // and down: 1 where the two are of one size.
    scale: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: Blur;
// The picture as it came in, of the size of the picture that comes out.
@group(0) @binding(1) var source: texture_2d<f32>;
// The picture blurred, as the second pass of its filter left it.
@group(0) @binding(2) var blurred: texture_2d<f32>;
// The picture whose alpha is the effect mask of the node, of the size of the picture that comes
// out; a texel of nothing, which is not read, where the node has none.
@group(0) @binding(3) var effect_mask: texture_2d<f32>;

// One triangle that covers the target.
@vertex
fn vs(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let x = f32((index << 1u) & 2u);
    let y = f32(index & 2u);
    return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

// The blurred picture at the pixel `pixel` of the picture that comes out (its column and its
// row): between the four texels about the middle of the pixel (`neighbourhood::sample`).
fn blur_at(pixel: vec2<f32>) -> vec4<f32> {
    let size = vec2<f32>(textureDimensions(blurred));
    let place = (pixel + vec2<f32>(0.5)) * params.scale.xy - vec2<f32>(0.5);
    let base = floor(place);
    let blend = place - base;
    let last = size - vec2<f32>(1.0);
    let low = vec2<i32>(clamp(base, vec2<f32>(0.0), last));
    let high = vec2<i32>(clamp(base + vec2<f32>(1.0), vec2<f32>(0.0), last));
    let upper_left = textureLoad(blurred, low, 0);
    let upper_right = textureLoad(blurred, vec2<i32>(high.x, low.y), 0);
    let lower_left = textureLoad(blurred, vec2<i32>(low.x, high.y), 0);
    let lower_right = textureLoad(blurred, high, 0);
    let top = upper_left + (upper_right - upper_left) * blend.x;
    let bottom = lower_left + (lower_right - lower_left) * blend.x;
    return top + (bottom - top) * blend.y;
}

// The effect mask at the pixel `at`: the alpha of the picture that is linked to the `mask`
// input of the node, within 0 and 1 (an alpha below 0, or one that is no number, is 0, and one
// above 1 is 1, as `fusion.wgsl` reads it).
fn blur_mask_at(at: vec2<i32>) -> f32 {
    let last = vec2<i32>(textureDimensions(effect_mask)) - vec2<i32>(1, 1);
    let alpha = textureLoad(effect_mask, clamp(at, vec2<i32>(0, 0), last), 0).a;
    if alpha > 0.0 {
        return min(alpha, 1.0);
    }
    return 0.0;
}

@fragment
fn fs(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let pixel = floor(position.xy);
    let at = vec2<i32>(pixel);
    let first = textureLoad(source, at, 0);
    var share = params.share.x;
    if params.share.y > 0.5 {
        share = share * blur_mask_at(at);
    }
    if share <= 0.0 {
        return first;
    }
    let own = blur_at(pixel);
    if share >= 1.0 {
        return own;
    }
    return first + (own - first) * share;
}
