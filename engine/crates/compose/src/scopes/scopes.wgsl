// The scopes of a frame: counted in integers into buffers, then drawn into a picture. The
// arithmetic of the counts is that of `reference.rs`, step by step, so that the counts are the
// processor's exactly. See `scopes/mod.rs`.
//
// No vector is indexed by a variable (the shader compiler of Direct3D 12 crashed on that in
// loops): the channels are taken one by one. Nor is a signed integer shifted right where a
// constant was added to it or multiplied into it: that too crashes the compiler (an illegal
// instruction, 2026-10-31), so every signed shift is `down`, done on unsigned integers.

struct Params {
    frame_w: u32,
    frame_h: u32,
    step: u32,
    sampled_w: u32,
    sampled_h: u32,
    grid_w: u32,
    grid_h: u32,
    planes: u32,
    kr: i32,
    kg: i32,
    kb: i32,
    cb_mul: i32,
    cr_mul: i32,
    zoom: i32,
    tones: u32,
    tone_low: i32,
    tone_high: i32,
    m0: i32,
    m1: i32,
    m2: i32,
    m3: i32,
    m4: i32,
    m5: i32,
    m6: i32,
    m7: i32,
    m8: i32,
    display: u32,
    mapping: u32,
    gain: f32,
    graticule: f32,
    samples: f32,
    image_w: u32,
    image_h: u32,
    luma_r: f32,
    luma_g: f32,
    luma_b: f32,
    partials: u32,
    strip: u32,
    channels: u32,
}

@group(0) @binding(0) var source_frame: texture_2d<f32>;
@group(0) @binding(1) var<storage, read> params: Params;
@group(0) @binding(2) var<storage, read_write> counts: array<atomic<u32>>;
@group(0) @binding(3) var<storage, read> to_linear: array<u32>;
@group(0) @binding(4) var picture: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(5) var overlay: texture_2d<f32>;
@group(0) @binding(6) var<storage, read_write> partials: array<vec4<f32>>;
@group(0) @binding(7) var<storage, read_write> results: array<vec4<f32>>;

const ONE: i32 = 16384;
const NONE: u32 = 0xffffffffu;
// Pixels down that a thread of the vectorscope and the chromaticity diagram counts.
const RUN: u32 = 8u;
// Pixels down that a thread of the histogram counts.
const HISTOGRAM_RUN: u32 = 32u;
// Pixels down that a thread of the statistics sums.
const STATISTICS_RUN: u32 = 16u;
const BIG: f32 = 3.0e38;

// What the picture shows.
const SHOW_LUMA: u32 = 0u;
const SHOW_RGB: u32 = 1u;
const SHOW_PARADE: u32 = 2u;
const SHOW_VECTORSCOPE: u32 = 3u;
const SHOW_HISTOGRAM: u32 = 4u;
const SHOW_CHROMATICITY: u32 = 5u;

// ---- counting ----

fn code(value: f32, low: f32, high: f32) -> i32 {
    return i32(round(min(max(value, low), high) * 16384.0));
}

fn wide(pixel: vec4<f32>) -> vec3<i32> {
    return vec3<i32>(code(pixel.r, -1.0, 2.0), code(pixel.g, -1.0, 2.0), code(pixel.b, -1.0, 2.0));
}

// `value >> shift` of a signed integer (rounding down), done on an unsigned one: 2^31 is added,
// which makes every value of 32 bits one that is not negative, and taken off again once
// shifted, exactly.
fn down(value: i32, shift: u32) -> i32 {
    return i32((bitcast<u32>(value) ^ 0x80000000u) >> shift) - i32(0x80000000u >> shift);
}

fn luma_of(c: vec3<i32>) -> i32 {
    return down(params.kr * c.x + params.kg * c.y + params.kb * c.z + 16384, 15u);
}

fn level(value: i32, levels: u32) -> u32 {
    return (u32(clamp(value, 0, ONE)) * (levels - 1u) + 8192u) >> 14u;
}

fn sampled(k: u32, j: u32) -> vec4<f32> {
    return textureLoad(source_frame, vec2<u32>(k * params.step, j * params.step), 0);
}

// The columns of the frame, one in `step`, that fall in a column of a waveform: the first of
// them (as a count of steps) and how many.
fn columns_of(column: u32, columns: u32) -> vec2<u32> {
    let fw = params.frame_w;
    let s = params.step;
    let x_low = (column * fw + columns - 1u) / columns;
    let x_high = ((column + 1u) * fw + columns - 1u) / columns;
    let k_low = (x_low + s - 1u) / s;
    let k_high = (x_high + s - 1u) / s;
    return vec2<u32>(k_low, k_high - k_low);
}

// One plane of 1024 levels for each of luma or R, G and B.
var<workgroup> column_bins: array<atomic<u32>, 3072>;

