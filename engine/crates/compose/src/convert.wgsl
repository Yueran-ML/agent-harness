// The composed picture, R'G'B' of the output space, made Y'CbCr 4:2:0 of limited range for an
// encoder, by the matrix of that space (BT.709 by default; BT.2020 of non-constant luminance for
// Rec.2100; BT.601):
// NV12 (8 bits) or P010 (10 bits in the high bits of 16), each plane written into a buffer of
// words laid out as `columbina_media::Picture` lays its planes out, rows `stride` bytes apart, so
// that it is read back as it is.
//
// Chroma is sited as the decoder and the view take it ("left", chroma location type 0): a sample
// of Cb and Cr lies on the first luma column of its pair and halfway between its two rows, so it
// is made of the pixels of three columns, weighed 1, 2, 1, over the two rows.

struct Convert {
    // The width and height of the picture.
    size: vec2<u32>,
    // The words of a row of luma, and of chroma.
    words: vec2<u32>,
    // 1 for P010, 0 for NV12; and the rows of chroma.
    ten: u32,
    chroma_rows: u32,
    // The matrix: 0 BT.709, 1 BT.2020, 2 BT.601 (`Target::set_matrix`).
    matrix: u32,
    pad1: u32,
};

@group(0) @binding(0) var canvas: texture_2d<f32>;
@group(0) @binding(1) var<storage, read_write> luma: array<u32>;
@group(0) @binding(2) var<storage, read_write> chroma: array<u32>;
@group(0) @binding(3) var<uniform> params: Convert;

fn pixel(x: i32, y: i32) -> vec3<f32> {
    let last = vec2<i32>(params.size) - vec2<i32>(1, 1);
    let at = clamp(vec2<i32>(x, y), vec2<i32>(0, 0), last);
    return clamp(textureLoad(canvas, at, 0).rgb, vec3<f32>(0.0), vec3<f32>(1.0));
}

