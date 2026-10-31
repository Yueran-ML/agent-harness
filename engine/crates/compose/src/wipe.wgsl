// The comparisons of the viewer (`wipe.rs`): a wipe of the frame shown (A) against a reference
// (B), and a split screen, a tile of the grid at a time. Neither is ever in an export.
//
// Written for FXC: no component of a vector is taken by an index that changes in a loop, and no
// integer is signed where it is shifted or divided.

// --- The wipe --------------------------------------------------------------------------------

struct WipeParams {
    // The width and height of the canvas, and its centre, in pixels.
    canvas: vec4<f32>,
    // The normal of the border of a line (x, y), how far the border is from the centre along
    // it, and the width of the softness, in pixels (0: a hard edge).
    line: vec4<f32>,
    // Half the width and height of the box, the weight of the reference of a mix (or the gain
    // of a difference), and the width of the border line, in pixels.
    box_: vec4<f32>,
    // The width and height of a strip of the blinds or a check of the board, in pixels.
    cell: vec4<f32>,
    // The colour of the border line.
    colour: vec4<f32>,
    // The style (0 a line, 1 a box, 2 a mix, 3 a difference, 4 blinds, 5 a checkerboard), and
    // whether the sides are swapped.
    kind: vec4<u32>,
}

@group(0) @binding(0) var current: texture_2d<f32>;
@group(0) @binding(1) var reference: texture_2d<f32>;
@group(0) @binding(2) var<uniform> wipe: WipeParams;

// One triangle that covers the target.
@vertex
fn vs(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let x = f32((index << 1u) & 2u);
    let y = f32(index & 2u);
    return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

// The weight of the reference at a signed distance `d` from the border: 0 before it, 1 after
// it, a straight ramp across the softness about it.
fn ramp(d: f32) -> f32 {
    let soft = wipe.line.w;
    if (soft > 0.0) {
        return clamp(d / soft + 0.5, 0.0, 1.0);
    }
    return select(0.0, 1.0, d >= 0.0);
}

@fragment
fn fs_wipe(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let texel = vec2<i32>(position.xy);
    let a = textureLoad(current, texel, 0);
    let b = textureLoad(reference, texel, 0);
    let style = wipe.kind.x;
    if (style == 3u) {
        return vec4<f32>(abs(a.rgb - b.rgb) * wipe.box_.z, 1.0);
    }
    let from_centre = position.xy - wipe.canvas.zw;
    var w = 0.0;
    var d = 0.0;
    var edged = false;
    if (style == 0u) {
        d = dot(from_centre, wipe.line.xy) - wipe.line.z;
        w = ramp(d);
        edged = true;
    } else if (style == 1u) {
        let outside = abs(from_centre) - wipe.box_.xy;
        d = max(outside.x, outside.y);
        w = ramp(d);
        edged = true;
    } else if (style == 2u) {
        w = wipe.box_.z;
    } else if (style == 4u) {
        let strip = u32(floor(position.y / wipe.cell.y));
        w = f32(strip & 1u);
    } else {
        let across = u32(floor(position.x / wipe.cell.x));
        let down = u32(floor(position.y / wipe.cell.y));
        w = f32((across + down) & 1u);
    }
    if (wipe.kind.y != 0u) {
        w = 1.0 - w;
    }
    var shown = a;
    if (w >= 1.0) {
        shown = b;
    } else if (w > 0.0) {
        shown = a + (b - a) * w;
    }
    let border = wipe.box_.w;
    if (edged && border > 0.0) {
        let cover = clamp(border * 0.5 - abs(d) + 0.5, 0.0, 1.0);
        shown = vec4<f32>(shown.rgb + (wipe.colour.rgb - shown.rgb) * cover, shown.a);
    }
    return shown;
}

// --- A tile of a split screen ----------------------------------------------------------------

struct TileParams {
    // Where the picture is drawn in the canvas: left, top, right, bottom, in pixels.
    image: vec4<f32>,
    // How many taps each way a pixel takes of the picture (1 to 4), and the width of the
    // outline in pixels (0: none).
    look: vec4<f32>,
    // The colour of the outline.
    colour: vec4<f32>,
}

@group(0) @binding(3) var picture: texture_2d<f32>;
@group(0) @binding(4) var linear: sampler;
@group(0) @binding(5) var<uniform> tile: TileParams;

@fragment
fn fs_tile(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let corner = tile.image.xy;
    let size = tile.image.zw - corner;
    let at = (position.xy - corner) / size;
    if (at.x < 0.0 || at.y < 0.0 || at.x > 1.0 || at.y > 1.0) {
        discard;
    }
    let outline = tile.look.y;
    if (outline > 0.0) {
        let inside = min(
            min(position.x - corner.x, tile.image.z - position.x),
            min(position.y - corner.y, tile.image.w - position.y)
        );
        if (inside < outline) {
            return tile.colour;
        }
    }
    let taps = u32(tile.look.x);
    let share = 1.0 / f32(taps);
    var sum = vec4<f32>(0.0);
    for (var down = 0u; down < taps; down = down + 1u) {
        for (var across = 0u; across < taps; across = across + 1u) {
            let offset = (vec2<f32>(f32(across), f32(down)) + 0.5) * share - 0.5;
            let place = (position.xy + offset - corner) / size;
            sum = sum + textureSampleLevel(picture, linear, place, 0.0);
        }
    }
    return sum * (share * share);
}