// A workgroup for each column of the waveform: it counts the pixels of the columns of the frame
// that fall in it, by level, then writes them; no other writes there.
@compute @workgroup_size(256)
fn waveform_main(
    @builtin(workgroup_id) group: vec3<u32>,
    @builtin(local_invocation_index) local: u32,
) {
    let column = group.x;
    let columns = params.grid_w;
    let levels = params.grid_h;
    let planes = params.planes;
    for (var i = local; i < planes * levels; i += 256u) {
        atomicStore(&column_bins[i], 0u);
    }
    workgroupBarrier();
    let span = columns_of(column, columns);
    let total = span.y * params.sampled_h;
    for (var i = local; i < total; i += 256u) {
        let c = wide(sampled(span.x + i % span.y, i / span.y));
        if planes == 1u {
            atomicAdd(&column_bins[level(luma_of(c), levels)], 1u);
        } else {
            atomicAdd(&column_bins[level(c.x, levels)], 1u);
            atomicAdd(&column_bins[levels + level(c.y, levels)], 1u);
            atomicAdd(&column_bins[2u * levels + level(c.z, levels)], 1u);
        }
    }
    workgroupBarrier();
    for (var i = local; i < planes * levels; i += 256u) {
        atomicStore(&counts[i * columns + column], atomicLoad(&column_bins[i]));
    }
}

fn vector_bin(c: vec3<i32>) -> u32 {
    let y = luma_of(c);
    if params.tones == 1u && y >= params.tone_low {
        return NONE;
    }
    if params.tones == 2u && (y < params.tone_low || y > params.tone_high) {
        return NONE;
    }
    if params.tones == 3u && y <= params.tone_high {
        return NONE;
    }
    let cb = down((c.z - y) * params.cb_mul + 16384, 15u);
    let cr = down((c.x - y) * params.cr_mul + 16384, 15u);
    let n = i32(params.grid_w);
    let middle = i32(params.grid_w >> 1u);
    let x = down(down(cb * params.zoom, 4u) * n, 14u) + middle;
    let up = down(down(cr * params.zoom, 4u) * n, 14u) + middle;
    // The edge (a Cb or Cr of 0.5, of a bar of 100 %) is in the last bin.
    if x < 0 || x > n || up < 0 || up > n {
        return NONE;
    }
    // As unsigned integers: a signed sum here crashed the compiler too.
    let column = min(u32(x), params.grid_w - 1u);
    let row = params.grid_w - 1u - min(u32(up), params.grid_w - 1u);
    return row * params.grid_w + column;
}

fn chroma_bin(c: vec3<i32>) -> u32 {
    let r = i32(to_linear[u32(clamp(c.x, 0, ONE))]);
    let g = i32(to_linear[u32(clamp(c.y, 0, ONE))]);
    let b = i32(to_linear[u32(clamp(c.z, 0, ONE))]);
    let x = down(params.m0 * r + params.m1 * g + params.m2 * b, 12u);
    let y = down(params.m3 * r + params.m4 * g + params.m5 * b, 12u);
    let z = down(params.m6 * r + params.m7 * g + params.m8 * b, 12u);
    let sum = x + y + z;
    if sum <= 0 || x < 0 || y < 0 {
        return NONE;
    }
    // Not negative from here on: divided as unsigned integers.
    let column = (u32(x) * params.grid_w * 5u) / (u32(sum) * 4u);
    let row = (u32(y) * params.grid_h * 10u) / (u32(sum) * 9u);
    if column >= params.grid_w || row >= params.grid_h {
        return NONE;
    }
    return (params.grid_h - 1u - row) * params.grid_w + column;
}

fn add(bin: u32, run: u32) {
    if bin != NONE && run > 0u {
        atomicAdd(&counts[bin], run);
    }
}

// A thread counts RUN pixels down a column, adding a run of pixels of one bin at once.
@compute @workgroup_size(16, 16)
fn vectorscope_main(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= params.sampled_w {
        return;
    }
    var last = NONE;
    var run = 0u;
    for (var r = 0u; r < RUN; r += 1u) {
        let j = id.y * RUN + r;
        if j >= params.sampled_h {
            break;
        }
        let bin = vector_bin(wide(sampled(id.x, j)));
        if bin == last {
            run += 1u;
        } else {
            add(last, run);
            last = bin;
            run = 1u;
        }
    }
    add(last, run);
}

@compute @workgroup_size(16, 16)
fn chromaticity_main(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= params.sampled_w {
        return;
    }
    var last = NONE;
    var run = 0u;
    for (var r = 0u; r < RUN; r += 1u) {
        let j = id.y * RUN + r;
        if j >= params.sampled_h {
            break;
        }
        let bin = chroma_bin(wide(sampled(id.x, j)));
        if bin == last {
            run += 1u;
        } else {
            add(last, run);
            last = bin;
            run = 1u;
        }
    }
    add(last, run);
}

