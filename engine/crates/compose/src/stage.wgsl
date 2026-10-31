// A stage of colour over the whole canvas (`color.rs`): the timeline grade, or the output
// transform. Each pixel of the canvas, which is opaque, is read as it is, and written into
// another texture of its size, its colour changed.

@group(0) @binding(0) var canvas: texture_2d<f32>;

// The lines from "colour: begin" to "colour: end" are put in place by those of the stage.
// colour: begin
const colour_identity: bool = true;
fn colour(rgb: vec3<f32>) -> vec3<f32> {
    return rgb;
}
// colour: end

// One triangle that covers the target.
@vertex
fn vs(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let x = f32((index << 1u) & 2u);
    let y = f32(index & 2u);
    return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

@fragment
fn fs(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let texel = textureLoad(canvas, vec2<i32>(position.xy), 0);
    return vec4<f32>(colour(texel.rgb), texel.a);
}
