// A pass of the composition of a clip (`fusion.rs`): a foreground moved, and merged with a
// background, each pixel of the picture that comes out made of the pixel of the background at
// its place and of the foreground as it lies there. It draws a Merge node; and a Transform node,
// which is its picture moved and laid over nothing by a merge that changes nothing.
//
// Before these lines is put the maths of the composite modes, which is that of the Layer Mixer
// of a grade (the end of `grade/corrector.wgsl`, its names made `layer_…`, as `blend.wgsl` has
// it for the composite mode of a layer): `layer_composite`, and `LAYER_LUMA`, the weights of R,
// G and B in luma that it reads.
//
// The pictures hold R, G, B and A of the working space of the timeline, premultiplied by their
// alpha, in half floats: what is above 1 or below 0 is kept as it is.

struct Params {
    // The pixel at (x, y) of the picture that comes out, in its pixels from its top left
    // corner, reads the foreground at
    // (row0.x * x + row0.y * y + row0.z, row1.x * x + row1.y * y + row1.z), in the pixels of
    // the foreground from its top left corner.
    row0: vec4<f32>,
    row1: vec4<f32>,
    // x: the pixels of the picture that comes out to a pixel of the foreground; y: how many
    // points across and down each pixel is made of; z: the Apply Mode, numbered as
    // `grade/layout.rs` numbers the composite modes.
    extra: vec4<f32>,
    // Subtractive/Additive, Alpha Gain, Burn In and Blend, as `merge::Settings::held` makes
    // them.
    sliders: vec4<f32>,
    // The weights of R, G and B in the luma of the working space.
    luma: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var background: texture_2d<f32>;
@group(0) @binding(2) var foreground: texture_2d<f32>;

// One triangle that covers the target.
@vertex
fn vs(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let x = f32((index << 1u) & 2u);
    let y = f32(index & 2u);
    return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

// The foreground at `place`, in its pixels from its top left corner, `size` of them: between
// the four texels about it, as the shader of pictures reads its planes (`yuv.wgsl`), those
// beyond its edges being those at its edges. At the middle of a texel it is that texel, to the
// bit.
fn fusion_at(place: vec2<f32>, size: vec2<f32>) -> vec4<f32> {
    let centre = clamp(place, vec2<f32>(0.0), size) - vec2<f32>(0.5);
    let last = vec2<i32>(size) - vec2<i32>(1, 1);
    let base = floor(centre);
    let low = clamp(vec2<i32>(base), vec2<i32>(0, 0), last);
    let high = clamp(vec2<i32>(base) + vec2<i32>(1, 1), vec2<i32>(0, 0), last);
    let blend = centre - base;
    let top = mix(
        textureLoad(foreground, low, 0),
        textureLoad(foreground, vec2<i32>(high.x, low.y), 0),
        blend.x,
    );
    let bottom = mix(
        textureLoad(foreground, vec2<i32>(low.x, high.y), 0),
        textureLoad(foreground, high, 0),
        blend.x,
    );
    return mix(top, bottom, blend.y);
}

// The foreground as it lies at the pixel whose middle is `position`: moved, made larger or
// smaller, turned and flipped by the map of the uniform; nothing where it does not reach, its
// edges smoothed over a pixel as those of a layer are. Where it is made smaller, each pixel is
// made of `extra.y` × `extra.y` points of it, over the pixel and as far again about it, weighed
// as a tent, as a picture drawn smaller is (`yuv.wgsl`); how far a pixel reaches in it is read
// from the map, which is the same at every pixel.
fn fusion_moved(position: vec2<f32>) -> vec4<f32> {
    let size = vec2<f32>(textureDimensions(foreground));
    let at = vec2<f32>(
        params.row0.x * position.x + params.row0.y * position.y + params.row0.z,
        params.row1.x * position.x + params.row1.y * position.y + params.row1.z,
    );
    let inside = min(min(at.x, size.x - at.x), min(at.y, size.y - at.y)) * params.extra.x;
    let coverage = clamp(inside + 0.5, 0.0, 1.0);
    if coverage <= 0.0 {
        return vec4<f32>(0.0);
    }
    let points = u32(params.extra.y + 0.5);
    var sampled: vec4<f32>;
    if points <= 1u {
        sampled = fusion_at(at, size);
    } else {
        let across = vec2<f32>(params.row0.x, params.row1.x);
        let down = vec2<f32>(params.row0.y, params.row1.y);
        var sum = vec4<f32>(0.0);
        var weights = 0.0;
        let count = f32(points);
        for (var i = 0u; i < points; i++) {
            let u = (f32(i) + 0.5) / count * 2.0 - 1.0;
            for (var j = 0u; j < points; j++) {
                let v = (f32(j) + 0.5) / count * 2.0 - 1.0;
                let weight = (1.0 - abs(u)) * (1.0 - abs(v));
                sum += fusion_at(at + u * across + v * down, size) * weight;
                weights += weight;
            }
        }
        sampled = sum / weights;
    }
    return sampled * coverage;
}

// A premultiplied colour divided by its alpha; nothing where the alpha is not above 0.
fn fusion_straight(colour: vec3<f32>, alpha: f32) -> vec3<f32> {
    if alpha > 0.0 {
        return colour / alpha;
    }
    return vec3<f32>(0.0);
}

// A pixel of the foreground merged with a pixel of the background, both premultiplied, and so
// the result: the stages of `merge.rs`, in their order, each by the formula of the function of
// that name there, to which this is held (`tests/fusion.rs`).
fn fusion_merged(under: vec4<f32>, over: vec4<f32>) -> vec4<f32> {
    let subtractive_additive = params.sliders.x;
    let alpha_gain = params.sliders.y;
    let burn_in = params.sliders.z;
    let blend = params.sliders.w;
    // Alpha Gain (`gained`): the alpha that every later stage reads.
    let alpha = over.a * alpha_gain;
    // The Apply Mode (`applied`): Normal leaves the colour of the foreground as it is; another
    // mode makes the share of it that lies over the background what the mode makes of the two
    // colours, each divided by the alpha that it was given with.
    var colour = over.rgb;
    let mode = u32(params.extra.z + 0.5);
    if mode != 0u {
        LAYER_LUMA = params.luma.xyz;
        let behind = under.a;
        let mixed = layer_composite(
            mode,
            fusion_straight(under.rgb, behind),
            fusion_straight(over.rgb, over.a),
        );
        colour = over.rgb * (1.0 - behind) + over.a * behind * mixed;
    }
    // Subtractive/Additive (`added`): the colour that is added in.
    let share = subtractive_additive + (1.0 - subtractive_additive) * alpha;
    colour = colour * share;
    // Burn In (`held_out`): the alpha that holds the background out.
    let held = alpha * (1.0 - burn_in);
    // The operator (`laid`), Over: `x` is 1 and `y` is what the foreground leaves.
    let rest = 1.0 - held;
    let merged = vec4<f32>(colour + under.rgb * rest, held + under.a * rest);
    // Blend (`blended`): the background and the merged result are kept to the bit at its ends.
    if blend >= 1.0 {
        return merged;
    }
    if blend > 0.0 {
        return under + (merged - under) * blend;
    }
    return under;
}

@fragment
fn fs(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    // The background is of the size of the picture that comes out; a pass without one is given
    // a texel of nothing, which is read at every pixel.
    let last = vec2<i32>(textureDimensions(background)) - vec2<i32>(1, 1);
    let under = textureLoad(background, clamp(vec2<i32>(position.xy), vec2<i32>(0, 0), last), 0);
    return fusion_merged(under, fusion_moved(position.xy));
}