// R, G, B and luma, of at most 1024 bins each.
var<workgroup> histogram_bins: array<atomic<u32>, 4096>;

// A workgroup counts into its own bins, then adds those it has to the whole, keeping the
// highest count of each channel after the bins.
@compute @workgroup_size(16, 16)
fn histogram_main(
    @builtin(global_invocation_id) id: vec3<u32>,
    @builtin(local_invocation_index) local: u32,
) {
    let bins = params.grid_w;
    for (var i = local; i < 4u * bins; i += 256u) {
        atomicStore(&histogram_bins[i], 0u);
    }
    workgroupBarrier();
    if id.x < params.sampled_w {
        for (var r = 0u; r < HISTOGRAM_RUN; r += 1u) {
            let j = id.y * HISTOGRAM_RUN + r;
            if j >= params.sampled_h {
                break;
            }
            let c = wide(sampled(id.x, j));
            atomicAdd(&histogram_bins[level(c.x, bins)], 1u);
            atomicAdd(&histogram_bins[bins + level(c.y, bins)], 1u);
            atomicAdd(&histogram_bins[2u * bins + level(c.z, bins)], 1u);
            atomicAdd(&histogram_bins[3u * bins + level(luma_of(c), bins)], 1u);
        }
    }
    workgroupBarrier();
    for (var i = local; i < 4u * bins; i += 256u) {
        let count = atomicLoad(&histogram_bins[i]);
        if count > 0u {
            let total = atomicAdd(&counts[i], count) + count;
            atomicMax(&counts[4u * bins + i / bins], total);
        }
    }
}

// ---- drawing ----

// How bright a bin is of `density` times the count of a bin of a uniform spread: by a log or a
// gamma, the gain of the look making it brighter, full at 64 times at a gain of 1.
fn brightness(density: f32) -> f32 {
    if density <= 0.0 {
        return 0.0;
    }
    let g = params.gain * density;
    if params.mapping == 1u {
        return pow(min(g / 64.0, 1.0), 1.0 / 2.2);
    }
    return min(log2(1.0 + g) / log2(65.0), 1.0);
}

fn wave(plane: u32, lvl: u32, column: u32, columns: u32, levels: u32) -> f32 {
    let n = atomicLoad(&counts[(plane * levels + lvl) * columns + column]);
    if n == 0u {
        return 0.0;
    }
    let span = columns_of(column, columns);
    let samples = f32(span.y * params.sampled_h);
    return brightness(f32(n) * f32(levels) / samples);
}

fn tint(channel: u32) -> vec3<f32> {
    if channel == 0u {
        return vec3<f32>(1.0, 0.22, 0.2);
    }
    if channel == 1u {
        return vec3<f32>(0.25, 1.0, 0.3);
    }
    if channel == 2u {
        return vec3<f32>(0.3, 0.5, 1.0);
    }
    return vec3<f32>(0.92, 0.92, 0.92);
}

fn histogram_colour(x: u32, y: u32) -> vec3<f32> {
    let bins = params.grid_w;
    let strip = params.strip;
    let index = y / strip;
    var channel = index;
    if params.channels == 4u {
        channel = (index + 3u) % 4u;
    }
    var peak = max(max(atomicLoad(&counts[4u * bins]), atomicLoad(&counts[4u * bins + 1u])),
        atomicLoad(&counts[4u * bins + 2u]));
    if params.channels == 4u {
        peak = max(peak, atomicLoad(&counts[4u * bins + 3u]));
    }
    let n = atomicLoad(&counts[channel * bins + x]);
    if n == 0u {
        return vec3<f32>(0.0);
    }
    let height = brightness(64.0 * f32(n) / f32(max(peak, 1u)));
    let up = f32(strip - 1u - y % strip) + 0.5;
    if up / f32(strip) <= height {
        return tint(channel) * 0.9;
    }
    return vec3<f32>(0.0);
}

@compute @workgroup_size(16, 16)
fn display_main(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= params.image_w || id.y >= params.image_h {
        return;
    }
    var colour = vec3<f32>(0.0);
    let show = params.display;
    let levels = params.grid_h;
    if show == SHOW_LUMA {
        colour = vec3<f32>(wave(0u, levels - 1u - id.y, id.x, params.grid_w, levels)) * tint(3u);
    } else if show == SHOW_RGB {
        let lvl = levels - 1u - id.y;
        colour = vec3<f32>(
            wave(0u, lvl, id.x, params.grid_w, levels),
            wave(1u, lvl, id.x, params.grid_w, levels),
            wave(2u, lvl, id.x, params.grid_w, levels),
        );
    } else if show == SHOW_PARADE {
        let plane = id.x / params.grid_w;
        let column = id.x % params.grid_w;
        colour = wave(plane, levels - 1u - id.y, column, params.grid_w, levels) * tint(plane);
    } else if show == SHOW_VECTORSCOPE || show == SHOW_CHROMATICITY {
        let n = atomicLoad(&counts[id.y * params.grid_w + id.x]);
        let bins = f32(params.grid_w * params.grid_h);
        colour = vec3<f32>(brightness(f32(n) * bins / params.samples)) * tint(3u);
    } else if show == SHOW_HISTOGRAM {
        colour = histogram_colour(id.x, id.y);
    }
    let lines = textureLoad(overlay, vec2<u32>(id.xy), 0);
    colour = mix(colour, lines.rgb, lines.a * params.graticule);
    textureStore(picture, vec2<u32>(id.xy), vec4<f32>(colour, 1.0));
}

