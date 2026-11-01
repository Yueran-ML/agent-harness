struct OverlayParams {
    inverse: vec4<f32>,
    // Device centre, scale removed from inverse, device stroke width.
    shape: vec4<f32>,
    dest: vec4<f32>,
    // Canvas pixels per device pixel, quantized by the same CPU function used for hit.
    sampling: vec4<f32>,
    colour: vec4<f32>,
    wipe: WipeParams,
}
@group(0) @binding(0) var<uniform> overlay: OverlayParams;

@vertex
fn vs_overlay(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let x = f32((index << 1u) & 2u);
    let y = f32(index & 2u);
    return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

@fragment
fn fs_overlay(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    // Clip before division, including tiny legal viewport sizes. No controls in black bars.
    let offset = p.xy - overlay.dest.xy;
    if (any(offset < vec2<f32>(0.0)) || any(offset >= overlay.dest.zw)) { discard; }
    // The ratio is supplied, avoiding approximate GPU division at exact texel centres.
    let canvas = offset * overlay.sampling.xy;
    if (!sample_has_current(overlay.wipe, canvas)) { discard; }
    let delta = p.xy - overlay.shape.xy;
    let kind = overlay.colour.w;
    let inv = overlay.inverse;
    if (kind > 0.5) {
        var distance = abs(dot(delta, inv.xy));
        var reach = overlay.shape.w * 0.5;
        if (kind > 1.5 && kind < 2.5) {
            distance = length(delta);
            reach = overlay.shape.z;
        } else if (kind > 2.5) {
            let t = clamp(dot(delta, inv.xy) / dot(inv.xy, inv.xy), 0.0, 1.0);
            distance = length(delta - t * inv.xy);
        }
        let alpha = clamp(reach + 0.5 - distance, 0.0, 1.0);
        if (alpha <= 0.0) { discard; }
        return vec4<f32>(overlay.colour.rgb, alpha);
    }
    let scale = overlay.shape.z;
    let magnitude = max(scale, max(abs(delta.x), abs(delta.y)));
    let at = delta / magnitude;
    let local = vec2<f32>(inv.x*at.x + inv.y*at.y, inv.z*at.x + inv.w*at.y);
    let radius = length(local);
    if (radius == 0.0 || overlay.shape.w <= 0.0) { discard; }
    let direction = local / radius;
    let gradient = vec2<f32>(inv.x*direction.x + inv.z*direction.y,
        inv.y*direction.x + inv.w*direction.y);
    let distance = abs(radius - scale/magnitude) / length(gradient) * magnitude;
    let alpha = clamp(overlay.shape.w*0.5 + 0.5 - distance, 0.0, 1.0);
    if (alpha <= 0.0) { discard; }
    return vec4<f32>(overlay.colour.rgb, alpha);
}
