// A pass of the filter of a neighbourhood (`neighbourhood.rs`, which does the same on the
// processor): each pixel of the target is the weighed mean of the pixels of the source along a
// row (across) or a column (down) about its centre, which is `ratio` pixels of the source for a
// pixel of the target along the pass (more than 1 where the filter is taken at fewer points),
// and lies on the same column (or row) across it. The weights are a Gaussian of a standard
// deviation of `reach` pixels of the source, or 1 within `reach` of the centre (a box). The
// pixels beyond the edges are those at the edges.
//
// Places are counted in floats, and the loop in u32: the compiler of Direct3D 12 crashes on
// some arithmetic of signed integers with constants.

struct Pass {
    // x: 1 across, 0 down; y: 0 a Gaussian, 1 a box; z: its reach; w: how many pixels each side
    // of the centre are read.
    shape: vec4<f32>,
    // x, y: the width and height of the source; z: the ratio.
    sizes: vec4<f32>,
};

@group(0) @binding(0) var<uniform> filter_pass: Pass;
@group(0) @binding(1) var source: texture_2d<f32>;

// One triangle that covers the target.
@vertex
fn vs(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let x = f32((index << 1u) & 2u);
    let y = f32(index & 2u);
    return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

@fragment
fn fs(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let shape = filter_pass.shape;
    let sizes = filter_pass.sizes;
    let across = shape.x > 0.5;
    let gaussian = shape.y < 0.5;
    let pixel = floor(position.xy);
    var centre = (pixel.y + 0.5) * sizes.z;
    var row = pixel.x;
    var last = sizes.y - 1.0;
    if across {
        centre = (pixel.x + 0.5) * sizes.z;
        row = pixel.y;
        last = sizes.x - 1.0;
    }
    let first = floor(centre - 0.5) - shape.w;
    let count = u32(2.0 * shape.w + 2.0);
    let spread = shape.z * shape.z;
    var sum = vec4<f32>(0.0);
    var weights = 0.0;
    for (var k = 0u; k < count; k = k + 1u) {
        let index = first + f32(k);
        let d = index + 0.5 - centre;
        var weight = select(0.0, 1.0, abs(d) <= shape.z);
        if gaussian {
            weight = exp(-0.5 * d * d / spread);
        }
        let at = i32(clamp(index, 0.0, last));
        var place = vec2<i32>(i32(row), at);
        if across {
            place = vec2<i32>(at, i32(row));
        }
        sum = sum + textureLoad(source, place, 0) * weight;
        weights = weights + weight;
    }
    return sum / weights;
}
