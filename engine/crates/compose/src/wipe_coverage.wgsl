struct WipeParams {
    // The width and height of the canvas, and its centre, in pixels.
    canvas: vec4<f32>,
    // The normal of the border of a line (x, y), how far the border is from the centre along
    // it, and the width of the softness, in pixels (0: a hard edge).
    line: vec4<f32>,
    // Half the width and height of the box, the weight of the reference of a mix (or the gain
    // of a difference), and the width of the border line, in pixels.
    box_: vec4<f32>,
    // The width and height of a strip of the blinds or a check of the board, in pixels.
    cell: vec4<f32>,
    // The colour of the border line.
    colour: vec4<f32>,
    // The style (0 a line, 1 a box, 2 a mix, 3 a difference, 4 blinds, 5 a checkerboard), and
    // whether the sides are swapped.
    kind: vec4<u32>,
}

// The weight of the reference at a signed distance `d` from the border: 0 before it, 1 after
// it, a straight ramp across the softness about it.
fn ramp(wipe: WipeParams, d: f32) -> f32 {
    let soft = wipe.line.w;
    if (soft > 0.0) {
        return clamp(d / soft + 0.5, 0.0, 1.0);
    }
    return select(0.0, 1.0, d >= 0.0);
}

// Reference share and signed distance, with exactly the image wipe arithmetic.
fn reference_weight(wipe: WipeParams, at: vec2<f32>) -> vec2<f32> {
    let style = wipe.kind.x;
    let from_centre = at - wipe.canvas.zw;
    var w = 0.0;
    var d = 0.0;
    if (style == 0u) {
        d = dot(from_centre, wipe.line.xy) - wipe.line.z;
        w = ramp(wipe, d);
    } else if (style == 1u) {
        let outside = abs(from_centre) - wipe.box_.xy;
        d = max(outside.x, outside.y);
        w = ramp(wipe, d);
    } else if (style == 2u) {
        w = wipe.box_.z;
    } else if (style == 4u) {
        let strip = u32(floor(at.y / wipe.cell.y));
        w = f32(strip & 1u);
    } else {
        let across = u32(floor(at.x / wipe.cell.x));
        let down = u32(floor(at.y / wipe.cell.y));
        w = f32((across + down) & 1u);
    }
    if (wipe.kind.y != 0u) {
        w = 1.0 - w;
    }
    return vec2<f32>(w, d);
}

fn has_current(wipe: WipeParams, at: vec2<f32>) -> bool {
    return wipe.kind.x == 3u || reference_weight(wipe, at).x < 1.0;
}

// The view samples the composed image with a clamped bilinear sampler. A neighbouring
// current texel counts only when both axis weights are positive, even at a hard wipe edge.
fn sample_has_current(wipe: WipeParams, at: vec2<f32>) -> bool {
    if (wipe.kind.x == 3u) { return true; }
    let sample = clamp(at, vec2<f32>(0.5), wipe.canvas.xy - vec2<f32>(0.5));
    let low = floor(sample - vec2<f32>(0.5)) + vec2<f32>(0.5);
    let fraction = sample - low;
    return ((1.0-fraction.x) > 0.0 && (1.0-fraction.y) > 0.0 && has_current(wipe, low))
        || (fraction.x > 0.0 && (1.0-fraction.y) > 0.0 && has_current(wipe, low + vec2<f32>(1.0,0.0)))
        || ((1.0-fraction.x) > 0.0 && fraction.y > 0.0 && has_current(wipe, low + vec2<f32>(0.0,1.0)))
        || (fraction.x > 0.0 && fraction.y > 0.0 && has_current(wipe, low + vec2<f32>(1.0,1.0)));
}
