// The passes of a Delta Keyer node of the composition of a clip (`fusion.rs`), held to the
// mathematics of the keyer on the processor (`keyer.rs`), whose functions these are, line for
// line, by the same names with `keyer_` before them.
//
// A keyer that reads no pixel about another is one pass, which makes its picture: the key of
// each pixel (its pre-matte), that through the thresholds (its matte), the solid and the garbage
// mattes that come into the node, the result with its spill suppressed and what the matte gave
// back filled, the view of the node, and its effect mask.
//
// One that reads the pixels about each one (`keyer::Finesse`) has passes before that. With a
// Pre-Blur, the picture is blurred first by the two passes of a filter of the neighbourhoods
// (`neighbourhood.wgsl`), and the key is taken of that, read between its texels. With an
// Erode/Dilate, a Matte Blur or a Clean, the matte of the key is drawn alone into a texture,
// white premultiplied by it; each of the three then filters that texture by the same two passes
// and draws the matte anew from it and from what the filter made, read between its texels; and
// the pass that makes the picture reads the matte that the last of them drew.
//
// The pictures hold R, G, B and A premultiplied by their alpha, in half floats, and are keyed as
// they are, in the working space of the timeline. A colour is that of a pixel not premultiplied,
// and black where its alpha is not above 0 (`keyer::unpremultiplied`); the final result is made
// of that colour and multiplied by the alpha that came in. A matte that comes into the node, and
// its effect mask, are the alpha of a picture within 0 and 1.
//
// Places are counted in floats, and what is counted in whole numbers in u32: the compiler of
// Direct3D 12 crashes on some arithmetic of signed integers with constants
// (`neighbourhood.wgsl`).

