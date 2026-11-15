// A picture between two of one media (`compose.rs`): what a clip that is retimed by Frame Blend
// shows at a frame whose place in its source lies between two pictures. These lines are put in
// place of those from "planes: begin" to "planes: end" of `yuv.wgsl`, whose own `sample_at`,
// which reads one picture, is named `picture_at` for this shader.
//
// The second picture is of the media of the first: of its size, its sampling and its coding, so
// that the one uniform says how either is read. Its planes are bound after those of the first.
@group(0) @binding(4) var second_luma_plane: texture_2d<Texel>;
@group(0) @binding(5) var second_chroma_plane: texture_2d<Texel>;
@group(0) @binding(6) var second_alpha_plane: texture_2d<Texel>;

// Whether the planes read are those of the second picture.
var<private> of_second: bool = false;

fn luma_at(texel: vec2<i32>) -> f32 {
    if of_second {
        return f32(textureLoad(second_luma_plane, texel, 0).r);
    }
    return f32(textureLoad(luma_plane, texel, 0).r);
}

fn chroma_at(texel: vec2<i32>) -> vec2<f32> {
    var read = textureLoad(chroma_plane, texel, 0);
    if of_second {
        read = textureLoad(second_chroma_plane, texel, 0);
    }
    return vec2<f32>(f32(read.r), f32(read.g));
}

// A pixel of R'G'B'A, whose one plane is bound as the plane of luma.
fn rgba_at(texel: vec2<i32>) -> vec4<f32> {
    var read = textureLoad(luma_plane, texel, 0);
    if of_second {
        read = textureLoad(second_luma_plane, texel, 0);
    }
    return vec4<f32>(f32(read.r), f32(read.g), f32(read.b), f32(read.a));
}

fn alpha_at(texel: vec2<i32>) -> f32 {
    if of_second {
        return f32(textureLoad(second_alpha_plane, texel, 0).r);
    }
    return f32(textureLoad(alpha_plane, texel, 0).r);
}

// The two pictures at `shown`, mixed: `siting.z` of the way from the first to the second. They
// are mixed as `picture_at` gives them, which is as the media has them: Y'CbCr, or R'G'B' of a
// file of it, in the codes of the file with their gamma, before they are made R'G'B', and so
// before the input transform, the grade, the crop and the transform of the layer, which are done
// to the mixed picture as they are to one. Where the pictures have alpha, what is mixed is
// premultiplied by it, so that a pixel that one of them does not cover gives no colour.
fn sample_at(shown: vec2<f32>) -> vec4<f32> {
    of_second = false;
    let first = picture_at(shown);
    of_second = true;
    let second = picture_at(shown);
    of_second = false;
    return mix(first, second, params.siting.z);
}
