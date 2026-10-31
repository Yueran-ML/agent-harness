// The maths of the nodes of a grade on the card (`grade/mod.rs` says what each formula is and
// why). `grade/maths.rs` does the same on the processor, line for line, and the tests hold the
// two to each other. Before this text the compositor of a grade puts the constants of its
// options: `GRADE_LUMA` (the weights of R, G and B in the luma of the working space) and
// `GRADE_S_CURVE` (whether Contrast is an S-curve).
//
// A corrector reads the block of its settings at `at` (see `grade/layout.rs`); the points of
// its curves are further on, at offsets that the block holds.

const GRADE_PI: f32 = 3.14159265;
const GRADE_TAU: f32 = 6.28318531;
const GRADE_SQRT3_HALF: f32 = 0.866025404;
const GRADE_INV_SQRT3: f32 = 0.577350269;
// Half the width of the blend between two zones of the Log wheels.
const GRADE_LOG_OVERLAP: f32 = 0.05;

fn grade_luma(x: vec3<f32>) -> f32 {
    return GRADE_LUMA.r * x.r + GRADE_LUMA.g * x.g + GRADE_LUMA.b * x.b;
}

// x to the power e, of the sign of x; x itself where e is 1.
fn grade_spow(x: f32, e: f32) -> f32 {
    if (e == 1.0 || x == 0.0) {
        return x;
    }
    return sign(x) * pow(abs(x), e);
}

