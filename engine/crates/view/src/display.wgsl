// The display mapping of the view: the picture, drawn as before into a texture of half floats of
// the size of the window, mapped onto the surface. `display.rs` has the maths, its reference on
// the processor (`Mapping::map`), and what each value of `Mapping` is.
//
// Every channel is written out by itself: a vector indexed in a loop crashes the compiler of
// Direct3D 12.

struct Mapping {
    // x: the content (0 SDR, 1 PQ, 2 HLG); y: the surface (0 SDR, 1 scRGB); z: the scale of SDR
    // white in scRGB; w: the system gamma of HLG.
    kind: vec4<f32>,
    // x: the knee, y: the peak of the monitor, z: the peak of the content, w: the nits of 1.0
    // of the surface.
    levels: vec4<f32>,
}

@group(0) @binding(0) var drawn: texture_2d<f32>;
@group(0) @binding(1) var<uniform> mapping: Mapping;

@vertex fn vs(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let positions = array(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    return vec4(positions[index], 0.0, 1.0);
}

fn srgb_eotf(value: f32) -> f32 {
    if (value <= 0.04045) {
        return value / 12.92;
    }
    return pow((value + 0.055) / 1.055, 2.4);
}

fn srgb_oetf(light: f32) -> f32 {
    if (light <= 0.0031308) {
        return light * 12.92;
    }
    return 1.055 * pow(light, 1.0 / 2.4) - 0.055;
}

fn pq_eotf(code: f32) -> f32 {
    let m1 = 2610.0 / 16384.0;
    let m2 = 2523.0 / 4096.0 * 128.0;
    let c1 = 3424.0 / 4096.0;
    let c2 = 2413.0 / 4096.0 * 32.0;
    let c3 = 2392.0 / 4096.0 * 32.0;
    let power = pow(clamp(code, 0.0, 1.0), 1.0 / m2);
    return 10000.0 * pow(max(power - c1, 0.0) / (c2 - c3 * power), 1.0 / m1);
}

fn hlg_inverse_oetf(code: f32) -> f32 {
    let c = clamp(code, 0.0, 1.0);
    if (c <= 0.5) {
        return c * c / 3.0;
    }
    return (exp((c - 0.55991073) / 0.17883277) + 0.28466892) / 12.0;
}

fn roll_off(nits: f32) -> f32 {
    let knee = mapping.levels.x;
    let peak = mapping.levels.y;
    let top = mapping.levels.z;
    if (nits <= knee) {
        return nits;
    }
    if (top <= peak) {
        return min(nits, peak);
    }
    let a = peak - knee;
    let w = (top - knee) / a;
    let x = (nits - knee) / a;
    if (x >= w) {
        return peak;
    }
    return knee + a * x * (1.0 + x / (w * w)) / (1.0 + x);
}

fn map(rgb: vec3<f32>) -> vec3<f32> {
    let unit = mapping.levels.w;
    if (mapping.kind.x < 0.5) {
        // SDR content, which is mapped only into scRGB: its white onto the white of SDR.
        let v = clamp(rgb, vec3(0.0), vec3(1.0));
        let light = vec3(srgb_eotf(v.r), srgb_eotf(v.g), srgb_eotf(v.b));
        return light * mapping.kind.z;
    }
    var wide: vec3<f32>;
    if (mapping.kind.x < 1.5) {
        wide = vec3(pq_eotf(rgb.r), pq_eotf(rgb.g), pq_eotf(rgb.b));
    } else {
        let scene = vec3(hlg_inverse_oetf(rgb.r), hlg_inverse_oetf(rgb.g), hlg_inverse_oetf(rgb.b));
        let luma = 0.2627 * scene.r + 0.6780 * scene.g + 0.0593 * scene.b;
        var gain = 0.0;
        if (luma > 0.0) {
            gain = mapping.levels.z * pow(luma, mapping.kind.w - 1.0);
        }
        wide = scene * gain;
    }
    // BT.2020 into BT.709 (BT.2087).
    var nits = vec3(
        1.660491 * wide.r - 0.587641 * wide.g - 0.072850 * wide.b,
        -0.124550 * wide.r + 1.132900 * wide.g - 0.008349 * wide.b,
        -0.018151 * wide.r - 0.100579 * wide.g + 1.118730 * wide.b,
    );
    let largest = max(nits.r, max(nits.g, nits.b));
    if (largest > 0.0) {
        nits = nits * (roll_off(largest) / largest);
    }
    let out = nits / unit;
    if (mapping.kind.y < 0.5) {
        let v = clamp(out, vec3(0.0), vec3(1.0));
        return vec3(srgb_oetf(v.r), srgb_oetf(v.g), srgb_oetf(v.b));
    }
    return out;
}

@fragment fn fs(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let texel = textureLoad(drawn, vec2<i32>(position.xy), 0);
    return vec4(map(texel.rgb), 1.0);
}
