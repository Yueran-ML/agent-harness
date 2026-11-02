// The Blur palette of a corrector (pp. 3454-3456), as `blur.rs` does it on the processor, line for
// line: `x` what the node made, `near` and `wide` each channel of it filtered by its near and
// wide neighbourhoods, and the block of the palette at `at` (`blur.rs`, `BLOCK`). Channels are
// written out one by one: the compiler of Direct3D 12 crashes on vectors indexed in loops.

fn grade_blur_core(d: f32, level: f32, soft: f32) -> f32 {
    if level <= 0.0 {
        return 1.0;
    }
    let low = max(level - soft, 0.0);
    if low < level {
        return smoothstep(low, level, d);
    }
    if d >= level {
        return 1.0;
    }
    return 0.0;
}

// `v` kept in a half float, to the nearest, half way to the even one, within the range of half
// floats (`neighbourhood.rs`, `half`, as the processor does it). Not `pack2x16float`, which a card
// may round towards 0 where the processor rounds to the nearest.
fn grade_blur_half(v: f32) -> f32 {
    let size = min(abs(v), 65504.0);
    var kept = 0.0;
    if size < 6.1035156e-5 {
        // Below the least normal half float, its steps are of 2^-24.
        let steps = size * 16777216.0;
        var whole = floor(steps);
        let left = steps - whole;
        if left > 0.5 || (left == 0.5 && fract(whole * 0.5) != 0.0) {
            whole = whole + 1.0;
        }
        kept = whole / 16777216.0;
    } else {
        // Ten bits of the fraction kept of 23.
        let bits = bitcast<u32>(size);
        let odd = (bits >> 13u) & 1u;
        kept = bitcast<f32>((bits + 0x0fffu + odd) & 0xffffe000u);
    }
    return select(kept, -kept, v < 0.0);
}

// A colour kept in half floats, as the textures of neighbourhoods keep it.
fn grade_blur_kept(x: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(grade_blur_half(x.x), grade_blur_half(x.y), grade_blur_half(x.z));
}

// The step between half floats about a value whose size is `v`.
fn grade_blur_step(v: f32) -> f32 {
    return max(bitcast<f32>(bitcast<u32>(v) & 0x7f800000u) * 0.0009765625, 5.9604645e-8);
}

// The detail of a channel that is sharpened (`blur.rs`, `detail`): what it is in half floats less
// its neighbourhood in half floats, less three steps of half floats either way.
fn grade_blur_detail(kind: f32, kept: f32, held: f32) -> f32 {
    if kind != 2.0 {
        return 0.0;
    }
    let d = kept - held;
    let steps = 3.0 * grade_blur_step(max(abs(kept), abs(held)));
    return d - clamp(d, -steps, steps);
}

fn grade_blur_channel(kind: f32, amount: f32, w: f32, x: f32, near: f32, detail: f32) -> f32 {
    if kind == 1.0 {
        return near;
    }
    if kind == 2.0 {
        return x + amount * w * detail;
    }
    return x;
}

fn grade_blur(x: vec3<f32>, near: vec3<f32>, wide: vec3<f32>, at: u32) -> vec3<f32> {
    let kind = param(at);
    let amount = param(at + 1u);
    let more = param(at + 2u);
    let kept = grade_blur_kept(x);
    let held = grade_blur_kept(near);
    let detail = vec3<f32>(
        grade_blur_detail(kind.x, kept.x, held.x),
        grade_blur_detail(kind.y, kept.y, held.y),
        grade_blur_detail(kind.z, kept.z, held.z),
    );
    var w = 1.0;
    if kind.w != 0.0 {
        let d = max(abs(detail.x), max(abs(detail.y), abs(detail.z)));
        w = grade_blur_core(d, more.x, more.y);
    }
    let made = vec3<f32>(
        grade_blur_channel(kind.x, amount.x, w, x.x, near.x, detail.x),
        grade_blur_channel(kind.y, amount.y, w, x.y, near.y, detail.y),
        grade_blur_channel(kind.z, amount.z, w, x.z, near.z, detail.z),
    );
    if more.w == 0.0 {
        return made;
    }
    return wide + (made - wide) * more.z;
}
