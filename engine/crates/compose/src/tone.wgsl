// The DRTs of colour management (`tone.rs`, whose documentation says what each does), on linear
// light where 1.0 is `unit` nits. The curve maps a most of `peak` nits down to `top`: unchanged
// to the knee, then two cubics of Hermite through the knots of RM20's rule (pp. 232–233), then
// straight on; mapping up is its inverse, found by halving.

fn tone_hermite(at: f32, x0: f32, x1: f32, y0: f32, y1: f32, d0: f32, d1: f32) -> f32 {
    let h = x1 - x0;
    let t = (at - x0) / h;
    let t2 = t * t;
    let t3 = t2 * t;
    return (2.0 * t3 - 3.0 * t2 + 1.0) * y0 + (t3 - 2.0 * t2 + t) * h * d0
        + (-2.0 * t3 + 3.0 * t2) * y1 + (t3 - t2) * h * d1;
}

// `curve`: the knee, the top, the peak, and the slope at the top; `slope_peak` that at the peak.
fn tone_compress(x: f32, curve: vec4<f32>, slope_peak: f32) -> f32 {
    let knee = curve.x;
    let top = curve.y;
    let peak = curve.z;
    let at_top = 0.9 * top;
    if x <= knee {
        return x;
    }
    if x <= top {
        return tone_hermite(x, knee, top, knee, at_top, 1.0, curve.w);
    }
    if x <= peak {
        return tone_hermite(x, top, peak, at_top, top, curve.w, slope_peak);
    }
    return top + slope_peak * (x - peak);
}

fn tone_expand(x: f32, curve: vec4<f32>, slope_peak: f32) -> f32 {
    let knee = curve.x;
    let top = curve.y;
    let peak = curve.z;
    if x <= knee {
        return x;
    }
    if x >= top {
        return peak + (x - top) / slope_peak;
    }
    var low = knee;
    var high = peak;
    for (var i = 0u; i < 32u; i++) {
        let middle = 0.5 * (low + high);
        if tone_compress(middle, curve, slope_peak) < x {
            low = middle;
        } else {
            high = middle;
        }
    }
    return 0.5 * (low + high);
}

fn tone_map(x: f32, curve: vec4<f32>, how: vec4<f32>) -> f32 {
    if how.z > 0.5 {
        return tone_expand(x, curve, how.x);
    }
    return tone_compress(x, curve, how.x);
}

fn tone_smoothstep(low: f32, high: f32, x: f32) -> f32 {
    let t = clamp((x - low) / (high - low), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

// `how`: the slope at the peak, the kind (0 Simple, 1 Luminance Mapping, 2 DaVinci, 3 Saturation
// Preserving), 1 to map up, and the nits of 1.0. `luma`: the coefficients of luminance. `sat`:
// the Start and Limit of the Sat. Rolloff, and the nits below which DaVinci desaturates shadows.
fn tone_drt(rgb: vec3<f32>, curve: vec4<f32>, how: vec4<f32>, luma: vec4<f32>, sat: vec4<f32>) -> vec3<f32> {
    let unit = how.w;
    let up = how.z > 0.5;
    if how.y < 0.5 {
        return vec3<f32>(
            tone_map(rgb.r * unit, curve, how),
            tone_map(rgb.g * unit, curve, how),
            tone_map(rgb.b * unit, curve, how),
        ) / unit;
    }
    let luminance = dot(rgb, luma.xyz) * unit;
    if luminance <= 0.0 {
        return rgb;
    }
    let out = tone_map(luminance, curve, how);
    let scale = out / luminance;
    var saturation = 1.0;
    if how.y > 1.5 && how.y < 2.5 {
        var highlights = 1.0;
        if !up {
            highlights = 1.0 - tone_smoothstep(curve.y, curve.z, luminance);
        }
        saturation = highlights * (0.75 + 0.25 * tone_smoothstep(0.0, sat.z, luminance));
    } else if how.y > 2.5 {
        var measured = luminance;
        if up {
            measured = out;
        }
        saturation = 1.0 - tone_smoothstep(sat.x, sat.y, measured);
    }
    let grey = vec3<f32>(out / unit);
    return grey + (rgb * scale - grey) * saturation;
}
