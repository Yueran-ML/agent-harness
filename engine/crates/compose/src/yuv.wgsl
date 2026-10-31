// A picture of Y'CbCr 4:2:0 in two planes, drawn as R'G'B': by the matrix and range of the
// picture (BT.601, BT.709 or BT.2020, limited or full, from the uniform), the gamma of the file
// kept (the target is not sRGB, so nothing is encoded twice).
//
// Two ways of drawing share it:
// - the view's (`vs`, `fs`): the picture fills the viewport, the rectangle of the target where it
//   is fitted (`dest`);
// - the compositor's (`vs_layer`, `fs_layer`): the picture is a layer, a quad of the frame that it
//   is fitted into on the canvas, put on the canvas by an affine map (`row0`, `row1`), with its
//   edges smoothed over a pixel and its opacity, premultiplied.
//
// The planes are read texel by texel and blended here, two by two, rather than sampled: so that
// planes of 16-bit integers, where a card has no 16-bit normalized formats, are read the same
// way, and so that chroma is sited "left" (chroma location type 0, the default of MPEG-2 and
// H.264/HEVC, whatever the colour primaries): on the first luma sample of its pair of columns,
// and halfway between its two rows. Files that say another siting are drawn so too (phase 1).
// The line below is changed to `alias Texel = u32;` for planes of integers.
alias Texel = f32;

