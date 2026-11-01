// Used only for a custom viewport. Fit retains the shared shader's original fs entry point.
@fragment
fn fs_viewport(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let relative = position.xy - params.dest.xy;
    if any(relative < vec2(0.0)) || any(relative >= params.dest.zw) {
        return vec4(0.0, 0.0, 0.0, 1.0);
    }
    let shown = relative / params.dest.zw;
    return vec4(rgb_at(shown), 1.0);
}
