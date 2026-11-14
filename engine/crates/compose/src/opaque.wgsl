// The canvas made opaque once its layers are on it, in a frame where a layer is blended with
// what is under it (`compose.rs`). Such a frame is cleared to nothing rather than to black, so
// that the alpha of the canvas says where layers lie under one that is blended; laid over black
// its colours are what they are (they are premultiplied), and its alpha is 1. So only alpha is
// written, by the write mask of the pipeline: 1, everywhere.

// One triangle that covers the target.
@vertex
fn vs(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let x = f32((index << 1u) & 2u);
    let y = f32(index & 2u);
    return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

@fragment
fn fs() -> @location(0) vec4<f32> {
    return vec4<f32>(0.0, 0.0, 0.0, 1.0);
}
