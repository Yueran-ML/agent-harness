// A picture of Y'CbCr in two planes, 4:2:0, 4:2:2 or 4:4:4, drawn as R'G'B': by the matrix and
// range of the picture (BT.601, BT.709 or BT.2020, limited or full, in codes of its own bits,
// from the uniform), the gamma of the file kept (the target is not sRGB, so nothing is encoded
// twice). Or a picture of R'G'B'A in one plane (`kind.x`), drawn as it is: of integers at full
// scale (of limited range, where it says so, taken to full), or of half floats, whose values
// beyond 0..1 are kept. Either may have alpha (`kind.y`), straight or premultiplied: of Y'CbCr
// in a plane of its own, of R'G'B'A its fourth value.
//
// Two ways of drawing share it:
// - the view's (`vs`, `fs`): the picture fills the viewport, the rectangle of the target where it
//   is fitted (`dest`), over black where it has alpha;
// - the compositor's (`vs_layer`, `fs_layer`): the picture is a layer, a quad of the frame that it
//   is fitted into on the canvas, put on the canvas by an affine map (`row0`, `row1`), with its
//   edges smoothed over a pixel and its opacity, premultiplied; cut by its crop (`kept`), and
//   blended with what is under it by its composite mode (`laid`). A layer of a clip that is
//   retimed by Frame Blend shows two pictures of its media mixed (`between.wgsl`), which is
//   another shader made of this one: as it is written here, one picture is read.
//
// The planes are read texel by texel and blended here, two by two, rather than sampled: so that
// planes of 16-bit integers, where a card has no 16-bit normalized formats, are read the same
// way, and so that chroma is sited where the file says it lies (`siting`). By default "left"
// (chroma location type 0, the default of MPEG-2 and H.264/HEVC, whatever the colour
// primaries): of 4:2:0, on the first luma sample of its pair of columns, and halfway between its
// two rows; of 4:2:2, on the first of its pair of columns, in its own row (co-sited, as BT.601
// and BT.709 lay 4:2:2); of 4:4:4, on its own sample. A file that says "center" (JPEG, MPEG-1)
// has it halfway between its columns too; "top left" and "top" on the first of its rows, "bottom
// left" and "bottom" on the second (`yuv::siting`).
// The line below is changed to `alias Texel = u32;` for planes of integers.
alias Texel = f32;

