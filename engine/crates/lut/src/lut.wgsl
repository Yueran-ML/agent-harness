// columbina-lut: LUTs read on the card, point by point with textureLoad (no sampler), so that
// the card computes what the reference of the crate computes on the processor, in f32.
//
// Each table comes with two vec4s of parameters:
//   lo:  the lower bound of its domain in each channel, and in w the index of its last point;
//   inv: one over the width of its domain in each channel (w unused).
// Input is clamped to the domain, then placed between points 0 and `last`.
//
// A 1D table is a texture_2d of rows of `width` points (the 1D textures of a card are too short
// for 65536), point i at (i % width, i / width), R, G and B in .rgb.
// A cube is a texture_3d with red along x, green along y, blue along z, as the rows of a .cube.
// Hardware filtering is not used: it weighs with as few as 8 bits of fraction, and it is not
// tetrahedral; the corners are loaded, and weighed here.

fn lut_place(rgb: vec3<f32>, lo: vec4<f32>, inv: vec4<f32>) -> vec3<f32> {
    return clamp((rgb - lo.xyz) * inv.xyz, vec3<f32>(0.0), vec3<f32>(1.0)) * lo.w;
}

// The table at `x` (0 to `last`), all of its channels, of which the caller takes one. (Not a
// loop over the channels indexing vectors by its counter: the shader compiler of Direct3D 12
// crashed on that, taking the process with it.)
fn lut_1d_at(table: texture_2d<f32>, x: f32, last: i32) -> vec4<f32> {
    let width = i32(textureDimensions(table).x);
    let i = clamp(i32(floor(x)), 0, last - 1);
    let j = i + 1;
    let a = textureLoad(table, vec2<i32>(i % width, i / width), 0);
    let b = textureLoad(table, vec2<i32>(j % width, j / width), 0);
    return a + (b - a) * (x - f32(i));
}

fn lut_1d(table: texture_2d<f32>, rgb: vec3<f32>, lo: vec4<f32>, inv: vec4<f32>) -> vec3<f32> {
    let last = i32(lo.w);
    let x = lut_place(rgb, lo, inv);
    return vec3<f32>(
        lut_1d_at(table, x.x, last).r,
        lut_1d_at(table, x.y, last).g,
        lut_1d_at(table, x.z, last).b,
    );
}

struct LutCell {
    base: vec3<i32>,
    f: vec3<f32>,
}

// The lowest corner of the cell about the input, and where in the cell it is; the top of the
// domain is the top of the last cell.
fn lut_cell(rgb: vec3<f32>, lo: vec4<f32>, inv: vec4<f32>) -> LutCell {
    let last = i32(lo.w);
    let x = lut_place(rgb, lo, inv);
    let base = clamp(vec3<i32>(floor(x)), vec3<i32>(0), vec3<i32>(last - 1));
    return LutCell(base, x - vec3<f32>(base));
}

// Four corners: those of the tetrahedron about the input, of the six that share the grey
// diagonal of the cell. The same branches and ties as Lut3d::tetrahedral.
fn lut_3d_tetrahedral(cube: texture_3d<f32>, rgb: vec3<f32>, lo: vec4<f32>, inv: vec4<f32>) -> vec3<f32> {
    let cell = lut_cell(rgb, lo, inv);
    let b = cell.base;
    let f = cell.f;
    var w: vec4<f32>;
    var p: vec3<i32>;
    var q: vec3<i32>;
    if f.x > f.y {
        if f.y > f.z {
            w = vec4<f32>(1.0 - f.x, f.x - f.y, f.y - f.z, f.z);
            p = vec3<i32>(1, 0, 0);
            q = vec3<i32>(1, 1, 0);
        } else if f.x > f.z {
            w = vec4<f32>(1.0 - f.x, f.x - f.z, f.z - f.y, f.y);
            p = vec3<i32>(1, 0, 0);
            q = vec3<i32>(1, 0, 1);
        } else {
            w = vec4<f32>(1.0 - f.z, f.z - f.x, f.x - f.y, f.y);
            p = vec3<i32>(0, 0, 1);
            q = vec3<i32>(1, 0, 1);
        }
    } else if f.z > f.y {
        w = vec4<f32>(1.0 - f.z, f.z - f.y, f.y - f.x, f.x);
        p = vec3<i32>(0, 0, 1);
        q = vec3<i32>(0, 1, 1);
    } else if f.z > f.x {
        w = vec4<f32>(1.0 - f.y, f.y - f.z, f.z - f.x, f.x);
        p = vec3<i32>(0, 1, 0);
        q = vec3<i32>(0, 1, 1);
    } else {
        w = vec4<f32>(1.0 - f.y, f.y - f.x, f.x - f.z, f.z);
        p = vec3<i32>(0, 1, 0);
        q = vec3<i32>(1, 1, 0);
    }
    return w.x * textureLoad(cube, b, 0).rgb
        + w.y * textureLoad(cube, b + p, 0).rgb
        + w.z * textureLoad(cube, b + q, 0).rgb
        + w.w * textureLoad(cube, b + vec3<i32>(1, 1, 1), 0).rgb;
}

// All eight corners, weighed along red, then green, then blue, as Lut3d::trilinear.
fn lut_3d_trilinear(cube: texture_3d<f32>, rgb: vec3<f32>, lo: vec4<f32>, inv: vec4<f32>) -> vec3<f32> {
    let cell = lut_cell(rgb, lo, inv);
    let b = cell.base;
    let f = cell.f;
    let x00 = mix(textureLoad(cube, b, 0).rgb, textureLoad(cube, b + vec3<i32>(1, 0, 0), 0).rgb, f.x);
    let x10 = mix(textureLoad(cube, b + vec3<i32>(0, 1, 0), 0).rgb, textureLoad(cube, b + vec3<i32>(1, 1, 0), 0).rgb, f.x);
    let x01 = mix(textureLoad(cube, b + vec3<i32>(0, 0, 1), 0).rgb, textureLoad(cube, b + vec3<i32>(1, 0, 1), 0).rgb, f.x);
    let x11 = mix(textureLoad(cube, b + vec3<i32>(0, 1, 1), 0).rgb, textureLoad(cube, b + vec3<i32>(1, 1, 1), 0).rgb, f.x);
    return mix(mix(x00, x10, f.y), mix(x01, x11, f.y), f.z);
}