// ---- statistics ----

var<workgroup> reduction: array<vec4<f32>, 256>;

// The sum (0), least (1) or most (2) of a value of each thread of the workgroup.
fn reduce(local: u32, value: vec4<f32>, op: u32) -> vec4<f32> {
    reduction[local] = value;
    workgroupBarrier();
    for (var half = 128u; half > 0u; half = half >> 1u) {
        if local < half {
            let a = reduction[local];
            let b = reduction[local + half];
            if op == 0u {
                reduction[local] = a + b;
            } else if op == 1u {
                reduction[local] = min(a, b);
            } else {
                reduction[local] = max(a, b);
            }
        }
        workgroupBarrier();
    }
    let result = reduction[0];
    workgroupBarrier();
    return result;
}

// Each workgroup sums its pixels in an order of its own (so the sums are the same each time),
// into five values: sums of R, G, B and luma; their least; their most; sums of Cb, Cr and
// their squares; and the sum of the length of chroma.
@compute @workgroup_size(16, 16)
fn statistics_main(
    @builtin(global_invocation_id) id: vec3<u32>,
    @builtin(local_invocation_index) local: u32,
    @builtin(workgroup_id) group: vec3<u32>,
    @builtin(num_workgroups) groups: vec3<u32>,
) {
    var sums = vec4<f32>(0.0);
    var lows = vec4<f32>(BIG);
    var highs = vec4<f32>(-BIG);
    var chroma = vec4<f32>(0.0);
    var lengths = vec4<f32>(0.0);
    if id.x < params.frame_w {
        for (var r = 0u; r < STATISTICS_RUN; r += 1u) {
            let y = id.y * STATISTICS_RUN + r;
            if y >= params.frame_h {
                break;
            }
            let p = textureLoad(source_frame, vec2<u32>(id.x, y), 0);
            let luma = params.luma_r * p.r + params.luma_g * p.g + params.luma_b * p.b;
            let value = vec4<f32>(p.rgb, luma);
            sums += value;
            lows = min(lows, value);
            highs = max(highs, value);
            let cb = (p.b - luma) / (2.0 * (1.0 - params.luma_b));
            let cr = (p.r - luma) / (2.0 * (1.0 - params.luma_r));
            chroma += vec4<f32>(cb, cr, cb * cb, cr * cr);
            lengths.x += sqrt(cb * cb + cr * cr);
        }
    }
    let at = (group.y * groups.x + group.x) * 5u;
    let all_sums = reduce(local, sums, 0u);
    let all_lows = reduce(local, lows, 1u);
    let all_highs = reduce(local, highs, 2u);
    let all_chroma = reduce(local, chroma, 0u);
    let all_lengths = reduce(local, lengths, 0u);
    if local == 0u {
        partials[at] = all_sums;
        partials[at + 1u] = all_lows;
        partials[at + 2u] = all_highs;
        partials[at + 3u] = all_chroma;
        partials[at + 4u] = all_lengths;
    }
}

// The values of every workgroup made one, in an order that does not change.
@compute @workgroup_size(256)
fn statistics_final(@builtin(local_invocation_index) local: u32) {
    var sums = vec4<f32>(0.0);
    var lows = vec4<f32>(BIG);
    var highs = vec4<f32>(-BIG);
    var chroma = vec4<f32>(0.0);
    var lengths = vec4<f32>(0.0);
    for (var i = local; i < params.partials; i += 256u) {
        let at = i * 5u;
        sums += partials[at];
        lows = min(lows, partials[at + 1u]);
        highs = max(highs, partials[at + 2u]);
        chroma += partials[at + 3u];
        lengths += partials[at + 4u];
    }
    let all_sums = reduce(local, sums, 0u);
    let all_lows = reduce(local, lows, 1u);
    let all_highs = reduce(local, highs, 2u);
    let all_chroma = reduce(local, chroma, 0u);
    let all_lengths = reduce(local, lengths, 0u);
    if local == 0u {
        results[0] = all_sums;
        results[1] = all_lows;
        results[2] = all_highs;
        results[3] = all_chroma;
        results[4] = all_lengths;
    }
}
