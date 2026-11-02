// The Blur palette of a corrector (pp. 3454-3456), as `blur.rs` does it on the processor, line for
// line: `x` what the node made, `near` and `wide` each channel of it filtered by its near and
// wide neighbourhoods, and the block of the palette at `at` (`blur.rs`, `BLOCK`). Channels are
// written out one by one: the compiler of Direct3D 12 crashes on vectors indexed in loops.

fn grade_blur_core(d: f32, level: f32, soft: f32) -> f32 {
    if soft > 0.0 {
        return smoothstep(level, level + soft, d);
    }
    if d > level || level <= 0.0 {
        return 1.0;
    }
    return 0.0;
}

fn grade_blur_detail(kind: f32, x: f32, near: f32) -> f32 {
    if kind == 2.0 {
        return abs(x - near);
    }
    return 0.0;
}

fn grade_blur_channel(kind: f32, amount: f32, w: f32, x: f32, near: f32) -> f32 {
    if kind == 1.0 {
        return near;
    }
    if kind == 2.0 {
        return x + amount * w * (x - near);
    }
    return x;
}

fn grade_blur(x: vec3<f32>, near: vec3<f32>, wide: vec3<f32>, at: u32) -> vec3<f32> {
    let kind = param(at);
    let amount = param(at + 1u);
    let more = param(at + 2u);
    var w = 1.0;
    if kind.w != 0.0 {
        let d = max(grade_blur_detail(kind.x, x.x, near.x),
            max(grade_blur_detail(kind.y, x.y, near.y), grade_blur_detail(kind.z, x.z, near.z)));
        w = grade_blur_core(d, more.x, more.y);
    }
    let made = vec3<f32>(
        grade_blur_channel(kind.x, amount.x, w, x.x, near.x),
        grade_blur_channel(kind.y, amount.y, w, x.y, near.y),
        grade_blur_channel(kind.z, amount.z, w, x.z, near.z),
    );
    if more.w == 0.0 {
        return made;
    }
    return wide + (made - wide) * more.z;
}