struct Params {
    // The view's: x, y, width, height of the picture in the target, in pixels.
    dest: vec4<f32>,
    // The width and height of the luma plane, then of the chroma plane, in texels.
    planes: vec4<f32>,
    // What a texel is multiplied by to be a code (255 for 8 bits, 1023.98 for P010 of 16-bit
    // unorm, 1/64 for P010 of integers); the code of black and the range of codes to white of
    // luma; the quarter turns clockwise that the picture is turned to be shown.
    luma: vec4<f32>,
    // The code of no colour and the range of codes of chroma.
    chroma: vec4<f32>,
    // What Cr and Cb, from -0.5 to 0.5, are multiplied by: R' from Cr, G' from Cb and from Cr
    // (taken away), B' from Cb (`YuvMatrix::coefficients`).
    matrix: vec4<f32>,
    // The compositor's: a point (x, y) of the frame, in its pixels, is put at
    // (row0.x * x + row0.y * y + row0.z, row1.x * x + row1.y * y + row1.z) of the canvas, whose
    // width is row0.w and height row1.w.
    row0: vec4<f32>,
    row1: vec4<f32>,
    // The quad drawn, from (x0, y0) to (x1, y1) of the frame: the whole of it, for a picture.
    quad: vec4<f32>,
    // The opacity of the layer, the pixels of the canvas to a pixel of the frame, and how many
    // points across and down each pixel is made of.
    extra: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var luma_plane: texture_2d<Texel>;
@group(0) @binding(2) var chroma_plane: texture_2d<Texel>;

fn luma_at(texel: vec2<i32>) -> f32 {
    return f32(textureLoad(luma_plane, texel, 0).r);
}

fn chroma_at(texel: vec2<i32>) -> vec2<f32> {
    let read = textureLoad(chroma_plane, texel, 0);
    return vec2<f32>(f32(read.r), f32(read.g));
}

// The four texels around a place, in texels from the centre of the first, clamped to the edges.
struct Around {
    low: vec2<i32>,
    high: vec2<i32>,
    blend: vec2<f32>,
};

fn around(place: vec2<f32>, size: vec2<f32>) -> Around {
    let last = vec2<i32>(size) - vec2<i32>(1, 1);
    let base = floor(place);
    let low = vec2<i32>(base);
    var out: Around;
    out.low = clamp(low, vec2<i32>(0, 0), last);
    out.high = clamp(low + vec2<i32>(1, 1), vec2<i32>(0, 0), last);
    out.blend = place - base;
    return out;
}

fn luma(place: vec2<f32>) -> f32 {
    let a = around(place, params.planes.xy);
    let top = mix(luma_at(a.low), luma_at(vec2<i32>(a.high.x, a.low.y)), a.blend.x);
    let bottom = mix(luma_at(vec2<i32>(a.low.x, a.high.y)), luma_at(a.high), a.blend.x);
    return mix(top, bottom, a.blend.y);
}

fn chroma(place: vec2<f32>) -> vec2<f32> {
    let a = around(place, params.planes.zw);
    let top = mix(chroma_at(a.low), chroma_at(vec2<i32>(a.high.x, a.low.y)), a.blend.x);
    let bottom = mix(chroma_at(vec2<i32>(a.low.x, a.high.y)), chroma_at(a.high), a.blend.x);
    return mix(top, bottom, a.blend.y);
}

// Y', Cb and Cr of the picture at `shown`, a place in the picture as it is shown, from 0 to 1
// across and down: Y' from 0 to 1, Cb and Cr about 0.
fn ycc_at(shown: vec2<f32>) -> vec3<f32> {
    // Where it is in the picture as it is stored, which is turned clockwise to be shown.
    var stored: vec2<f32>;
    switch u32(params.luma.w + 0.5) {
        case 1u: {
            stored = vec2<f32>(shown.y, 1.0 - shown.x);
        }
        case 2u: {
            stored = vec2<f32>(1.0 - shown.x, 1.0 - shown.y);
        }
        case 3u: {
            stored = vec2<f32>(1.0 - shown.y, shown.x);
        }
        default: {
            stored = shown;
        }
    }
    // In luma texels, from the centre of the first. A chroma texel lies on the luma texel of
    // twice its column, and between the luma rows of twice its row and the one after.
    let place = stored * params.planes.xy - vec2<f32>(0.5, 0.5);
    let chroma_place = vec2<f32>(place.x * 0.5, (place.y - 0.5) * 0.5);

    let scale = params.luma.x;
    let y = (luma(place) * scale - params.luma.y) / params.luma.z;
    let c = (chroma(chroma_place) * scale - vec2<f32>(params.chroma.x, params.chroma.x))
        / params.chroma.y;
    return vec3<f32>(y, c.x, c.y);
}

// R'G'B' of Y', Cb and Cr, by the matrix of the picture; from 0 to 1, as in phase 1.
fn rgb_of(ycc: vec3<f32>) -> vec3<f32> {
    let y = ycc.x;
    let cb = ycc.y;
    let cr = ycc.z;
    let r = y + params.matrix.x * cr;
    let g = y - params.matrix.y * cb - params.matrix.z * cr;
    let b = y + params.matrix.w * cb;
    return clamp(vec3<f32>(r, g, b), vec3<f32>(0.0), vec3<f32>(1.0));
}

// The compositor's: the input transform and the grade of the layer, on R'G'B' once it is
// decoded (`color.rs`). The lines from "colour: begin" to "colour: end" are put in place by
// those of a layer that has them; as they are written, they do nothing.
// colour: begin
const colour_identity: bool = true;
fn colour(rgb: vec3<f32>) -> vec3<f32> {
    return rgb;
}
// colour: end

// R'G'B' of the picture at `shown`.
fn rgb_at(shown: vec2<f32>) -> vec3<f32> {
    return rgb_of(ycc_at(shown));
}

// The view's: one triangle that covers the viewport.
@vertex
fn vs(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let x = f32((index << 1u) & 2u);
    let y = f32(index & 2u);
    return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

@fragment
fn fs(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let shown = (position.xy - params.dest.xy) / params.dest.zw;
    return vec4<f32>(rgb_at(shown), 1.0);
}

// The compositor's.
struct Layer {
    @builtin(position) position: vec4<f32>,
    // Where the pixel is in the frame, in its pixels.
    @location(0) local: vec2<f32>,
};

// The six corners of the two triangles of a quad, from 0 to 1.
fn corner(index: u32) -> vec2<f32> {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
    );
    return corners[index];
}

@vertex
fn vs_layer(@builtin(vertex_index) index: u32) -> Layer {
    // The quad grown by a pixel of the canvas on each side, for its edges to be smoothed.
    let grow = 1.0 / params.extra.y;
    let local = mix(params.quad.xy - vec2<f32>(grow), params.quad.zw + vec2<f32>(grow), corner(index));
    let x = params.row0.x * local.x + params.row0.y * local.y + params.row0.z;
    let y = params.row1.x * local.x + params.row1.y * local.y + params.row1.z;
    var out: Layer;
    out.position = vec4<f32>(x / params.row0.w * 2.0 - 1.0, 1.0 - y / params.row1.w * 2.0, 0.0, 1.0);
    out.local = local;
    return out;
}

// How much of the pixel at `local` the quad covers: from how far inside its nearest edge the
// pixel's centre is, in pixels of the canvas.
fn coverage(local: vec2<f32>) -> f32 {
    let inside = min(
        min(local.x - params.quad.x, params.quad.z - local.x),
        min(local.y - params.quad.y, params.quad.w - local.y),
    ) * params.extra.y;
    return clamp(inside + 0.5, 0.0, 1.0);
}

// A picture drawn smaller than it is is not read at one point of each pixel, which would pick
// some of its pixels and pass over the others (fine stripes would come out as stripes again, or
// as one of their colours): each pixel is made of `extra.z` × `extra.z` points of it, over the
// pixel and as far again about it, weighed as a tent, and Y'CbCr is averaged before it is made
// R'G'B'. How far a pixel reaches in the picture is taken from the derivatives of where it is,
// before anything is chosen, as they must be.
@fragment
fn fs_layer(layer: Layer) -> @location(0) vec4<f32> {
    let size = params.quad.zw - params.quad.xy;
    let shown = (layer.local - params.quad.xy) / size;
    let across = dpdx(shown);
    let down = dpdy(shown);
    let points = u32(params.extra.z + 0.5);
    var ycc: vec3<f32>;
    if points <= 1u {
        ycc = ycc_at(clamp(shown, vec2<f32>(0.0), vec2<f32>(1.0)));
    } else {
        var sum = vec3<f32>(0.0);
        var weights = 0.0;
        let count = f32(points);
        for (var i = 0u; i < points; i++) {
            let u = (f32(i) + 0.5) / count * 2.0 - 1.0;
            for (var j = 0u; j < points; j++) {
                let v = (f32(j) + 0.5) / count * 2.0 - 1.0;
                let weight = (1.0 - abs(u)) * (1.0 - abs(v));
                let at = clamp(shown + u * across + v * down, vec2<f32>(0.0), vec2<f32>(1.0));
                sum += ycc_at(at) * weight;
                weights += weight;
            }
        }
        ycc = sum / weights;
    }
    let alpha = coverage(layer.local) * params.extra.x;
    return vec4<f32>(colour(rgb_of(ycc)) * alpha, alpha);
}
