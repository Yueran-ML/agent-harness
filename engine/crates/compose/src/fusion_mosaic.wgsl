// The passes of a Mosaic node of the composition of a clip (`fusion.rs`), as `mosaic.rs` lays
// them out, to which this is held (`tests/fusion.rs`): the picture is laid out in a grid of
// cells, and each cell is shown in one colour, the mean of the picture under it.
//
// A fragment does not read every pixel of a cell for every pixel that it draws. The colours of
// the cells are made first, in two passes: across, into a texture of a texel for each cell
// across and each row of the picture, each the mean of the pixels of its cell in its row; and
// down, into a texture of a texel for each cell, each the mean of the texels of the first of
// the rows of its cell. Every row of a cell has as many pixels, so that the mean of the means
// of its rows is its mean. Then each pixel of the picture is drawn from the texel of its cell,
// mixed with the picture as it came in by the key at the pixel: the effect mask of the node.
// With Whole Cells Only the least of the key over each cell is made as the colours are, across
// and down, and a cell is shown where it is 1, and nowhere else.
//
// The grid comes as `mosaic::Grid` has it, in whole numbers of steps, 256 to a pixel, and the
// cell of a pixel and the pixels of a cell are counted here in whole numbers, by the formulas of
// `Grid::cell` and `Grid::rect`, so that the card and the processor put every pixel in the same
// cell. They are counted in u32: the compiler of Direct3D 12 crashes on some arithmetic of
// signed integers with constants (`neighbourhood.wgsl`).
//
// The pictures hold R, G, B and A premultiplied by their alpha, in half floats; the textures of
// the cells are of half floats too. The key is the alpha of the picture that is linked to the
// `mask` input of the node, within 0 and 1 (`mosaic::level`): an alpha below 0, or one that is
// no number, is 0, and one above 1 is 1.

struct Mosaic {
    // x: what the pass makes: 0, the cells across, of each row; 1, the cells down; 2, the
    // picture. y: what a pass across or down makes of the pixels of a cell: 0, the mean of
    // their colours; 1, the least of their keys. z: the key of the pass that makes the picture:
    // 0, none, and every cell is shown; 1, the alpha of `key` at the pixel; 2, the least of the
    // key over the cell, which is the texel of the cell in `key`, and a cell is shown where it
    // is 1.
    kind: vec4<f32>,
    // xy: the width and the height of a cell, in steps; zw: how many steps the middle of the
    // first pixel is past the side of cells at or before it, across and down.
    grid: vec4<f32>,
    // xy: the width and the height of the picture, in pixels.
    frame: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: Mosaic;
// What the pass reads the pixels of: across, the picture, or the picture whose alpha is the
// key; down, what the pass across made; and of the pass that makes the picture, the picture as
// it came in.
@group(0) @binding(1) var source: texture_2d<f32>;
// The colours of the cells, a texel each: read by the pass that makes the picture alone.
@group(0) @binding(2) var cells: texture_2d<f32>;
// The key of the pass that makes the picture: the picture whose alpha it is, or the least of
// it over each cell, a texel each; a texel of nothing, which is not read, where there is none.
@group(0) @binding(3) var key: texture_2d<f32>;

// The steps that a pixel is divided into (`mosaic::STEPS`).
const MOSAIC_STEPS: u32 = 256u;

// One triangle that covers the target.
@vertex
fn vs(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let x = f32((index << 1u) & 2u);
    let y = f32(index & 2u);
    return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

// A key as a mosaic takes it (`level`): within 0 and 1, and 0 where it is no number.
fn mosaic_level(value: f32) -> f32 {
    if value > 0.0 {
        return min(value, 1.0);
    }
    return 0.0;
}

// The cell of the pixel `pixel` along an axis (`Grid::cell`), counted from that of the first
// pixel: `size` the size of a cell in steps and `lead` the lead of the grid along the axis.
fn mosaic_cell(pixel: u32, size: u32, lead: u32) -> u32 {
    return (pixel * MOSAIC_STEPS + lead) / max(size, 1u);
}

// The first pixel of the cell `index` along an axis `span` pixels long (`Grid::rect`): `span`
// where the cell is beyond the frame.
fn mosaic_edge(index: u32, size: u32, lead: u32, span: u32) -> u32 {
    let side = index * size;
    return min((max(side, lead) - lead + (MOSAIC_STEPS - 1u)) / MOSAIC_STEPS, span);
}

// What a pass across (`down` false) or down makes of the texel at `pixel` of its target: the
// mean of the pixels of the source in the cell `pixel.x` of the row `pixel.y`, or of the
// texels of the column `pixel.x` in the rows of the cell `pixel.y`; or the least of their keys.
fn mosaic_gathered(pixel: vec2<u32>, down: bool, least: bool) -> vec4<f32> {
    var index = pixel.x;
    var size = u32(params.grid.x);
    var lead = u32(params.grid.z);
    var span = u32(params.frame.x);
    if down {
        index = pixel.y;
        size = u32(params.grid.y);
        lead = u32(params.grid.w);
        span = u32(params.frame.y);
    }
    let start = mosaic_edge(index, size, lead, span);
    let end = mosaic_edge(index + 1u, size, lead, span);
    if end <= start {
        return vec4<f32>(0.0);
    }
    var sum = vec4<f32>(0.0);
    var lowest = 1.0;
    for (var at = start; at < end; at = at + 1u) {
        var place = vec2<i32>(i32(at), i32(pixel.y));
        if down {
            place = vec2<i32>(i32(pixel.x), i32(at));
        }
        let texel = textureLoad(source, place, 0);
        sum = sum + texel;
        lowest = min(lowest, mosaic_level(texel.a));
    }
    if least {
        return vec4<f32>(lowest);
    }
    return sum / f32(end - start);
}

// The pixel at `pixel` of the picture: the colour of its cell, mixed with the picture as it
// came in by the key (`mixed`). A key of 0 is the picture as it came in, and a key of 1 the
// colour of the cell, both to the bit.
fn mosaic_shown(pixel: vec2<u32>) -> vec4<f32> {
    let at = vec2<i32>(i32(pixel.x), i32(pixel.y));
    let cell = vec2<i32>(
        i32(mosaic_cell(pixel.x, u32(params.grid.x), u32(params.grid.z))),
        i32(mosaic_cell(pixel.y, u32(params.grid.y), u32(params.grid.w))),
    );
    let first = textureLoad(source, at, 0);
    let keyed = u32(params.kind.z + 0.5);
    var share = 1.0;
    if keyed == 1u {
        share = mosaic_level(textureLoad(key, at, 0).a);
    }
    if keyed == 2u {
        share = select(0.0, 1.0, textureLoad(key, cell, 0).a >= 1.0);
    }
    if share <= 0.0 {
        return first;
    }
    let colour = textureLoad(cells, cell, 0);
    if share >= 1.0 {
        return colour;
    }
    return first + (colour - first) * share;
}

@fragment
fn fs(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let pixel = vec2<u32>(floor(position.xy));
    let made = u32(params.kind.x + 0.5);
    if made == 2u {
        return mosaic_shown(pixel);
    }
    return mosaic_gathered(pixel, made == 1u, params.kind.y > 0.5);
}
