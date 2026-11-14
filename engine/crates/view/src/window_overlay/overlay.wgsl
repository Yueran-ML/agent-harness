struct OverlayGlobals {
    dest: vec4<f32>,
    // Canvas pixels per device pixel (xy), quantized by the same CPU function used for hit,
    // and the surface size in device pixels (zw).
    sampling: vec4<f32>,
    // Device stroke width.
    style: vec4<f32>,
    wipe: WipeParams,
}
struct OverlayPrimitive {
    inverse: vec4<f32>,
    // Device centre, scale removed from inverse, kind.
    shape: vec4<f32>,
    colour: vec4<f32>,
    // The device quad that holds every covered pixel: origin and first axis, second axis.
    quad: vec4<f32>,
    axis: vec4<f32>,
}
@group(0) @binding(0) var<uniform> overlay: OverlayGlobals;
@group(0) @binding(1) var<storage, read> primitives: array<OverlayPrimitive>;

struct Covered {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) index: u32,
}

// One instance per primitive, two triangles over its own quad, drawn in list order.
@vertex
fn vs_overlay(@builtin(vertex_index) corner: u32, @builtin(instance_index) index: u32) -> Covered {
    let primitive = primitives[index];
    let s = select(0.0, 1.0, corner == 1u || corner == 4u || corner == 5u);
    let t = select(0.0, 1.0, corner == 2u || corner == 3u || corner == 5u);
    let device = primitive.quad.xy + s * primitive.quad.zw + t * primitive.axis.xy;
    var out: Covered;
    out.position = vec4<f32>(
        device.x / overlay.sampling.z * 2.0 - 1.0,
        1.0 - device.y / overlay.sampling.w * 2.0,
        0.0,
        1.0,
    );
    out.index = index;
    return out;
}

@fragment
fn fs_overlay(covered: Covered) -> @location(0) vec4<f32> {
    let p = covered.position;
    let primitive = primitives[covered.index];
    // Clip before division, including tiny legal viewport sizes. No controls in black bars.
    let offset = p.xy - overlay.dest.xy;
    if (any(offset < vec2<f32>(0.0)) || any(offset >= overlay.dest.zw)) { discard; }
    // The ratio is supplied, avoiding approximate GPU division at exact texel centres.
    let canvas = offset * overlay.sampling.xy;
    if (!sample_has_current(overlay.wipe, canvas)) { discard; }
    let delta = p.xy - primitive.shape.xy;
    let kind = primitive.shape.w;
    let width = overlay.style.x;
    let inv = primitive.inverse;
    if (kind > 0.5) {
        var distance = abs(dot(delta, inv.xy));
        var reach = width * 0.5;
        if (kind > 1.5 && kind < 2.5) {
            distance = length(delta);
            reach = primitive.shape.z;
        } else if (kind > 2.5) {
            let t = clamp(dot(delta, inv.xy) / dot(inv.xy, inv.xy), 0.0, 1.0);
            distance = length(delta - t * inv.xy);
        }
        let alpha = clamp(reach + 0.5 - distance, 0.0, 1.0);
        if (alpha <= 0.0) { discard; }
        return vec4<f32>(primitive.colour.rgb, alpha);
    }
    let scale = primitive.shape.z;
    let magnitude = max(scale, max(abs(delta.x), abs(delta.y)));
    let at = delta / magnitude;
    let local = vec2<f32>(inv.x*at.x + inv.y*at.y, inv.z*at.x + inv.w*at.y);
    let radius = length(local);
    if (radius == 0.0 || width <= 0.0) { discard; }
    let direction = local / radius;
    let gradient = vec2<f32>(inv.x*direction.x + inv.z*direction.y,
        inv.y*direction.x + inv.w*direction.y);
    let distance = abs(radius - scale/magnitude) / length(gradient) * magnitude;
    let alpha = clamp(width*0.5 + 0.5 - distance, 0.0, 1.0);
    if (alpha <= 0.0) { discard; }
    return vec4<f32>(primitive.colour.rgb, alpha);
}