struct Params {
    // The view's: x, y, width, height of the picture in the target, in pixels.
    dest: vec4<f32>,
    // The width and height of the luma plane (of R'G'B'A, of its one plane), then of the chroma
    // plane, in texels.
    planes: vec4<f32>,
    // What a texel is multiplied by to be a code (255 for 8 bits, 1023.98 for 10 bits in words
    // of 16-bit unorm, 1/64 for them as integers, 65535/16 and 1/16 for 12 bits); the code of
    // black and the range of codes to white of luma (of R'G'B'A, from 0 to 1: 0 and 1 of full
    // range); the quarter turns clockwise that the picture is turned to be shown.
    luma: vec4<f32>,
    // The code of no colour and the range of codes of chroma; then the samples of chroma to a
    // sample of luma across and down: 0.5 where chroma is halved (across of 4:2:0 and 4:2:2,
    // down of 4:2:0), else 1.
    chroma: vec4<f32>,
    // What Cr and Cb, from -0.5 to 0.5, are multiplied by: R' from Cr, G' from Cb and from Cr
    // (taken away), B' from Cb (`YuvMatrix::coefficients`).
    matrix: vec4<f32>,
    // Where a sample of chroma lies, in luma texels from the luma sample at its own place
    // (twice its place, where chroma is halved): x across, y down (0 and 0.5, "left" of 4:2:0).
    // Then z, how much of a second picture is mixed into this one, from 0 to 1: read by the
    // shader of a layer of two pictures alone (`between.wgsl`), and 0 of any other. And w, of
    // two pictures moved along the flows between them before they are mixed, how many pixels
    // of them a texel of the flows is across and down: read by that shader alone
    // (`flow_between.wgsl`), and 0 of any other.
    siting: vec4<f32>,
    // What the picture is: x 0 of Y'CbCr, 1 of R'G'B'A of integers, 2 of R'G'B'A of floats; y
    // its alpha: 0 none, 1 straight, 2 premultiplied; z what a texel of R'G'B'A, or of the plane
    // of alpha, is multiplied by to be from 0 to 1 (1 of normalized and float texels, 1/65535 of
    // integers); w 1 where R'G'B' decoded keeps what is beyond 0..1 (colour management), 0 where
    // it is held to 0..1.
    kind: vec4<f32>,
    // The compositor's: a point (x, y) of the frame, in its pixels, is put at
    // (row0.x * x + row0.y * y + row0.z, row1.x * x + row1.y * y + row1.z) of the canvas, whose
    // width is row0.w and height row1.w.
    row0: vec4<f32>,
    row1: vec4<f32>,
    // The quad drawn, from (x0, y0) to (x1, y1) of the frame: the whole of it, for a picture.
    quad: vec4<f32>,
    // The opacity of the layer, the pixels of the canvas to a pixel of the frame, how many
    // points across and down each pixel is made of, and over how many pixels of the frame each
    // cut edge of its crop fades.
    extra: vec4<f32>,
    // What its crop keeps of the frame, from (x0, y0) to (x1, y1) in its pixels; a side that is
    // not cut lies far beyond the frame.
    crop: vec4<f32>,
    // Its composite mode, numbered as `grade/layout.rs` numbers them, and the weights of R, G
    // and B in the luma of the working space: read by a layer that is blended alone.
    blend: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var luma_plane: texture_2d<Texel>;
@group(0) @binding(2) var chroma_plane: texture_2d<Texel>;
@group(0) @binding(3) var alpha_plane: texture_2d<Texel>;

// The texels of the planes. The lines from "planes: begin" to "planes: end" are put in place by
// those of `between.wgsl` for a layer that shows two pictures of one media mixed (`compose.rs`),
// which reads the planes of either picture. As they are written, they read the one picture.
// planes: begin
fn luma_at(texel: vec2<i32>) -> f32 {
    return f32(textureLoad(luma_plane, texel, 0).r);
}

fn chroma_at(texel: vec2<i32>) -> vec2<f32> {
    let read = textureLoad(chroma_plane, texel, 0);
    return vec2<f32>(f32(read.r), f32(read.g));
}

// A pixel of R'G'B'A, whose one plane is bound as the plane of luma.
fn rgba_at(texel: vec2<i32>) -> vec4<f32> {
    let read = textureLoad(luma_plane, texel, 0);
    return vec4<f32>(f32(read.r), f32(read.g), f32(read.b), f32(read.a));
}

fn alpha_at(texel: vec2<i32>) -> f32 {
    return f32(textureLoad(alpha_plane, texel, 0).r);
}
// planes: end

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

fn rgba(place: vec2<f32>) -> vec4<f32> {
    let a = around(place, params.planes.xy);
    let top = mix(rgba_at(a.low), rgba_at(vec2<i32>(a.high.x, a.low.y)), a.blend.x);
    let bottom = mix(rgba_at(vec2<i32>(a.low.x, a.high.y)), rgba_at(a.high), a.blend.x);
    return mix(top, bottom, a.blend.y);
}

fn alpha(place: vec2<f32>) -> f32 {
    let a = around(place, params.planes.xy);
    let top = mix(alpha_at(a.low), alpha_at(vec2<i32>(a.high.x, a.low.y)), a.blend.x);
    let bottom = mix(alpha_at(vec2<i32>(a.low.x, a.high.y)), alpha_at(a.high), a.blend.x);
    return mix(top, bottom, a.blend.y);
}

// The picture at `shown`, a place in the picture as it is shown, from 0 to 1 across and down,
// before it is made R'G'B' (`rgb_of`): Y' from 0 to 1 and Cb and Cr about 0, or R', G' and B'
// from 0 to 1 (beyond, of floats); then its alpha. Where it has alpha, the first three are
// premultiplied by it (as they are stored, or multiplied here where it is straight), so that
// points of it are averaged as they are seen; where it has none, alpha is 1.
fn sample_at(shown: vec2<f32>) -> vec4<f32> {
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
    // In luma texels, from the centre of the first.
    let place = stored * params.planes.xy - vec2<f32>(0.5, 0.5);
    var value: vec3<f32>;
    var opacity = 1.0;
    if params.kind.x < 0.5 {
        // A chroma texel lies `siting` from the luma texel of twice its place (of 4:2:0, twice
        // its column and its row): "left" of 4:2:0 at (2x, 2y + 0.5), so that the place in
        // chroma texels is (x · 0.5, (y − 0.5) · 0.5); of 4:2:2 "left", on (2x, y), so
        // (x · 0.5, y); of 4:4:4, on (x, y).
        let step = params.chroma.zw;
        let chroma_place = (place - params.siting.xy) * step;
        let scale = params.luma.x;
        let y = (luma(place) * scale - params.luma.y) / params.luma.z;
        let c = (chroma(chroma_place) * scale - vec2<f32>(params.chroma.x, params.chroma.x))
            / params.chroma.y;
        value = vec3<f32>(y, c.x, c.y);
        if params.kind.y > 0.5 {
            opacity = alpha(place) * params.kind.z;
        }
    } else {
        let read = rgba(place) * params.kind.z;
        value = (read.rgb - vec3<f32>(params.luma.y)) / params.luma.z;
        if params.kind.y > 0.5 {
            opacity = read.a;
        }
    }
    if params.kind.y > 0.5 && params.kind.y < 1.5 {
        value = value * opacity;
    }
    return vec4<f32>(value, opacity);
}

// R'G'B' of what `sample_at` gives (unpremultiplied): of Y'CbCr by the matrix of the picture;
// of R'G'B'A of integers as its levels make it; of floats as it is, beyond 0..1 too
// (scene-linear light, as of OpenEXR).
//
// Y'CbCr and integer R'G'B' are held to 0..1 here, as phase 1 held them, unless `kind.w` says
// that what is beyond is kept: where colour management is on (T7), whose input transforms keep
// superwhites and superblacks and the values of Log beyond 1 (RM20 pp. 221, 239). `DaVinci YRGB`
// keeps the clamp, so that a project that is not managed is drawn to the bit as it was.
fn rgb_of(value: vec3<f32>) -> vec3<f32> {
    if params.kind.x > 1.5 {
        return value;
    }
    var rgb = value;
    if params.kind.x < 0.5 {
        let y = value.x;
        let cb = value.y;
        let cr = value.z;
        rgb = vec3<f32>(
            y + params.matrix.x * cr,
            y - params.matrix.y * cb - params.matrix.z * cr,
            y + params.matrix.w * cb,
        );
    }
    if params.kind.w > 0.5 {
        return rgb;
    }
    return clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0));
}

