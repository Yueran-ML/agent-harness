// The flows between two pictures of one media (`flow.rs`): how far each point of the first moves
// to be in the second (forward), and each point of the second to be in the first (backward),
// found by Dense Inverse Search (Kroeger et al., ECCV 2016) over a pyramid of their luma, from
// the coarsest level to the finest. Each entry point is a pass of `flow.rs`:
// - `base_main` reads the luma of the two pictures from their planes into the finest level;
// - `halve_main` makes a level of the one finer than it;
// - `patches_main` moves each patch of a level, both ways, from where the flows of the level
//   coarser than it put it;
// - `dense_main` makes the flows of the level dense, of the patches that lie over each pixel;
// - `store_main` smooths the flows of the finest level and writes them, in pixels of the
//   pictures, into the texture that the layer reads (`flow_between.wgsl`).
// Places in a level are counted in its pixels from the centre of the first, and its flows are
// in its pixels, forward in x and y and backward in z and w.
//
// The line below is changed to `alias Texel = u32;` for planes of integers.
alias Texel = f32;

// A patch is PATCH × PATCH pixels of a level, and one begins every STRIDE pixels across and
// down.
const PATCH: i32 = 8;
const STRIDE: i32 = 4;
// What is added to the matrix of Gauss-Newton of a patch, across and down: a patch whose luma
// hardly changes over it is hardly moved.
const DAMPING: f32 = 1e-4;
// The farthest that a patch is moved by one step, and the farthest from where it began, in
// pixels of its level; and the step short enough to end its search.
const MOST_STEP: f32 = 1.0;
const MOST_MOVED: f32 = 4.0;
const SETTLED: f32 = 0.005;
// How near in luma a pixel about another must be for its flows to be taken into the other's as
// they are smoothed: one this far from it counts e^-1 as much as one of its own luma.
const ALIKE: f32 = 0.05;