struct Keyer {
    // Of the pass that makes the picture and of the pass that makes the matte of the key, the
    // four rows of `keyer::Keyer::uniform`. xyz: the screen colour; w: its dominant channel, 0
    // red, 1 green, 2 blue.
    //
    // Of a pass that draws the matte anew: of Erode/Dilate, x: 1 where the matte is grown and
    // 0 where it is shrunk, y: how much of the box about a pixel is to be in the matte
    // (`keyer::MORPH_EDGE`); of Clean, x: Clean Background, y: Clean Foreground.
    screen: vec4<f32>,
    // x: Gain; y: the colour difference of the screen colour; z: Balance; w: a sum of what the
    // pass reads: 1, a garbage matte; 2, it the other way round; 4, a solid matte; 8, an effect
    // mask; 16, the matte that the passes before drew; 32, the picture blurred, to be keyed.
    key: vec4<f32>,
    // x: the lower threshold; y: the upper; z: the share of the greater of the other two
    // channels in the limit of spill; w: how much of the spill is taken away.
    matte: vec4<f32>,
    // xyz: the replace colour; w: the replace mode, 0 None, 1 Source, 2 Hard Color and 3 Soft
    // Color.
    replace: vec4<f32>,
    // x: what the pass makes: 0, the picture; 1, the matte of the key; 2, the matte through
    // Erode/Dilate; 3, through the Matte Blur; 4, through the Cleans. y: the view of the pass
    // that makes the picture: 0 Pre Matte, 1 Matte, 2 Status and 3 Final Result. zw: the texels
    // of what a filter made (`blurred`) to a pixel of the picture that comes out, across and
    // down: 1 where the two are of one size.
    made: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: Keyer;
// The picture as it came in; of a pass that draws the matte anew, the matte so far. Of the size
// of the picture that comes out.
@group(0) @binding(1) var source: texture_2d<f32>;
// What a filter of the neighbourhoods made: the picture blurred by the Pre-Blur, or, of a pass
// that draws the matte anew, the matte filtered. A texel of nothing, which is not read, where
// there is none.
@group(0) @binding(2) var blurred: texture_2d<f32>;
// The matte that the passes before drew, of the size of the picture that comes out; a texel of
// nothing, which is not read, where the matte is that of the key.
@group(0) @binding(3) var refined: texture_2d<f32>;
// The pictures whose alphas are the garbage matte, the solid matte and the effect mask of the
// node, of the size of the picture that comes out; a texel of nothing, which is not read, in the
// place of one that the node has not.
@group(0) @binding(4) var garbage: texture_2d<f32>;
@group(0) @binding(5) var solid: texture_2d<f32>;
@group(0) @binding(6) var effect_mask: texture_2d<f32>;

// The largest value of a channel (`keyer::LARGEST`).
const KEYER_LARGEST: f32 = 65504.0;
// The least that a matte is divided by.
const KEYER_LEAST: f32 = 0.000001;
// How far a matte is from its pre-matte before the status shows it as changed
// (`keyer::STATUS_TOLERANCE`).
const KEYER_STATUS_TOLERANCE: f32 = 0.00390625;

// One triangle that covers the target.
@vertex
fn vs(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let x = f32((index << 1u) & 2u);
    let y = f32(index & 2u);
    return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

// A channel as the maths reads it (`sound`): 0 where it is no number, and no further from 0
// than the largest value.
fn keyer_sound(value: f32) -> f32 {
    if value > 0.0 {
        return min(value, KEYER_LARGEST);
    }
    if value < 0.0 {
        return max(value, -KEYER_LARGEST);
    }
    return 0.0;
}

// A matte as the maths reads it (`unit`): within 0 and 1, and 0 where it is no number.
fn keyer_unit(value: f32) -> f32 {
    if value > 0.0 {
        return min(value, 1.0);
    }
    return 0.0;
}

// The colour of a premultiplied pixel (`unpremultiplied`): not premultiplied, and black where
// its alpha is not above 0.
fn keyer_colour(pixel: vec4<f32>) -> vec3<f32> {
    let alpha = keyer_sound(pixel.a);
    if alpha > 0.0 {
        return pixel.rgb / alpha;
    }
    return vec3<f32>(0.0);
}

// The channel of `x` that is the dominant one of the screen, and the lesser and the greater of
// its other two (`split`).
fn keyer_split(x: vec3<f32>) -> vec3<f32> {
    let dominant = u32(params.screen.w + 0.5);
    var parts = vec3<f32>(x.b, x.r, x.g);
    if dominant == 0u {
        parts = vec3<f32>(x.r, x.g, x.b);
    }
    if dominant == 1u {
        parts = vec3<f32>(x.g, x.r, x.b);
    }
    return vec3<f32>(parts.x, min(parts.y, parts.z), max(parts.y, parts.z));
}

// `x` with `value` for the channel that is the dominant one of the screen (`put`).
fn keyer_put(x: vec3<f32>, value: f32) -> vec3<f32> {
    let dominant = u32(params.screen.w + 0.5);
    if dominant == 0u {
        return vec3<f32>(value, x.g, x.b);
    }
    if dominant == 1u {
        return vec3<f32>(x.r, value, x.b);
    }
    return vec3<f32>(x.r, x.g, value);
}

// How far the dominant channel of `x` is above its other two, mixed: `weight` of the greater
// and the rest of the lesser (`above`).
fn keyer_above(weight: f32, x: vec3<f32>) -> f32 {
    let parts = keyer_split(x);
    return parts.x - (parts.y + weight * (parts.z - parts.y));
}

// `x` with each channel as the maths reads it.
fn keyer_sound3(x: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(keyer_sound(x.r), keyer_sound(x.g), keyer_sound(x.b));
}

// The pre-matte of the colour `x` (`pre_matte`).
fn keyer_pre_matte(x: vec3<f32>) -> f32 {
    let difference = keyer_above(params.key.z, keyer_sound3(x));
    return clamp(1.0 - params.key.x * (difference / params.key.y), 0.0, 1.0);
}

// A matte through the thresholds (`thresholds`).
fn keyer_thresholds(given: f32) -> f32 {
    let level = keyer_unit(given);
    let low = params.matte.x;
    let high = params.matte.y;
    if low == 0.0 && high == 1.0 {
        return level;
    }
    if high <= low {
        return select(0.0, 1.0, level > low);
    }
    return clamp((level - low) / (high - low), 0.0, 1.0);
}

// `x` with its spill suppressed (`suppress`); `x` is sound.
fn keyer_suppressed(x: vec3<f32>) -> vec3<f32> {
    let over = max(keyer_above(params.matte.z, x), 0.0);
    return keyer_put(x, keyer_split(x).x - params.matte.w * over);
}

// The final result of the colour `x`, of its pre-matte and its matte, premultiplied
// (`result_with`).
fn keyer_result(given: vec3<f32>, pre_given: f32, matte_given: f32) -> vec4<f32> {
    let x = keyer_sound3(given);
    let pre = keyer_unit(pre_given);
    let level = keyer_unit(matte_given);
    // The screen that the key takes away: of each channel no more than is there.
    let gone = 1.0 - pre;
    let lifted = x - clamp(gone * params.screen.rgb, vec3<f32>(0.0), max(x, vec3<f32>(0.0)));
    let kept = keyer_suppressed(lifted);
    let removed = x - kept;
    // The share of what the key made transparent that the matte gave back, and the share of
    // what the key kept that the matte left.
    let back = clamp((level - pre) / max(gone, KEYER_LEAST), 0.0, 1.0);
    let left = min(level / max(pre, KEYER_LEAST), 1.0);
    let mode = u32(params.replace.w + 0.5);
    var fill = vec3<f32>(0.0);
    if mode == 1u {
        fill = removed;
    }
    if mode == 2u {
        fill = params.replace.rgb * gone;
    }
    if mode == 3u {
        let taken = keyer_split(removed).x;
        let whole = keyer_split(params.screen.rgb).x;
        let share = min(taken / max(whole, KEYER_LEAST), 1.0);
        fill = params.replace.rgb * share;
    }
    return vec4<f32>((kept + back * fill) * left, level);
}

// What the Status view shows of a pixel, of its pre-matte and its matte (`status`).
fn keyer_status(pre_given: f32, matte_given: f32) -> vec3<f32> {
    let pre = keyer_unit(pre_given);
    let level = keyer_unit(matte_given);
    var shade = 0.5;
    if level <= 0.0 {
        shade = 0.0;
    }
    if level >= 1.0 {
        shade = 1.0;
    }
    if level > pre + KEYER_STATUS_TOLERANCE {
        return vec3<f32>(0.0, shade, 0.0);
    }
    if level < pre - KEYER_STATUS_TOLERANCE {
        return vec3<f32>(1.0 - shade, 0.0, 0.0);
    }
    return vec3<f32>(shade);
}

// The alpha of a picture as a matte that comes into the node, or its effect mask: within 0 and
// 1, and 0 where it is no number.
fn keyer_garbage_at(at: vec2<i32>) -> f32 {
    let last = vec2<i32>(textureDimensions(garbage)) - vec2<i32>(1, 1);
    return keyer_unit(textureLoad(garbage, clamp(at, vec2<i32>(0, 0), last), 0).a);
}

fn keyer_solid_at(at: vec2<i32>) -> f32 {
    let last = vec2<i32>(textureDimensions(solid)) - vec2<i32>(1, 1);
    return keyer_unit(textureLoad(solid, clamp(at, vec2<i32>(0, 0), last), 0).a);
}

fn keyer_mask_at(at: vec2<i32>) -> f32 {
    let last = vec2<i32>(textureDimensions(effect_mask)) - vec2<i32>(1, 1);
    return keyer_unit(textureLoad(effect_mask, clamp(at, vec2<i32>(0, 0), last), 0).a);
}

// What a filter made at the pixel `pixel` of the picture that comes out (its column and its
// row): between the four texels about the middle of the pixel (`neighbourhood::sample`), those
// beyond its edges being those at its edges.
fn keyer_between(pixel: vec2<f32>) -> vec4<f32> {
    let size = vec2<f32>(textureDimensions(blurred));
    let place = (pixel + vec2<f32>(0.5)) * params.made.zw - vec2<f32>(0.5);
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

// The smooth step of the maths of a grade (`grade::maths::smoothstep`).
fn keyer_smoothstep(low: f32, high: f32, x: f32) -> f32 {
    let t = clamp((x - low) / (high - low), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

// The matte at the pixel `pixel` drawn anew by the control that `kind` names, from the matte so
// far (`source`) and from that filtered (`blurred`): Erode/Dilate (`keyer::eroded`), the Matte
// Blur (`keyer::softened`) or the Cleans (`keyer::cleaned`). White premultiplied by it.
fn keyer_stage(kind: u32, pixel: vec2<f32>) -> vec4<f32> {
    let was = keyer_unit(textureLoad(source, vec2<i32>(pixel), 0).a);
    let around = keyer_between(pixel).a;
    var level = was;
    if kind == 2u {
        let edge = params.screen.y;
        if params.screen.x != 0.0 {
            level = max(was, clamp(around / edge, 0.0, 1.0));
        } else {
            level = min(was, 1.0 - clamp((1.0 - around) / edge, 0.0, 1.0));
        }
    }
    if kind == 3u {
        level = clamp(around, 0.0, 1.0);
    }
    if kind == 4u {
        if params.screen.x > 0.0 {
            level = level * keyer_smoothstep(0.0, params.screen.x, around);
        }
        if params.screen.y > 0.0 {
            level = 1.0 - (1.0 - level) * keyer_smoothstep(0.0, params.screen.y, 1.0 - around);
        }
    }
    return vec4<f32>(level);
}

@fragment
fn fs(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let pixel = floor(position.xy);
    let at = vec2<i32>(pixel);
    let kind = u32(params.made.x + 0.5);
    if kind >= 2u {
        return keyer_stage(kind, pixel);
    }
    let flags = u32(params.key.w + 0.5);
    let first = textureLoad(source, at, 0);
    let colour = keyer_colour(first);
    // The key is of the picture blurred, where it has a Pre-Blur; the colours are its own.
    var keyed = colour;
    if (flags & 32u) != 0u {
        keyed = keyer_colour(keyer_between(pixel));
    }
    let pre = keyer_pre_matte(keyed);
    var level = keyer_thresholds(pre);
    if kind == 1u {
        return vec4<f32>(level);
    }
    if (flags & 16u) != 0u {
        level = keyer_unit(textureLoad(refined, at, 0).a);
    }
    // The solid matte and then the garbage matte, which wins where both are 1 (`met`).
    var after = level;
    if (flags & 4u) != 0u {
        let filled = keyer_solid_at(at);
        if filled >= 1.0 {
            after = 1.0;
        } else if filled > 0.0 {
            after = after + (1.0 - after) * filled;
        }
    }
    if (flags & 1u) != 0u {
        var taken = keyer_garbage_at(at);
        if (flags & 2u) != 0u {
            taken = 1.0 - taken;
        }
        if taken >= 1.0 {
            after = 0.0;
        } else if taken > 0.0 {
            after = after * (1.0 - taken);
        }
    }
    // What the view shows (`node_pixel`): the mattes and the status opaque, and the final
    // result times the alpha that came in.
    let view = u32(params.made.y + 0.5);
    var own = vec4<f32>(vec3<f32>(keyer_unit(pre)), 1.0);
    if view == 1u {
        own = vec4<f32>(vec3<f32>(keyer_unit(level)), 1.0);
    }
    if view == 2u {
        own = vec4<f32>(keyer_status(pre, after), 1.0);
    }
    if view == 3u {
        own = keyer_result(colour, pre, after) * max(keyer_sound(first.a), 0.0);
    }
    if (flags & 8u) == 0u {
        return own;
    }
    // The effect mask: the pixel as it came in where it is 0, and what the view shows where it
    // is 1, both to the bit.
    let share = keyer_mask_at(at);
    if share <= 0.0 {
        return first;
    }
    if share >= 1.0 {
        return own;
    }
    return first + (own - first) * share;
}
