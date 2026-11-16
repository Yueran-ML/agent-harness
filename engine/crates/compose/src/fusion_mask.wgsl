// A pass of the composition of a clip (`fusion.rs`) that makes each pixel of its settings and of
// the mask that comes in at that pixel, and reads nothing else of any picture: a mask node (a
// Rectangle, an Ellipse or a Polygon), whose picture is white premultiplied by the value of its
// mask, so that R, G, B and A are all that value; and a Background that has an effect mask,
// whose picture is its colour, premultiplied, times that mask.
//
// The mathematics of a mask is that of `mask.rs`, step by step, each by the formula of the
// function that is named beside it, to which this is held (`tests/fusion.rs`). The settings come
// as `mask.rs` makes them pixels of the picture that is drawn (`Shape::placed`, `Common::edge`):
// nothing is made pixels here, and the cosine and the sine of an Angle are not worked out here.
//
// Before these lines are put the feather of an edge and the distance to a side as the windows
// of the Color page have them (the start of `grade/windows.wgsl`): `window_feather` and
// `window_distance`, which are those of a mask too.
//
// The pictures hold R, G, B and A premultiplied by their alpha, in half floats. The mask that
// comes in is the alpha of the picture that is linked to the `mask` input of the node, within 0
// and 1: an alpha below 0, or one that is no number, is 0, and one above 1 is 1.