struct Params {
    // This level: its width and height in its pixels, and where its luma begins in `luma`; then
    // nothing.
    level: vec4<u32>,
    // Its patches across and down; then nothing.
    patches: vec4<u32>,
    // The level coarser than it, whose flows begin its patches: its width and height, and 1
    // where there is one (0 at the coarsest); then nothing.
    coarse: vec4<u32>,
    // The level finer than it, which it is made of: its width, its height and where its luma
    // begins; then nothing.
    finer: vec4<u32>,
    // The pictures: their width and height in their pixels, how many of their pixels a pixel of
    // the finest level is across and down, and how they are stored: 0 Y'CbCr, 1 R'G'B'A of
    // integers, 2 of floats.
    picture: vec4<u32>,
    // What a texel of the plane of luma is multiplied by to be a code, the code of black and the
    // range of codes to white (of R'G'B'A, from 0 to 1), and what a texel of R'G'B'A is
    // multiplied by to be from 0 to 1: as `yuv.wgsl` reads them.
    codes: vec4<f32>,
    // The steps of the search of a patch, at most; then the weights of R', G' and B' in luma.
    search: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: Params;
// The luma of the two pictures at every level, the finest first, each row after row, from 0 to
// 1: the first's in x and the second's in y.
@group(0) @binding(1) var<storage, read_write> luma: array<vec2<f32>>;
// The flows of the level coarser than the one searched.
@group(0) @binding(2) var<storage, read_write> coarse: array<vec4<f32>>;
// The flows of each patch of the level searched, row after row.
@group(0) @binding(3) var<storage, read_write> patches: array<vec4<f32>>;
// The flows of the level searched, made dense, row after row.
@group(0) @binding(4) var<storage, read_write> dense: array<vec4<f32>>;
// The flows of the finest level, in pixels of the pictures, which the layer reads.
@group(0) @binding(5) var flows: texture_storage_2d<rgba16float, write>;

// The planes of luma of the two pictures (of R'G'B'A, their one planes), read by `base_main`.
@group(1) @binding(0) var first_plane: texture_2d<Texel>;
@group(1) @binding(1) var second_plane: texture_2d<Texel>;

// The index of pixel `p` of a level of `size`, held to its edges.
fn index_of(size: vec2<i32>, p: vec2<i32>) -> u32 {
    let held = clamp(p, vec2<i32>(0), size - vec2<i32>(1));
    return u32(held.y * size.x + held.x);
}

// The luma of picture `which` (0 the first, 1 the second) at pixel `p` of this level.
fn luma_at(p: vec2<i32>, which: u32) -> f32 {
    let pair = luma[params.level.z + index_of(vec2<i32>(params.level.xy), p)];
    return select(pair.x, pair.y, which == 1u);
}

// The luma of picture `which` at `place` of this level, read between the four pixels about it.
fn luma_between(place: vec2<f32>, which: u32) -> f32 {
    let base = floor(place);
    let p = vec2<i32>(base);
    let f = place - base;
    let top = mix(luma_at(p, which), luma_at(p + vec2<i32>(1, 0), which), f.x);
    let bottom = mix(luma_at(p + vec2<i32>(0, 1), which), luma_at(p + vec2<i32>(1, 1), which), f.x);
    return mix(top, bottom, f.y);
}

// How the luma of picture `which` changes across and down at pixel `p` of this level, from the
// pixels on either side of it.
fn gradient(p: vec2<i32>, which: u32) -> vec2<f32> {
    return 0.5 * vec2<f32>(
        luma_at(p + vec2<i32>(1, 0), which) - luma_at(p - vec2<i32>(1, 0), which),
        luma_at(p + vec2<i32>(0, 1), which) - luma_at(p - vec2<i32>(0, 1), which),
    );
}

// The luma of a texel of a plane of luma (of R'G'B'A, of its one plane), from 0 to 1.
fn decoded(read: vec4<f32>) -> f32 {
    if params.picture.w == 0u {
        return clamp((read.r * params.codes.x - params.codes.y) / params.codes.z, 0.0, 1.0);
    }
    let rgb = (read.rgb * params.codes.w - vec3<f32>(params.codes.y)) / params.codes.z;
    return clamp(dot(rgb, params.search.yzw), 0.0, 1.0);
}

// The finest level: the luma of the two pictures, averaged over the pixels of each of its
// pixels.
@compute @workgroup_size(8, 8, 1)
fn base_main(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = params.level.xy;
    if id.x >= size.x || id.y >= size.y {
        return;
    }
    let last = vec2<i32>(params.picture.xy) - vec2<i32>(1);
    let per = i32(params.picture.z);
    let corner = vec2<i32>(id.xy) * per;
    var sum = vec2<f32>(0.0);
    for (var y = 0; y < per; y++) {
        for (var x = 0; x < per; x++) {
            let p = min(corner + vec2<i32>(x, y), last);
            let first = vec4<f32>(textureLoad(first_plane, p, 0));
            let second = vec4<f32>(textureLoad(second_plane, p, 0));
            sum += vec2<f32>(decoded(first), decoded(second));
        }
    }
    luma[params.level.z + id.y * size.x + id.x] = sum / f32(per * per);
}

// A weight of the filter that halves a level, of the four pixels about two: 1, 3, 3 and 1.
fn tap(at: i32) -> f32 {
    return select(1.0, 3.0, at == 1 || at == 2);
}

// A level of the one finer than it, halved: each of its pixels of the four by four pixels about
// the two by two that it covers, by the filter [1 3 3 1] / 8 across and down.
@compute @workgroup_size(8, 8, 1)
fn halve_main(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = params.level.xy;
    if id.x >= size.x || id.y >= size.y {
        return;
    }
    let finer = vec2<i32>(params.finer.xy);
    let corner = vec2<i32>(id.xy) * 2 - vec2<i32>(1);
    var sum = vec2<f32>(0.0);
    for (var y = 0; y < 4; y++) {
        for (var x = 0; x < 4; x++) {
            let at = params.finer.z + index_of(finer, corner + vec2<i32>(x, y));
            sum += luma[at] * (tap(x) * tap(y));
        }
    }
    luma[params.level.z + id.y * size.x + id.x] = sum / 64.0;
}

// Where patch `index` of a row (or a column) of `size` pixels begins: every STRIDE pixels, the
// last laid against the far edge, and at 0 where the row is shorter than a patch.
fn patch_origin(index: i32, size: i32) -> i32 {
    return max(min(index * STRIDE, size - PATCH), 0);
}

// The flows of the coarser level at `place` of this one, in pixels of this one: read between
// the four pixels of that level about it, and doubled. None at the coarsest level.
fn coarse_at(place: vec2<f32>) -> vec4<f32> {
    if params.coarse.z == 0u {
        return vec4<f32>(0.0);
    }
    let size = vec2<i32>(params.coarse.xy);
    let at = place * 0.5 - vec2<f32>(0.5);
    let base = floor(at);
    let p = vec2<i32>(base);
    let f = at - base;
    let top = mix(coarse[index_of(size, p)], coarse[index_of(size, p + vec2<i32>(1, 0))], f.x);
    let bottom = mix(
        coarse[index_of(size, p + vec2<i32>(0, 1))],
        coarse[index_of(size, p + vec2<i32>(1, 1))],
        f.x,
    );
    return mix(top, bottom, f.y) * 2.0;
}

// What is left of the patch of picture `own` that begins at `origin` against the other picture
// moved by `flow`, each less its mean over the patch: the sum of its squares.
fn cost(origin: vec2<i32>, own: u32, flow: vec2<f32>) -> f32 {
    var sum = 0.0;
    var squares = 0.0;
    for (var y = 0; y < PATCH; y++) {
        for (var x = 0; x < PATCH; x++) {
            let p = origin + vec2<i32>(x, y);
            let left = luma_between(vec2<f32>(p) + flow, 1u - own) - luma_at(p, own);
            sum += left;
            squares += left * left;
        }
    }
    return squares - sum * sum / f32(PATCH * PATCH);
}

// The flow of the patch of picture `own` that begins at `origin`, against the other picture,
// begun at `start`: moved by the inverse compositional search of Gauss-Newton of the paper, the
// patch and the other picture each less its mean over the patch, so that a change of brightness
// alone moves nothing. The luma of the patch, its gradients less their mean, and the matrix of
// the search that those make are read once. Where the search went further than MOST_MOVED, or
// left the patch less alike than it began, the patch stays where it began.
fn searched(origin: vec2<i32>, own: u32, start: vec2<f32>) -> vec2<f32> {
    let other = 1u - own;
    // Of the PATCH × PATCH pixels of the patch, row after row.
    var lumas: array<f32, 64>;
    var gradients: array<vec2<f32>, 64>;
    var mean = vec2<f32>(0.0);
    for (var at = 0; at < PATCH * PATCH; at++) {
        let p = origin + vec2<i32>(at % PATCH, at / PATCH);
        lumas[at] = luma_at(p, own);
        gradients[at] = gradient(p, own);
        mean += gradients[at];
    }
    mean /= f32(PATCH * PATCH);
    var outer = vec3<f32>(0.0);
    for (var at = 0; at < PATCH * PATCH; at++) {
        let g = gradients[at] - mean;
        gradients[at] = g;
        outer += vec3<f32>(g.x * g.x, g.x * g.y, g.y * g.y);
    }
    let xx = outer.x + DAMPING;
    let xy = outer.y;
    let yy = outer.z + DAMPING;
    let det = max(xx * yy - xy * xy, DAMPING * DAMPING);
    var flow = start;
    let steps = i32(params.search.x);
    for (var turn = 0; turn < steps; turn++) {
        var b = vec2<f32>(0.0);
        for (var at = 0; at < PATCH * PATCH; at++) {
            let p = origin + vec2<i32>(at % PATCH, at / PATCH);
            b += gradients[at] * (luma_between(vec2<f32>(p) + flow, other) - lumas[at]);
        }
        var delta = vec2<f32>(yy * b.x - xy * b.y, xx * b.y - xy * b.x) / det;
        let moved = length(delta);
        if moved > MOST_STEP {
            delta *= MOST_STEP / moved;
        }
        flow -= delta;
        if moved < SETTLED {
            break;
        }
    }
    if distance(flow, start) > MOST_MOVED || cost(origin, own, flow) > cost(origin, own, start) {
        return start;
    }
    return flow;
}

// The patches of this level, each searched both ways from where the flows of the coarser level
// put its middle.
@compute @workgroup_size(8, 8, 1)
fn patches_main(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= params.patches.x || id.y >= params.patches.y {
        return;
    }
    let size = vec2<i32>(params.level.xy);
    let origin = vec2<i32>(patch_origin(i32(id.x), size.x), patch_origin(i32(id.y), size.y));
    let start = coarse_at(vec2<f32>(origin) + vec2<f32>(0.5 * f32(PATCH)));
    patches[id.y * params.patches.x + id.x] = vec4<f32>(
        searched(origin, 0u, start.xy),
        searched(origin, 1u, start.zw),
    );
}

// The flow `flow` of a patch over pixel `p` of picture `own`, weighed by how well it matches
// there, as the paper weighs it: by 1 / max(1, d) of the difference d of the two pictures under
// it, in codes of 8 bits; and that weight, last.
fn weighed(p: vec2<i32>, flow: vec2<f32>, own: u32) -> vec3<f32> {
    let left = abs(luma_between(vec2<f32>(p) + flow, 1u - own) - luma_at(p, own));
    let weight = 1.0 / max(left * 255.0, 1.0);
    return vec3<f32>(flow * weight, weight);
}

// The flows of each pixel of this level: those of the patches that lie over it, each weighed by
// how well it matches there.
@compute @workgroup_size(8, 8, 1)
fn dense_main(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= params.level.x || id.y >= params.level.y {
        return;
    }
    let size = vec2<i32>(params.level.xy);
    let count = vec2<i32>(params.patches.xy);
    let p = vec2<i32>(id.xy);
    // A pixel lies in two patches at most each way that begin STRIDE apart, and perhaps in the
    // last, which lies against the far edge.
    let low = max(p / STRIDE - vec2<i32>(2), vec2<i32>(0));
    let high = min(p / STRIDE + vec2<i32>(1), count - vec2<i32>(1));
    var forward = vec3<f32>(0.0);
    var backward = vec3<f32>(0.0);
    for (var j = low.y; j <= high.y; j++) {
        let top = patch_origin(j, size.y);
        if p.y < top || p.y >= top + PATCH {
            continue;
        }
        for (var i = low.x; i <= high.x; i++) {
            let left = patch_origin(i, size.x);
            if p.x < left || p.x >= left + PATCH {
                continue;
            }
            let pair = patches[u32(j * count.x + i)];
            forward += weighed(p, pair.xy, 0u);
            backward += weighed(p, pair.zw, 1u);
        }
    }
    // A pixel that no patch lies over keeps the flows of the coarser level.
    var found = coarse_at(vec2<f32>(p) + vec2<f32>(0.5));
    if forward.z > 0.0 && backward.z > 0.0 {
        found = vec4<f32>(forward.xy / forward.z, backward.xy / backward.z);
    }
    dense[id.y * params.level.x + id.x] = found;
}

// The flows of the finest level, smoothed over the three by three pixels about each, by a tent
// across and down times how near the luma of each is to its own, each way of its own picture;
// and written, in pixels of the pictures, for the layer.
@compute @workgroup_size(8, 8, 1)
fn store_main(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= params.level.x || id.y >= params.level.y {
        return;
    }
    let size = vec2<i32>(params.level.xy);
    let p = vec2<i32>(id.xy);
    let here = luma[params.level.z + index_of(size, p)];
    var forward = vec3<f32>(0.0);
    var backward = vec3<f32>(0.0);
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let at = index_of(size, p + vec2<i32>(x, y));
            let near = f32((2 - abs(x)) * (2 - abs(y)));
            let apart = (luma[params.level.z + at] - here) / ALIKE;
            let alike = exp(-(apart * apart)) * near;
            let found = dense[at];
            forward += vec3<f32>(found.xy, 1.0) * alike.x;
            backward += vec3<f32>(found.zw, 1.0) * alike.y;
        }
    }
    let scale = f32(params.picture.z);
    let smoothed = vec4<f32>(forward.xy / forward.z, backward.xy / backward.z);
    textureStore(flows, p, smoothed * scale);
}