// BT.709: Kr = 0.2126, Kb = 0.0722; BT.2020: 0.2627, 0.0593; BT.601: 0.299, 0.114.
fn luma_of(rgb: vec3<f32>) -> f32 {
    if params.matrix == 1u {
        return dot(rgb, vec3<f32>(0.2627, 0.6780, 0.0593));
    }
    if params.matrix == 2u {
        return dot(rgb, vec3<f32>(0.299, 0.587, 0.114));
    }
    return dot(rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// Cb and Cr of `rgb`, whose luma is `y`: B' − Y' and R' − Y' divided by 2 (1 − Kb) and
// 2 (1 − Kr), each by its own constants so that BT.709 is divided as it always was.
fn chroma_of(rgb: vec3<f32>, y: f32) -> vec2<f32> {
    if params.matrix == 1u {
        return vec2<f32>((rgb.b - y) / 1.8814, (rgb.r - y) / 1.4746);
    }
    if params.matrix == 2u {
        return vec2<f32>((rgb.b - y) / 1.772, (rgb.r - y) / 1.402);
    }
    return vec2<f32>((rgb.b - y) / 1.8556, (rgb.r - y) / 1.5748);
}

// How far a value of 8 bits is moved before it is rounded down, at a place: an ordered dither
// of 4 × 4 (Bayer), from 1/32 to 31/32, so that a gradient of the canvas, finer than 8 bits, is
// not banded. Codes of 10 bits are rounded as they are.
fn dither(place: vec2<u32>) -> f32 {
    var bayer = array<u32, 16>(0u, 8u, 2u, 10u, 12u, 4u, 14u, 6u, 3u, 11u, 1u, 9u, 15u, 7u, 13u, 5u);
    return (f32(bayer[(place.y % 4u) * 4u + place.x % 4u]) + 0.5) / 16.0;
}

// A value made a code of `most`: rounded in 10 bits, dithered in 8. A value within an eighth of
// a code of a whole one is that code, undithered: the half floats of the canvas hold white and
// the codes of a picture of 8 bits only so near (a tenth of a code at most), and they are given
// back as they were.
fn coded(value: f32, most: f32, place: vec2<u32>) -> u32 {
    let nearest = round(value);
    if params.ten == 1u || abs(value - nearest) < 0.125 {
        return u32(clamp(nearest, 0.0, most));
    }
    return u32(clamp(floor(value + dither(place)), 0.0, most));
}

// A code of luma: black at 16 or 64, white at 235 or 940.
fn luma_code(rgb: vec3<f32>, place: vec2<u32>) -> u32 {
    if params.ten == 1u {
        return coded(64.0 + 876.0 * luma_of(rgb), 1023.0, place);
    }
    return coded(16.0 + 219.0 * luma_of(rgb), 255.0, place);
}

// The codes of Cb and Cr: no colour at 128 or 512.
fn chroma_codes(rgb: vec3<f32>, place: vec2<u32>) -> vec2<u32> {
    let y = luma_of(rgb);
    let c = chroma_of(rgb, y);
    if params.ten == 1u {
        let v = vec2<f32>(512.0) + 896.0 * c;
        return vec2<u32>(coded(v.x, 1023.0, place), coded(v.y, 1023.0, place));
    }
    let v = vec2<f32>(128.0) + 224.0 * c;
    // Cr dithered by another place of the pattern than Cb, so that their errors do not line up.
    return vec2<u32>(coded(v.x, 255.0, place), coded(v.y, 255.0, place + vec2<u32>(2u, 1u)));
}

// A word of luma: four pixels of NV12, or two of P010, the first in the low bits (little-endian).
@compute @workgroup_size(64, 1, 1)
fn luma_main(@builtin(global_invocation_id) id: vec3<u32>) {
    let word = id.x;
    let row = id.y;
    if word >= params.words.x || row >= params.size.y {
        return;
    }
    var packed = 0u;
    if params.ten == 1u {
        for (var k = 0u; k < 2u; k++) {
            let x = word * 2u + k;
            if x < params.size.x {
                packed |= (luma_code(pixel(i32(x), i32(row)), vec2<u32>(x, row)) << 6u) << (16u * k);
            }
        }
    } else {
        for (var k = 0u; k < 4u; k++) {
            let x = word * 4u + k;
            if x < params.size.x {
                packed |= luma_code(pixel(i32(x), i32(row)), vec2<u32>(x, row)) << (8u * k);
            }
        }
    }
    luma[row * params.words.x + word] = packed;
}

// The colour of the chroma sample `i` of row `j`.
fn chroma_rgb(i: u32, j: u32) -> vec3<f32> {
    let x = i32(i * 2u);
    let y = i32(j * 2u);
    let top = pixel(x - 1, y) + 2.0 * pixel(x, y) + pixel(x + 1, y);
    let bottom = pixel(x - 1, y + 1) + 2.0 * pixel(x, y + 1) + pixel(x + 1, y + 1);
    return (top + bottom) / 8.0;
}

// A word of chroma: two samples of NV12 (Cb, Cr, Cb, Cr), or one of P010 (Cb, Cr).
@compute @workgroup_size(64, 1, 1)
fn chroma_main(@builtin(global_invocation_id) id: vec3<u32>) {
    let word = id.x;
    let row = id.y;
    if word >= params.words.y || row >= params.chroma_rows {
        return;
    }
    let samples = (params.size.x + 1u) / 2u;
    var packed = 0u;
    if params.ten == 1u {
        if word < samples {
            let codes = chroma_codes(chroma_rgb(word, row), vec2<u32>(word, row));
            packed = (codes.x << 6u) | ((codes.y << 6u) << 16u);
        }
    } else {
        for (var k = 0u; k < 2u; k++) {
            let i = word * 2u + k;
            if i < samples {
                let codes = chroma_codes(chroma_rgb(i, row), vec2<u32>(i, row));
                packed |= (codes.x | (codes.y << 8u)) << (16u * k);
            }
        }
    }
    chroma[row * params.words.y + word] = packed;
}
