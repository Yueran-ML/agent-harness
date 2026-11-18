// A picture between two of one media, each moved along the motion between them (`compose.rs`):
// what a clip that is retimed by Optical Flow shows at a frame whose place in its source lies
// between two pictures. These lines are put after those of `between.wgsl`, whose own
// `sample_at`, which mixes the two as they are, is named `mixed_at` for this shader.
//
// The flows between the two, as `flow.rs` found them: at each place of the pictures as they are
// stored, how far a point moves from the first to be in the second (x and y), and from the second
// to be in the first (z and w), in their pixels. A texel of them is `siting.w` × `siting.w`
// pixels of the pictures.
@group(0) @binding(7) var flows: texture_2d<f32>;

// How far the way back of a point may end from where its way there began before a picture moved
// along its flows is no longer trusted there, in texels of the flows: a picture whose point is
// that far off is trusted e^-1 as much as one whose way back ends where it began.
const FLOW_TRUSTED: f32 = 1.0;

// The flows at `place`, in pixels of the pictures as they are stored from their top left corner,
// read between the four texels about it, those at the edges held.
fn flows_at(place: vec2<f32>) -> vec4<f32> {
    let a = around(place / params.siting.w - vec2<f32>(0.5), vec2<f32>(textureDimensions(flows)));
    let top = mix(
        textureLoad(flows, a.low, 0),
        textureLoad(flows, vec2<i32>(a.high.x, a.low.y), 0),
        a.blend.x,
    );
    let bottom = mix(
        textureLoad(flows, vec2<i32>(a.low.x, a.high.y), 0),
        textureLoad(flows, a.high, 0),
        a.blend.x,
    );
    return mix(top, bottom, a.blend.y);
}

// Where `shown`, a place in the pictures as they are shown (from 0 to 1 across and down), is in
// the pictures as they are stored, which are turned clockwise to be shown: in their pixels.
fn flow_stored(shown: vec2<f32>) -> vec2<f32> {
    var stored = shown;
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
        default: {}
    }
    return stored * params.planes.xy;
}

// A move `by` in the pictures as they are stored, in their pixels, as a move in the pictures as
// they are shown, from 0 to 1 across and down.
fn flow_shown_move(by: vec2<f32>) -> vec2<f32> {
    let share = by / params.planes.xy;
    var shown = share;
    switch u32(params.luma.w + 0.5) {
        case 1u: {
            shown = vec2<f32>(-share.y, share.x);
        }
        case 2u: {
            shown = -share;
        }
        case 3u: {
            shown = vec2<f32>(share.y, -share.x);
        }
        default: {}
    }
    return shown;
}

// How much a picture moved along its flows is trusted at a point, by how far the way back from
// where its flow takes the point ends from where it began (`off`, in pixels of the pictures).
fn flow_trusted(off: f32) -> f32 {
    let apart = off / (FLOW_TRUSTED * params.siting.w);
    return exp(-(apart * apart));
}

// Whether `place`, in pixels of the pictures as they are stored, lies within them: 1 or 0. What
// a picture would bring from beyond its edges is not in it.
fn flow_within(place: vec2<f32>) -> f32 {
    let inside = all(place >= vec2<f32>(0.0)) && all(place <= params.planes.xy);
    return select(0.0, 1.0, inside);
}

// The two pictures at `shown`, each moved along its flow to where it would be `siting.z` of the
// way from the first to the second, and mixed by that share as `mixed_at` mixes them, each also
// by how far it is trusted there; with the two as they are, mixed, beside them by how far
// neither is trusted, so that they are what is shown where neither is, and count for nothing
// where either is trusted whole. The point of the first that comes to `shown` is where the flow
// read at that point takes there: found in a few steps from `shown` itself, and in one where the
// flow is the same all about. The point of the second is found so by its flow back to the first.
fn sample_at(shown: vec2<f32>) -> vec4<f32> {
    let share = params.siting.z;
    let here = flow_stored(shown);
    var back = vec2<f32>(0.0);
    var on = vec2<f32>(0.0);
    for (var turn = 0; turn < 3; turn++) {
        back = -share * flows_at(here + back).xy;
        on = -(1.0 - share) * flows_at(here + on).zw;
    }
    let first_place = here + back;
    let second_place = here + on;
    let forward = flows_at(first_place).xy;
    let backward = flows_at(second_place).zw;
    let first_off = length(forward + flows_at(first_place + forward).zw);
    let second_off = length(backward + flows_at(second_place + backward).xy);
    let first_trust = flow_trusted(first_off) * flow_within(first_place);
    let second_trust = flow_trusted(second_off) * flow_within(second_place);
    let first_share = (1.0 - share) * first_trust;
    let second_share = share * second_trust;
    // How far neither is trusted: none where either is trusted whole, whose share is then
    // above 0, as `siting.z` is between 0 and 1.
    let doubt = 1.0 - max(first_trust, second_trust);
    of_second = false;
    let first = picture_at(shown + flow_shown_move(back));
    of_second = true;
    let second = picture_at(shown + flow_shown_move(on));
    of_second = false;
    let moved = first * first_share + second * second_share;
    return (moved + mixed_at(shown) * doubt) / (first_share + second_share + doubt);
}