// What `sample_at` gave, unpremultiplied where it has alpha (and is not transparent): what is
// made R'G'B'. Nothing is divided of a picture without alpha.
fn unpremultiplied(sampled: vec4<f32>) -> vec3<f32> {
    if params.kind.y < 0.5 || sampled.w <= 0.0 {
        return sampled.xyz;
    }
    return sampled.xyz / sampled.w;
}

// The compositor's: the input transform and the grade of the layer, on R'G'B' once it is
// decoded (`color.rs`). The lines from "colour: begin" to "colour: end" are put in place by
// those of a layer that has them; as they are written, they do nothing. `colour_at` is where
// the pixel lies in the picture as it is shown, from 0 to 1 across and down, for the
// neighbourhoods that a grade reads there.
// colour: begin
const colour_identity: bool = true;
var<private> colour_at: vec2<f32>;
fn colour(rgb: vec3<f32>) -> vec3<f32> {
    return rgb;
}
// colour: end

// The dimensions in the same space as colour_at, before clip placement.
fn colour_size() -> vec2<f32> { return select(params.planes.xy, params.planes.yx, (u32(params.luma.w) & 1u) != 0u); }
fn colour_window_at() -> vec2<f32> { return colour_at; }

// R'G'B' of the picture at `shown`, over black where it has alpha.
fn rgb_at(shown: vec2<f32>) -> vec3<f32> {
    let sampled = sample_at(shown);
    if params.kind.y < 0.5 {
        return rgb_of(sampled.xyz);
    }
    return rgb_of(unpremultiplied(sampled)) * sampled.w;
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

// How much of the pixel at `local` the crop keeps: from how far inside the nearest cut edge its
// centre is, across and down, the two multiplied. With no softness an edge is smoothed over a
// pixel of the canvas, as those of the quad are; with it, the layer fades over `extra.w` pixels
// of the frame, inward from the cut.
//
// A side that is not cut lies 1e30 pixels beyond the frame (`geometry.rs`), so that the pixel is
// that far inside it, which is more than any width a ramp is divided by: both ramps are then
// held to exactly 1, and a layer without a crop is multiplied by exactly 1, and drawn as it was
// before there were crops, to the bit.
fn kept(local: vec2<f32>) -> f32 {
    let pixel = 1.0 / params.extra.y;
    let inside = min(local - params.crop.xy, params.crop.zw - local);
    let ramp = clamp(
        (inside + vec2<f32>(0.5 * pixel)) / max(params.extra.w, pixel),
        vec2<f32>(0.0),
        vec2<f32>(1.0),
    );
    return ramp.x * ramp.y;
}

// The composite mode of the layer (`compose.rs`): the lines from "blend: begin" to "blend: end"
// are put in place by those of `blend.wgsl` for a layer that is blended with what is under it.
// As they are written, the layer is laid as it is: `over` is its colour, premultiplied, and
// `position` where its pixel is on the canvas.
// blend: begin
fn laid(over: vec4<f32>, position: vec4<f32>) -> vec4<f32> {
    return over;
}
// blend: end

// A picture drawn smaller than it is is not read at one point of each pixel, which would pick
// some of its pixels and pass over the others (fine stripes would come out as stripes again, or
// as one of their colours): each pixel is made of `extra.z` × `extra.z` points of it, over the
// pixel and as far again about it, weighed as a tent, and Y'CbCr (premultiplied, where it has
// alpha) is averaged before it is made R'G'B'. How far a pixel reaches in the picture is taken
// from the derivatives of where it is, before anything is chosen, as they must be.
//
// Its alpha, where it has any, is taken with its coverage and opacity: its colour is made R'G'B'
// and graded unpremultiplied, and laid on the canvas premultiplied, as the other layers are.
@fragment
fn fs_layer(layer: Layer) -> @location(0) vec4<f32> {
    let size = params.quad.zw - params.quad.xy;
    let shown = (layer.local - params.quad.xy) / size;
    let across = dpdx(shown);
    let down = dpdy(shown);
    let points = u32(params.extra.z + 0.5);
    var sampled: vec4<f32>;
    if points <= 1u {
        sampled = sample_at(clamp(shown, vec2<f32>(0.0), vec2<f32>(1.0)));
    } else {
        var sum = vec4<f32>(0.0);
        var weights = 0.0;
        let count = f32(points);
        for (var i = 0u; i < points; i++) {
            let u = (f32(i) + 0.5) / count * 2.0 - 1.0;
            for (var j = 0u; j < points; j++) {
                let v = (f32(j) + 0.5) / count * 2.0 - 1.0;
                let weight = (1.0 - abs(u)) * (1.0 - abs(v));
                let at = clamp(shown + u * across + v * down, vec2<f32>(0.0), vec2<f32>(1.0));
                sum += sample_at(at) * weight;
                weights += weight;
            }
        }
        sampled = sum / weights;
    }
    let alpha = coverage(layer.local) * params.extra.x * kept(layer.local);
    colour_at = clamp(shown, vec2<f32>(0.0), vec2<f32>(1.0));
    if params.kind.y < 0.5 {
        return laid(vec4<f32>(colour(rgb_of(sampled.xyz)) * alpha, alpha), layer.position);
    }
    if sampled.w <= 0.0 {
        return vec4<f32>(0.0);
    }
    let seen = sampled.w * alpha;
    return laid(
        vec4<f32>(colour(rgb_of(unpremultiplied(sampled))) * seen, seen),
        layer.position,
    );
}