fn grade_smoothstep(low: f32, high: f32, x: f32) -> f32 {
    let t = clamp((x - low) / (high - low), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

// The saturation of a colour, from 0 (grey) to 1: (max - min) / max.
fn grade_sat(x: vec3<f32>) -> f32 {
    let most = max(x.r, max(x.g, x.b));
    let least = min(x.r, min(x.g, x.b));
    if (most <= 0.0) {
        return 0.0;
    }
    return clamp((most - least) / most, 0.0, 1.0);
}

// The hue of a colour, from 0 to 1: red 0, green 1/3, blue 2/3; 0 for a grey.
fn grade_hue(x: vec3<f32>) -> f32 {
    let a = x.r - 0.5 * (x.g + x.b);
    let b = GRADE_SQRT3_HALF * (x.g - x.b);
    if (a == 0.0 && b == 0.0) {
        return 0.0;
    }
    let h = atan2(b, a) / GRADE_TAU;
    return select(h, h + 1.0, h < 0.0);
}

// A turn of the colour about the grey axis by an angle whose cosine less 1 is `cm1` and whose
// sine is `s`.
fn grade_rotate(x: vec3<f32>, cm1: f32, s: f32) -> vec3<f32> {
    let mean = (x.r + x.g + x.b) / 3.0;
    let across = vec3<f32>(x.b - x.g, x.r - x.b, x.g - x.r) * GRADE_INV_SQRT3;
    return x + (x - vec3<f32>(mean)) * cm1 + across * s;
}

// The colour with its difference from its luma multiplied by `k`.
fn grade_saturate(x: vec3<f32>, k: f32) -> vec3<f32> {
    return x + (x - vec3<f32>(grade_luma(x))) * (k - 1.0);
}

fn grade_lgg(x: f32, lift: f32, e: f32, gain: f32) -> f32 {
    return gain * grade_spow(x + lift * (1.0 - x), e);
}

fn grade_contrast(x: f32, c: f32, p: f32) -> f32 {
    if (!GRADE_S_CURVE) {
        return x + (x - p) * (c - 1.0);
    }
    if (c == 1.0 || x <= 0.0 || x >= 1.0) {
        return x;
    }
    if (x < p) {
        return p * pow(x / p, c);
    }
    return 1.0 - (1.0 - p) * pow((1.0 - x) / (1.0 - p), c);
}

// The value at x of the curve of `count` points from `at` (each its x, its y and its slope),
// held flat beyond its first and last points where `flat`, or carried on along their slopes.
// Between two points it is the cubic of Hermite through them with their slopes; none is 0.
fn grade_curve(at: u32, count: u32, x: f32, flat: bool) -> f32 {
    if (count == 0u) {
        return 0.0;
    }
    let first = param(at);
    if (x <= first.x || count == 1u) {
        return select(first.y + first.z * (x - first.x), first.y, flat);
    }
    let last = param(at + count - 1u);
    if (x >= last.x) {
        return select(last.y + last.z * (x - last.x), last.y, flat);
    }
    var i = 0u;
    for (var j = 1u; j + 1u < count; j = j + 1u) {
        if (param(at + j).x <= x) {
            i = j;
        } else {
            break;
        }
    }
    let p0 = param(at + i);
    let p1 = param(at + i + 1u);
    let dx = p1.x - p0.x;
    let dy = p1.y - p0.y;
    let t = (x - p0.x) / dx;
    let t2 = t * t;
    let t3 = t2 * t;
    let h10 = t3 - 2.0 * t2 + t;
    let h11 = t3 - t2;
    return p0.y + dy * t + h10 * (dx * p0.z - dy) + h11 * (dx * p1.z - dy);
}

// What a custom curve changes x by: its intensity times its value less x.
fn grade_custom(at: u32, offset: f32, count: f32, intensity: f32, x: f32) -> f32 {
    return intensity * (grade_curve(at + u32(offset), u32(count), x, false) - x);
}

fn grade_curves(x: vec3<f32>, at: u32) -> vec3<f32> {
    let intensity = param(at + 12u);
    let offsets = param(at + 13u);
    let counts = param(at + 14u);
    if (param(at + 15u).x != 0.0) {
        return x + vec3<f32>(
            grade_custom(at, offsets.x, counts.x, intensity.y, x.r),
            grade_custom(at, offsets.x, counts.x, intensity.z, x.g),
            grade_custom(at, offsets.x, counts.x, intensity.w, x.b),
        );
    }
    let mix = param(at + 3u).w;
    // What the curves of R, G and B change each channel by.
    let dr = vec3<f32>(
        grade_custom(at, offsets.y, counts.y, intensity.y, x.r),
        grade_custom(at, offsets.y, counts.y, intensity.y, x.g),
        grade_custom(at, offsets.y, counts.y, intensity.y, x.b),
    );
    let dg = vec3<f32>(
        grade_custom(at, offsets.z, counts.z, intensity.z, x.r),
        grade_custom(at, offsets.z, counts.z, intensity.z, x.g),
        grade_custom(at, offsets.z, counts.z, intensity.z, x.b),
    );
    let db = vec3<f32>(
        grade_custom(at, offsets.w, counts.w, intensity.w, x.r),
        grade_custom(at, offsets.w, counts.w, intensity.w, x.g),
        grade_custom(at, offsets.w, counts.w, intensity.w, x.b),
    );
    var o = x + vec3<f32>(dr.r, dg.g, db.b);
    let n = x + dr * GRADE_LUMA.r + dg * GRADE_LUMA.g + db * GRADE_LUMA.b;
    o = o + vec3<f32>(mix * (grade_luma(n) - grade_luma(o)));
    let y = grade_luma(o);
    return o + vec3<f32>(mix * grade_custom(at, offsets.x, counts.x, intensity.x, y));
}

// Offset, which is done as a part of the Log controls (p. 3073), and the Log wheels.
fn grade_log(x0: vec3<f32>, at: u32) -> vec3<f32> {
    let x = x0 + param(at + 4u).rgb;
    let ranges = param(at + 8u);
    let y = grade_luma(x);
    let shadow = 1.0 - grade_smoothstep(ranges.z - GRADE_LOG_OVERLAP, ranges.z + GRADE_LOG_OVERLAP, y);
    let highlight = grade_smoothstep(ranges.w - GRADE_LOG_OVERLAP, ranges.w + GRADE_LOG_OVERLAP, y);
    let midtone = max(1.0 - shadow - highlight, 0.0);
    return x + param(at + 9u).rgb * shadow + param(at + 10u).rgb * midtone
        + param(at + 11u).rgb * highlight;
}

// A corrector up to its LUT (p. 3331): Shadows and Highlights, Color Boost, Temp and Tint,
// Lift, Gamma and Gain with Lum Mix, Contrast, the custom curves, Offset and the Log wheels,
// Hue and Saturation.
fn grade_primary(x0: vec3<f32>, at: u32) -> vec3<f32> {
    var x = x0;
    // The steps that are at their neutral settings (each the identity there) are left out.
    let features = u32(param(at + 26u).x);
    let colour = param(at + 6u);
    if ((features & 1u) != 0u) {
        let zones = param(at + 8u);
        let y = grade_luma(x);
        x = x + vec3<f32>(zones.x * (1.0 - grade_smoothstep(0.0, 0.5, y))
            + zones.y * grade_smoothstep(0.5, 1.0, y));
    }
    if ((features & 2u) != 0u) {
        x = x + (x - vec3<f32>(grade_luma(x))) * (colour.w * (1.0 - grade_sat(x)));
    }
    if ((features & 4u) != 0u) {
        x = x * param(at + 7u).rgb;
    }
    if ((features & 8u) != 0u) {
        let lift = param(at);
        let e = param(at + 1u);
        let gain = param(at + 2u);
        let mean = param(at + 3u);
        var o = vec3<f32>(
            grade_lgg(x.r, lift.r, e.r, gain.r),
            grade_lgg(x.g, lift.g, e.g, gain.g),
            grade_lgg(x.b, lift.b, e.b, gain.b),
        );
        let n = vec3<f32>(
            grade_lgg(x.r, mean.x, mean.y, mean.z),
            grade_lgg(x.g, mean.x, mean.y, mean.z),
            grade_lgg(x.b, mean.x, mean.y, mean.z),
        );
        o = o + vec3<f32>(mean.w * (grade_luma(n) - grade_luma(o)));
        let y0 = grade_luma(o);
        x = o + vec3<f32>(mean.w * (grade_lgg(y0, lift.w, e.w, gain.w) - y0));
    }
    if ((features & 16u) != 0u) {
        let contrast = param(at + 5u);
        x = vec3<f32>(
            grade_contrast(x.r, contrast.x, contrast.y),
            grade_contrast(x.g, contrast.x, contrast.y),
            grade_contrast(x.b, contrast.x, contrast.y),
        );
    }
    if ((features & 32u) != 0u) {
        x = grade_curves(x, at);
    }
    if ((features & 64u) != 0u) {
        x = grade_log(x, at);
    }
    if ((features & 128u) != 0u) {
        x = grade_rotate(x, colour.y, colour.z);
    }
    if ((features & 256u) != 0u) {
        x = grade_saturate(x, colour.x);
    }
    return x;
}

fn grade_clip_low(x: f32, low: f32, width: f32) -> f32 {
    if (low <= 0.0 && width <= 0.0) {
        return x;
    }
    if (width <= 0.0) {
        return max(x, low);
    }
    let knee = low + width;
    if (x >= knee) {
        return x;
    }
    return knee - width * (1.0 - exp((x - knee) / width));
}

fn grade_clip_high(x: f32, high: f32, width: f32) -> f32 {
    if (high >= 1.0 && width <= 0.0) {
        return x;
    }
    if (width <= 0.0) {
        return min(x, high);
    }
    let knee = high - width;
    if (x <= knee) {
        return x;
    }
    return knee + width * (1.0 - exp((knee - x) / width));
}

// A corrector after its LUT: the HSL curves, which read the hue, saturation and luma of the
// picture that came into the node (p. 3122), and Soft Clip.
fn grade_secondary(x0: vec3<f32>, input: vec3<f32>, at: u32) -> vec3<f32> {
    var x = x0;
    let features = u32(param(at + 26u).x);
    if ((features & 512u) != 0u) {
        let offsets = param(at + 16u);
        let offsets2 = param(at + 17u);
        let counts = param(at + 18u);
        let counts2 = param(at + 19u);
        let hue = grade_hue(input);
        let sat = grade_sat(input);
        let lum = clamp(grade_luma(input), 0.0, 1.0);
        if (counts.x > 0.0) {
            let turn = grade_curve(at + u32(offsets.x), u32(counts.x), hue, true) * GRADE_PI;
            x = grade_rotate(x, cos(turn) - 1.0, sin(turn));
        }
        let k = (1.0 + grade_curve(at + u32(offsets.y), u32(counts.y), hue, true))
            * (1.0 + grade_curve(at + u32(offsets.w), u32(counts.w), lum, true))
            * (1.0 + grade_curve(at + u32(offsets2.x), u32(counts2.x), sat, true));
        x = grade_saturate(x, k);
        let light = grade_curve(at + u32(offsets.z), u32(counts.z), hue, true) * sat
            + grade_curve(at + u32(offsets2.y), u32(counts2.y), sat, true);
        x = x + vec3<f32>(grade_luma(x) * light);
    }
    if ((features & 1024u) != 0u) {
        let low = param(at + 20u);
        let high = param(at + 21u);
        let low_width = param(at + 22u);
        let high_width = param(at + 23u);
        x = vec3<f32>(
            grade_clip_high(grade_clip_low(x.r, low.r, low_width.r), high.r, high_width.r),
            grade_clip_high(grade_clip_low(x.g, low.g, low_width.g), high.g, high_width.g),
            grade_clip_high(grade_clip_low(x.b, low.b, low_width.b), high.b, high_width.b),
        );
    }
    return x;
}

// A key through the settings `p` of a key input or output: inverted where `p.x` is not 0, its
// contrast moved by the offset `p.z` (white held), and multiplied by the gain `p.y`.
fn grade_key(k: f32, p: vec4<f32>) -> f32 {
    let a = select(k, 1.0 - k, p.x != 0.0);
    return clamp((a + p.z * (1.0 - a)) * p.y, 0.0, 1.0);
}

// The key of a key mixer so far, and the next input's (adjusted), in Matte mode (`matte` not
// 0: where both are) or Mask mode (either).
fn grade_key_combine(sofar: f32, k: f32, matte: f32) -> f32 {
    return select(min(sofar + k, 1.0), sofar * k, matte != 0.0);
}

fn grade_clip_colour(c: vec3<f32>) -> vec3<f32> {
    let l = grade_luma(c);
    let least = min(c.r, min(c.g, c.b));
    let most = max(c.r, max(c.g, c.b));
    var o = c;
    if (least < 0.0 && l - least > 0.0) {
        o = vec3<f32>(l) + (o - vec3<f32>(l)) * (l / (l - least));
    }
    if (most > 1.0 && most - l > 0.0) {
        o = vec3<f32>(l) + (o - vec3<f32>(l)) * ((1.0 - l) / (most - l));
    }
    return o;
}

fn grade_set_lum(c: vec3<f32>, l: f32) -> vec3<f32> {
    return grade_clip_colour(c + vec3<f32>(l - grade_luma(c)));
}

fn grade_chroma(c: vec3<f32>) -> f32 {
    return max(c.r, max(c.g, c.b)) - min(c.r, min(c.g, c.b));
}

fn grade_set_sat(c: vec3<f32>, s: f32) -> vec3<f32> {
    let range = grade_chroma(c);
    if (range <= 0.0) {
        return vec3<f32>(0.0);
    }
    return (c - vec3<f32>(min(c.r, min(c.g, c.b)))) * (s / range);
}

fn grade_dodge(b: f32, f: f32) -> f32 {
    if (b <= 0.0) {
        return 0.0;
    }
    if (f >= 1.0) {
        return 1.0;
    }
    return min(1.0, b / (1.0 - f));
}

fn grade_burn(b: f32, f: f32) -> f32 {
    if (b >= 1.0) {
        return 1.0;
    }
    if (f <= 0.0) {
        return 0.0;
    }
    return 1.0 - min(1.0, (1.0 - b) / f);
}

fn grade_soft_light(b: f32, f: f32) -> f32 {
    if (f <= 0.5) {
        return b - (1.0 - 2.0 * f) * b * (1.0 - b);
    }
    var d = sqrt(max(b, 0.0));
    if (b <= 0.25) {
        d = ((16.0 * b - 12.0) * b + 4.0) * b;
    }
    return b + (2.0 * f - 1.0) * (d - b);
}

fn grade_hard_light(b: f32, f: f32) -> f32 {
    if (f <= 0.5) {
        return 2.0 * b * f;
    }
    return 1.0 - 2.0 * (1.0 - b) * (1.0 - f);
}

fn grade_vivid_light(b: f32, f: f32) -> f32 {
    if (f <= 0.5) {
        return grade_burn(b, 2.0 * f);
    }
    return grade_dodge(b, 2.0 * f - 1.0);
}

fn grade_pin_light(b: f32, f: f32) -> f32 {
    if (f <= 0.5) {
        return min(b, 2.0 * f);
    }
    return max(b, 2.0 * f - 1.0);
}

fn grade_divide(b: f32, f: f32) -> f32 {
    if (f > 0.0) {
        return b / f;
    }
    return select(0.0, 1.0, b > 0.0);
}

// A channel of `f` laid over `b` in the separable mode `mode`.
fn grade_blend(mode: u32, b: f32, f: f32) -> f32 {
    var r = f;
    switch (mode) {
        case 1u: { r = b + f; }
        case 3u: { r = grade_burn(b, f); }
        case 4u: { r = grade_dodge(b, f); }
        case 5u: { r = min(b, f); }
        case 7u: { r = abs(b - f); }
        case 8u: { r = grade_divide(b, f); }
        case 9u: { r = b + f - 2.0 * b * f; }
        case 10u: { r = select(0.0, 1.0, b + f >= 1.0); }
        case 11u: { r = grade_hard_light(b, f); }
        case 13u: { r = max(b, f); }
        case 15u: { r = b + f - 1.0; }
        case 16u: { r = b + f; }
        case 17u: { r = b + 2.0 * f - 1.0; }
        case 19u: { r = b * f; }
        case 20u: { r = grade_hard_light(f, b); }
        case 21u: { r = grade_pin_light(b, f); }
        case 23u: { r = b + f - b * f; }
        case 24u: { r = grade_soft_light(b, f); }
        case 25u: { r = b - f; }
        case 26u: { r = grade_vivid_light(b, f); }
        default: {}
    }
    return r;
}

// `f` laid over `b` in the composite mode `mode` (numbered as `grade/maths.rs` numbers them).
fn grade_composite(mode: u32, b: vec3<f32>, f: vec3<f32>) -> vec3<f32> {
    var r = vec3<f32>(
        grade_blend(mode, b.r, f.r),
        grade_blend(mode, b.g, f.g),
        grade_blend(mode, b.b, f.b),
    );
    switch (mode) {
        case 2u: { r = grade_set_lum(f, grade_luma(b)); }
        case 6u: { r = select(b, f, grade_luma(f) < grade_luma(b)); }
        case 12u: { r = grade_set_lum(grade_set_sat(f, grade_chroma(b)), grade_luma(b)); }
        case 14u: { r = select(b, f, grade_luma(f) > grade_luma(b)); }
        case 18u: { r = grade_set_lum(b, grade_luma(f)); }
        case 22u: { r = grade_set_lum(grade_set_sat(b, grade_chroma(f)), grade_luma(b)); }
        default: {}
    }
    return r;
}
