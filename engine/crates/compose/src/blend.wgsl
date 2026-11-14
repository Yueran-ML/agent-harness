// A layer blended with what is under it by its composite mode (`compose.rs`): these lines are
// put in place of those from "blend: begin" to "blend: end" of `layer.wgsl` and of `yuv.wgsl`,
// for a layer whose mode is not Normal and that has something under it. Before them is put the
// maths of the modes, which is that of the Layer Mixer of a grade (the end of
// `grade/corrector.wgsl`, its names made `layer_…`): `layer_composite`, and `LAYER_LUMA`, the
// weights of R, G and B in luma that it reads.
//
// What is under the layer is the canvas as it was before the layer was drawn: a texture cannot
// be read while it is drawn into, so the canvas is copied into this one first. It holds R'G'B'A
// premultiplied, as the canvas does, and is clear where no layer lies (a frame with such a
// layer is cleared to nothing, and made opaque once its layers are on it, `opaque.wgsl`).
@group(2) @binding(0) var backdrop: texture_2d<f32>;

// `over`, the colour of the layer premultiplied by its alpha `As` (its own, times its coverage,
// its crop and its opacity), blended with the canvas at `position`, of colour `Cb` and alpha
// `Ab`, as the W3C lays a blended colour over a backdrop (Compositing and Blending Level 1,
// "Blending"; `merge.rs` does the same on the processor): the colour of the layer where nothing
// is under it, and the mode's where something is,
//
//     Cs' = (1 - Ab) Cs + Ab B(Cb, Cs)
//
// premultiplied by `As` again. The pipeline then lays it over the canvas as it lays any layer,
// `Cs' As + (1 - As) Cb Ab`, which is the W3C's result, premultiplied. Colours are blended
// unpremultiplied, as a grade works on them. Where nothing is under the layer it is laid as it
// is, to the bit.
fn laid(over: vec4<f32>, position: vec4<f32>) -> vec4<f32> {
    let under = textureLoad(backdrop, vec2<i32>(position.xy), 0);
    let below = clamp(under.a, 0.0, 1.0);
    if over.a <= 0.0 || below <= 0.0 {
        return over;
    }
    LAYER_LUMA = params.blend.yzw;
    let source = over.rgb / over.a;
    let mixed = layer_composite(u32(params.blend.x + 0.5), under.rgb / under.a, source);
    return vec4<f32>(((1.0 - below) * source + below * mixed) * over.a, over.a);
}
