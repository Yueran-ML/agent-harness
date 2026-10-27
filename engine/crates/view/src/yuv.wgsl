// A picture of Y'CbCr 4:2:0 in two planes, drawn as R'G'B' into the rectangle of the target
// where it is fitted (the viewport): BT.709, limited range, the gamma of the file kept (the target
// is not sRGB, so nothing is encoded twice).
//
// The planes are read texel by texel and blended here, two by two, rather than sampled: so that
// planes of 16-bit integers, where a card has no 16-bit normalized formats, are read the same
// way, and so that chroma is sited as BT.709 sites it, with the first luma sample of its row
// (left) and between two rows (centre). The line below is changed to `alias Texel = u32;` for
// planes of integers.
alias Texel = f32;

struct Params {
    // x, y, width, height of the picture in the target, in pixels.
    dest: vec4<f32>,
    // The width and height of the luma plane, then of the chroma plane, in texels.
    planes: vec4<f32>,
    // What a texel is multiplied by to be a code (255 for 8 bits, 1023.98 for P010 of 16-bit
    // unorm, 1/64 for P010 of integers); the code of black and the range of codes to white of
    // luma; the quarter turns clockwise that the picture is turned to be shown.
    luma: vec4<f32>,
    // The code of no colour and the range of codes of chroma.
    chroma: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var luma_plane: texture_2d<Texel>;
@group(0) @binding(2) var chroma_plane: texture_2d<Texel>;

// One triangle that covers the viewport.
@vertex
fn vs(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let x = f32((index << 1u) & 2u);
    let y = f32(index & 2u);
    return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

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

@fragment
fn fs(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    // Where the pixel is in the picture as it is shown, from 0 to 1 across and down...
    let shown = (position.xy - params.dest.xy) / params.dest.zw;
    // ...and so in the picture as it is stored, which is turned clockwise to be shown.
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
    let cb = c.x;
    let cr = c.y;
    // BT.709: Kr = 0.2126, Kb = 0.0722.
    let r = y + 1.5748 * cr;
    let g = y - 0.187324 * cb - 0.468124 * cr;
    let b = y + 1.8556 * cb;
    return vec4<f32>(clamp(vec3<f32>(r, g, b), vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}