struct Mask {
    // Of a Rectangle and of an Ellipse, xy: its middle, in the pixels of the picture from its
    // top left corner; zw: the cosine and the sine of its Angle. Of a Background: its colour,
    // premultiplied.
    place: vec4<f32>,
    // x: what is drawn: 0 a Rectangle, 1 an Ellipse, 2 a Polygon, 3 a Background; y: the Paint
    // Mode: 0 where no mask comes in, 1 Merge, 2 Add, 3 Subtract (a Background has 1: it reads
    // the mask that comes in); z: the first of the sides of a Polygon among `sides`; w: how
    // many sides it has, 0 where it has fewer than three points.
    kind: vec4<f32>,
    // xy: half the width and half the height of a Rectangle or of an Ellipse, in pixels; z: the
    // radius of the corners of a Rectangle, in pixels; w: the Fill Method of a Polygon: 0
    // Alternate, 1 Non Zero Winding.
    extent: vec4<f32>,
    // x: half the Soft Edge, in pixels; y: half the Border Width, in pixels, with its sign; z:
    // Solid, 1 or 0; w: Invert, 1 or 0.
    edge: vec4<f32>,
    // x: Level, from 0 to 1.
    level: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: Mask;
// The picture whose alpha is the mask that comes in, of the size of the picture that comes out;
// a texel of nothing, which is not read, where none comes in.
@group(0) @binding(1) var incoming: texture_2d<f32>;
// The sides of the polygons of the frame, one after another: xy where a side starts and zw
// where it ends, in the pixels of the picture, y down.
@group(0) @binding(2) var<storage, read> sides: array<vec4<f32>>;

// The distance of a pixel from a shape that is nowhere (`FAR`).
const MASK_FAR: f32 = 1.0e30;
// How many steps the nearest point of an ellipse is found in (`ELLIPSE_STEPS`).
const MASK_ELLIPSE_STEPS: u32 = 8u;
// The least half of an ellipse that the maths divides by, in pixels (`LEAST_AXIS`).
const MASK_LEAST_AXIS: f32 = 1.0e-3;
// The least length that the maths divides by (`TINY`).
const MASK_TINY: f32 = 1.0e-12;

// One triangle that covers the target.
@vertex
fn vs(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let x = f32((index << 1u) & 2u);
    let y = f32(index & 2u);
    return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

// A value as the maths reads a mask (`unit`): 0 where it is no number, and within 0 and 1.
fn mask_unit(value: f32) -> f32 {
    if value > 0.0 {
        return min(value, 1.0);
    }
    return 0.0;
}

// `pixel` from `centre`, along the two axes of a shape turned by the angle whose cosine and
// sine `turn` is (`local`).
fn mask_local(pixel: vec2<f32>, centre: vec2<f32>, turn: vec2<f32>) -> vec2<f32> {
    let reach = pixel - centre;
    return vec2<f32>(reach.x * turn.x - reach.y * turn.y, reach.x * turn.y + reach.y * turn.x);
}

// The distance from `local` to the edge of a rectangle of the halves `extent` whose corners are
// rounded by `radius`, below 0 inside (`rectangle_distance`).
fn mask_rectangle(local: vec2<f32>, extent: vec2<f32>, radius: f32) -> f32 {
    let beyond = abs(local) - extent + vec2<f32>(radius);
    let outside = length(max(beyond, vec2<f32>(0.0)));
    let inside = min(max(beyond.x, beyond.y), 0.0);
    return outside + inside - radius;
}

// One step towards the point of an ellipse of the halves `axes` that is nearest to `place`,
// `turn` being the cosine and the sine of the parameter of a point of it (`ellipse_step`).
fn mask_ellipse_step(place: vec2<f32>, axes: vec2<f32>, turn: vec2<f32>) -> vec2<f32> {
    // The normal at the point, not of length 1, and its length.
    let normal = vec2<f32>(axes.y * turn.x, axes.x * turn.y);
    let scale = length(normal);
    let radius = scale * scale * scale / (axes.x * axes.y);
    let away = place - axes * turn;
    // `away` along the tangent, and along the normal.
    let along = (away.y * normal.x - away.x * normal.y) / scale;
    let outward = (away.x * normal.x + away.y * normal.y) / scale;
    // From the centre of the circle that the ellipse follows there to `place`.
    let reach = radius + outward;
    let far = max(length(vec2<f32>(along, reach)), MASK_TINY);
    let sine = along / far;
    // The cosine of the angle, less 1.
    var less = reach / far - 1.0;
    if reach > 0.0 {
        less = -(along * along) / (far * (far + reach));
    }
    let moved = vec2<f32>(
        axes.x * turn.x + radius * (less * normal.x - sine * normal.y) / scale,
        axes.y * turn.y + radius * (less * normal.y + sine * normal.x) / scale,
    );
    let next = clamp(moved / axes, vec2<f32>(0.0), vec2<f32>(1.0));
    let norm = length(next);
    if norm > MASK_TINY {
        return next / norm;
    }
    return turn;
}

// The distance from `local` to the edge of an ellipse of the halves `extent`, below 0 inside
// (`ellipse_distance`).
fn mask_ellipse(local: vec2<f32>, extent: vec2<f32>) -> f32 {
    let axes = max(extent, vec2<f32>(MASK_LEAST_AXIS));
    let place = abs(local);
    var turn = vec2<f32>(0.70710678118654752);
    for (var i = 0u; i < MASK_ELLIPSE_STEPS; i++) {
        turn = mask_ellipse_step(place, axes, turn);
    }
    let gap = length(place - axes * turn);
    let share = place / axes;
    if extent.x > 0.0 && extent.y > 0.0 && share.x * share.x + share.y * share.y < 1.0 {
        return -gap;
    }
    return gap;
}

// The distance from `pixel` to the edge of the polygon of the `count` sides from `start` on:
// that to the nearest of them, below 0 where the pixel is inside, by the Non Zero Winding fill
// where `winds` and by the Alternate fill where not (`polygon_distance`). A polygon without
// sides is nowhere.
fn mask_polygon(pixel: vec2<f32>, start: u32, count: u32, winds: bool) -> f32 {
    if count < 3u {
        return MASK_FAR;
    }
    var nearest = MASK_FAR;
    var odd = false;
    var winding = 0;
    for (var i = 0u; i < count; i++) {
        let side = sides[start + i];
        nearest = min(nearest, window_distance(pixel, side));
        if (side.y > pixel.y) != (side.w > pixel.y) {
            let crossing = (side.z - side.x) * (pixel.y - side.y) / (side.w - side.y) + side.x;
            if pixel.x < crossing {
                odd = !odd;
                if side.w > side.y {
                    winding += 1;
                } else {
                    winding -= 1;
                }
            }
        }
    }
    var inside = odd;
    if winds {
        inside = winding != 0;
    }
    if inside {
        return -nearest;
    }
    return nearest;
}

@fragment
fn fs(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let kind = u32(params.kind.x + 0.5);
    let paint = u32(params.kind.y + 0.5);
    // The mask that comes in, at the pixel.
    var under = 0.0;
    if paint != 0u {
        let last = vec2<i32>(textureDimensions(incoming)) - vec2<i32>(1, 1);
        let at = clamp(vec2<i32>(position.xy), vec2<i32>(0, 0), last);
        under = mask_unit(textureLoad(incoming, at, 0).a);
    }
    // A Background: its colour times its effect mask, all four channels.
    if kind == 3u {
        return params.place * under;
    }
    // The distance from the middle of the pixel to the edge of the shape (`Placed::distance`).
    var away = MASK_FAR;
    if kind == 0u {
        let local = mask_local(position.xy, params.place.xy, params.place.zw);
        away = mask_rectangle(local, params.extent.xy, params.extent.z);
    } else if kind == 1u {
        let local = mask_local(position.xy, params.place.xy, params.place.zw);
        away = mask_ellipse(local, params.extent.xy);
    } else {
        let start = u32(params.kind.z + 0.5);
        let count = u32(params.kind.w + 0.5);
        away = mask_polygon(position.xy, start, count, params.extent.w != 0.0);
    }
    // Border Width and Solid (`drawn`).
    var drawn = abs(away) - abs(params.edge.y);
    if params.edge.z != 0.0 {
        drawn = away - params.edge.y;
    }
    // Soft Edge (`feather`).
    let covered = 1.0 - window_feather(drawn, params.edge.x, params.edge.x);
    // Invert and Level (`value`).
    var shown = covered;
    if params.edge.w != 0.0 {
        shown = 1.0 - covered;
    }
    let own = mask_unit(shown * params.level.x);
    // The Paint Mode (`painted`): the mask alone where none comes in.
    var painted = own;
    if paint == 1u {
        painted = mask_unit(under + own * (1.0 - under));
    } else if paint == 2u {
        painted = mask_unit(under + own);
    } else if paint == 3u {
        painted = mask_unit(under - own);
    }
    return vec4<f32>(painted);
}
