//! Phase 4 end to end: one timeline that uses what the phase added to the picture, made through
//! the protocol; each chosen frame shown by the timeline monitor and grabbed as a still; the
//! timeline exported; and each chosen frame of the film held to the monitor's. They agree but for
//! the loss of the codec, which a wrong frame, a node left out or the wrong Retime Process would
//! not stay within.
//!
//! The timeline is 320 × 180 at 25 frames a second, 60 frames long, of the media of 1080p whose
//! every frame carries its number in blocks at its top left:
//!
//! - V1, frames 0–19: a clip of pictures 10 to 29 whose composition lays a Text+ (written on, its
//!   characters moved in turn by a follower, outlined, on a border) over its `MediaIn` by a
//!   Merge, the cells of a mosaic over that within an ellipse, and moves, turns and grows the
//!   whole by a Transform, all by keys.
//! - V1, frames 20–39: a clip from picture 30 with speed points, half speed for 10 frames and
//!   then one and a half, shown by Frame Blend.
//! - V1, frames 40–59: a clip from picture 60 with speed points, a third of the speed for 12
//!   frames and then one and a half, shown by Optical Flow.
//! - V2, frames 0–59: a clip moved across and grown by keys of its transform, turned, cut by a
//!   crop whose top has keys, at an opacity of 0.85, laid in the Screen mode.
//! - V3, frames 0–59: a title with a stroke, a shadow, a box and two spans, whose size, place
//!   across and blur of its shadow have keys.

use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use columbina_compose::{
    ColorRange, Composite, Compositor, Crop, LayerColor, Scene, SceneLayer, Source, Target,
    Transform, YuvCoding, YuvMatrix,
};
use columbina_media::{Codec, Converter, Hardware, NativePicture, PictureFormat, VideoDecoder};
use columbina_model::fusion::{Kind, Node};
use serde_json::{json, Value};
use tokio::io::AsyncBufReadExt;

use super::{clip, project, project_dir, support, transport_engine_with, Client};

/// How long a method may take whose frame needs shaders of the card that are not made yet, which
/// FXC takes most of a minute over each in a build for tests; and an export.
const SLOW: Duration = Duration::from_secs(600);

/// The size of the timeline, and of a block of the luma that is measured on its own.
const WIDTH: usize = 320;
const HEIGHT: usize = 180;
const BLOCK: usize = 8;

/// The frames compared. Of the composition: 3, 6, 7 and 12, between the keys of its Write On,
/// its follower and its Transform, and 19 at their last. Of the clip at Frame Blend: 21 and 23
/// halfway between two pictures, 26 on one, and 33 halfway in its second segment. Of the clip at
/// Optical Flow: 41 and 42 a third and two thirds of the way from one picture to the next, 43 on
/// one, and 53 and 59 halfway in its second segment. At every one the keys of the clip of V2 and
/// of the title are between theirs.
const FRAMES: [i64; 14] = [3, 6, 7, 12, 19, 21, 23, 26, 33, 41, 42, 43, 53, 59];

/// What a frame of the film is measured by against the monitor's, and how far it may be: the
/// PSNR of the luma over the picture, in dB; the mean difference of the luma over the worst
/// block of 8 × 8 pixels, the luma from 0 to 1; and the mean difference of R'G'B' over the
/// picture, in codes of 8 bits. The luma of the film is read from its own codes, and so is free
/// of its chroma, which the card makes at half the width and height of the picture: the monitor
/// and the export agree on it pixel for pixel but for the codec. A block finds what is wrong in
/// a small place (the number at the top left of a picture of the media), which the PSNR of the
/// whole picture hardly feels; the mean of R'G'B' finds a colour that is wrong over the picture.
#[derive(Clone, Copy, Debug)]
struct Measure {
    psnr: f64,
    block: f64,
    rgb: f64,
}

impl Measure {
    /// Whether it is within `tolerance`: its PSNR as high or higher, its worst block and its
    /// mean of R'G'B' as near or nearer.
    fn within(self, tolerance: Measure) -> bool {
        self.psnr >= tolerance.psnr && self.block <= tolerance.block && self.rgb <= tolerance.rgb
    }
}

/// The loss of `ProRes` 422 of 10 bits, coded frame by frame. Measured on the frames compared:
/// 42 to 46 dB, 3.2 to 4.4 codes of R'G'B', and a worst block of 4.7 to 7.8 codes, always on
/// the letters of the title, the sharpest edges of the picture; a wrong picture is 120 or more.
const PRORES: Measure = Measure {
    psnr: 38.0,
    block: 10.0 / 255.0,
    rgb: 6.0,
};

/// The loss of H.264 of 8 bits at a constant quality of 18, its codes dithered by the card: more
/// than that of `ProRes` on the edges of letters.
const H264: Measure = Measure {
    psnr: 30.0,
    block: 16.0 / 255.0,
    rgb: 8.0,
};

/// A frame as it is compared: the luma of each pixel, from 0 (black) to 1 (white), and its
/// R'G'B' in codes of 8 bits, row by row.
struct Shown {
    luma: Vec<f64>,
    rgb: Vec<[f64; 3]>,
}

/// The luma of R'G'B' (each from 0 to 1), by the weights of BT.709, which the export makes its
/// codes by.
fn luma_of(rgb: [f64; 3]) -> f64 {
    0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]
}

impl Shown {
    /// A still of the monitor, as R'G'B' of 16 bits.
    fn of_still(words: &[u16]) -> Shown {
        let rgb: Vec<[f64; 3]> = words
            .as_chunks::<3>()
            .0
            .iter()
            .map(|pixel| pixel.map(|word| f64::from(word) / f64::from(u16::MAX)))
            .collect();
        Shown {
            luma: rgb.iter().copied().map(luma_of).collect(),
            rgb: rgb
                .iter()
                .map(|pixel| pixel.map(|value| value * 255.0))
                .collect(),
        }
    }

    /// A frame of the film as it was decoded: its luma from its codes, by the range that it
    /// says, and its R'G'B' as the engine draws a picture of its coding, through `compose`
    /// into `target`.
    fn of_film(picture: NativePicture, compose: &mut Compositor, target: &Target) -> Shown {
        assert_eq!((picture.width, picture.height), (320, 180));
        assert!(!picture.format.is_rgb(), "{:?}", picture.format);
        // The luma of a still is worked out by the weights that the codes of the film were.
        assert_eq!(picture.matrix, YuvMatrix::Bt709);
        let coding = YuvCoding::of_native(&picture);
        let step = f64::from(1_u32 << (picture.bits - 8));
        let (black, span) = if coding.range == ColorRange::Full {
            (0.0, f64::from((1_u32 << picture.bits) - 1))
        } else {
            (16.0 * step, 219.0 * step)
        };
        // Codes of more than 8 bits are at the top of words of 16; what is below is a fraction.
        let below = f64::from(1_u32 << (16 - picture.bits));
        let width = picture.width as usize;
        let bytes = picture.format == PictureFormat::Nv12;
        let luma = picture
            .luma
            .chunks(picture.luma_stride)
            .take(HEIGHT)
            .flat_map(|row| -> Vec<f64> {
                if bytes {
                    row[..width].iter().copied().map(f64::from).collect()
                } else {
                    row[..2 * width]
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|pair| f64::from(u16::from_le_bytes(*pair)) / below)
                        .collect()
                }
            })
            .map(|code| (code - black) / span)
            .collect();
        let scene = Scene {
            width: 320,
            height: 180,
            layers: vec![SceneLayer {
                color: LayerColor {
                    coding,
                    ..LayerColor::default()
                },
                source: Source::Picture(Arc::new(picture), 0),
                transform: Transform::NONE,
                opacity: 1.0,
                crop: Crop::NONE,
                composite: Composite::Normal,
            }],
            ..Scene::default()
        };
        compose.render(&scene, target).unwrap();
        let rgb = target
            .read_rgba()
            .unwrap()
            .as_chunks::<4>()
            .0
            .iter()
            .map(|pixel| [0, 1, 2].map(|channel| f64::from(pixel[channel])))
            .collect();
        Shown { luma, rgb }
    }
}

/// How far `film` is from `monitor`.
fn distance(monitor: &Shown, film: &Shown) -> Measure {
    let mut squares = 0.0_f64;
    let mut pixels = 0.0_f64;
    let mut block = 0.0_f64;
    for top in (0..HEIGHT).step_by(BLOCK) {
        for left in (0..WIDTH).step_by(BLOCK) {
            let (mut sum, mut count) = (0.0_f64, 0.0_f64);
            for y in top..(top + BLOCK).min(HEIGHT) {
                for x in left..(left + BLOCK).min(WIDTH) {
                    let gap = (monitor.luma[y * WIDTH + x] - film.luma[y * WIDTH + x]).abs();
                    sum += gap;
                    squares += gap * gap;
                    count += 1.0;
                }
            }
            pixels += count;
            block = block.max(sum / count);
        }
    }
    let rgb: f64 = monitor
        .rgb
        .iter()
        .zip(&film.rgb)
        .map(|(seen, made)| {
            (0..3)
                .map(|channel| (seen[channel] - made[channel]).abs())
                .sum::<f64>()
        })
        .sum();
    Measure {
        psnr: 10.0 * (pixels / squares).log10(),
        block,
        rgb: rgb / (3.0 * pixels),
    }
}

/// The next message of the engine, waited for until `deadline`.
async fn next_message(client: &mut Client, deadline: Instant) -> Value {
    let mut line = String::new();
    tokio::time::timeout_at(deadline.into(), client.reader.read_line(&mut line))
        .await
        .expect("a message in time")
        .unwrap();
    serde_json::from_str(&line).unwrap()
}

/// The result of a method that may be long in coming, keeping the notifications before it.
async fn slow(client: &mut Client, method: &str, params: Value) -> Value {
    let id = client.request(method, Some(params)).await;
    let deadline = Instant::now() + SLOW;
    loop {
        let message = next_message(client, deadline).await;
        if message.get("id").is_some() {
            assert_eq!(message["id"], id, "{message}");
            assert!(message.get("error").is_none(), "{message}");
            return message["result"].clone();
        }
        client.notifications.push(message);
    }
}

/// Export by `params` of `render.start`, and give the last progress of the job.
async fn rendered(client: &mut Client, params: Value) -> Value {
    client.take_notifications();
    let job = slow(client, "render.start", params).await["job"].clone();
    let deadline = Instant::now() + SLOW;
    loop {
        let message = if client.notifications.is_empty() {
            next_message(client, deadline).await
        } else {
            client.notifications.remove(0)
        };
        if message["method"] == "render.progress"
            && message["params"]["job"] == job
            && message["params"]["state"] != "running"
        {
            return message["params"].clone();
        }
    }
}

/// The R'G'B' of 16 bits of a PNG, and its size.
fn png16(path: &Path) -> (u32, u32, Vec<u16>) {
    let bytes = std::fs::read(path).unwrap();
    let mut reader = png::Decoder::new(Cursor::new(&bytes)).read_info().unwrap();
    let info = reader.info();
    assert_eq!(
        (info.color_type, info.bit_depth),
        (png::ColorType::Rgb, png::BitDepth::Sixteen)
    );
    let (width, height) = (info.width, info.height);
    let mut data = vec![0; reader.output_buffer_size().unwrap()];
    reader.next_frame(&mut data).unwrap();
    let words = data
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_be_bytes(*pair))
        .collect();
    (width, height, words)
}

/// Seek the timeline monitor to `frame`, and give what it shows there exactly: grabbed as a
/// still into the gallery at `gallery`, a PNG of 16 bits, which holds R'G'B' within 0 and 1 as
/// the export makes its codes of them. The first four pixels of the still are those that the
/// monitor's own digest reads.
async fn shown(client: &mut Client, gallery: &Path, frame: i64) -> Shown {
    client.take_notifications();
    client
        .result(
            "transport.seek",
            Some(json!({"monitor": "timeline", "frame": frame})),
        )
        .await;
    client
        .until_state("the frame sought", |state| {
            state["monitor"] == "timeline" && state["shown"] == frame && state["exact"] == true
        })
        .await;
    let grabbed = slow(
        client,
        "still.grab",
        json!({"view": "timeline", "format": "png"}),
    )
    .await;
    assert_eq!(grabbed["source"]["frame"], frame, "{grabbed}");
    let id = grabbed["id"].as_str().unwrap();
    let (width, height, words) = png16(&gallery.join("stills").join(format!("{id}.png")));
    assert_eq!((width, height), (320, 180), "{grabbed}");
    let digest = client
        .result(
            "transport.frame_digest",
            Some(json!({"monitor": "timeline"})),
        )
        .await;
    assert_eq!(digest["frame"], frame, "{digest}");
    for (place, pixel) in digest["pixels"].as_array().unwrap().iter().enumerate() {
        for channel in 0..3 {
            let stored = f64::from(words[place * 3 + channel]) / f64::from(u16::MAX) * 255.0;
            let read = pixel[channel].as_f64().unwrap();
            assert!(
                (stored - read).abs() <= 0.6,
                "frame {frame}, pixel {place}, channel {channel}: {stored} of the still, {read} \
                 shown"
            );
        }
    }
    Shown::of_still(&words)
}

/// A key of a setting at `frame` of what its keys count, of `value`, straight to the next.
fn key(frame: i64, value: f64) -> Value {
    json!({"frame": frame, "value": value, "interp": "linear"})
}

/// A setting of keys, each `(frame, value)`.
fn keys(pairs: &[(i64, f64)]) -> Value {
    let each: Vec<Value> = pairs
        .iter()
        .map(|&(frame, value)| key(frame, value))
        .collect();
    json!({ "keys": each })
}

/// The style of the title: a stroke, a shadow, a box, and two spans of "Columbina phase", the
/// first word bold in yellow and the second larger and italic.
fn style() -> Value {
    json!({
        "font": "", "size": 22, "color": "#ffffffff", "x": 0.5, "y": 0.86, "align": "center",
        "stroke": {"color": "#101010ff", "width": 2.0},
        "shadow": {"color": "#000000b0", "x": 2.0, "y": 2.0, "blur": 1.5},
        "background": {"color": "#203a8cd0", "padding": 5.0, "radius": 4.0},
        "spans": [
            {"from": 0, "to": 9, "color": "#ffd23cff", "bold": true},
            {"from": 10, "to": 15, "scale": 1.25, "italic": true},
        ],
    })
}

/// The timeline, of media whose duration counts `scale` ticks a second, as it is added; the
/// names of the transaction.
async fn added(client: &mut Client, scale: i64) -> Value {
    let from = |id: &str, start: i64, length: i64, picture: i64| {
        let mut made = clip(id, "m1", start, length);
        made["source"]["in"] = json!({"ticks": picture * scale / 25, "scale": scale});
        made
    };
    let mut title = clip("$title", "unused", 0, 60);
    title["source"] = json!({"kind": "title", "text": "Columbina phase", "style": style()});
    let track = |id: &str, name: &str, clips: Vec<Value>| {
        json!({"id": id, "kind": "video", "name": name, "enabled": true, "locked": false,
            "clips": clips, "transitions": []})
    };
    let first = vec![
        from("$fused", 0, 20, 10),
        from("$blended", 20, 20, 30),
        from("$flowed", 40, 20, 60),
    ];
    client
        .apply(json!([{"op": "add_timeline", "timeline": {
            "id": "$t", "name": "Phase 4", "width": 320, "height": 180,
            "rate": {"num": 25, "den": 1}, "sample_rate": 48000,
            "tracks": [
                track("$v1", "V1", first),
                track("$v2", "V2", vec![from("$moved", 0, 60, 0)]),
                track("$v3", "V3", vec![title]),
            ],
        }}]))
        .await["names"]
        .clone()
}

/// Speed points of the clips of V1 at 20 and at 40, shown by Frame Blend and by Optical Flow.
async fn retimed(client: &mut Client, names: &Value) {
    let timeline = &names["$t"];
    let points = |first: [i64; 2], at: i64, then: [i64; 2]| {
        let speed = |[num, den]: [i64; 2]| json!({"num": num, "den": den});
        json!({"points": [{"frame": 0, "speed": speed(first)},
            {"frame": at, "speed": speed(then)}]})
    };
    client
        .apply(json!([
            {"op": "set_retime", "timeline": timeline, "id": names["$blended"],
                "retime": points([1, 2], 10, [3, 2])},
            {"op": "set_clip", "timeline": timeline, "id": names["$blended"],
                "retime_process": "frame_blend"},
            {"op": "set_retime", "timeline": timeline, "id": names["$flowed"],
                "retime": points([1, 3], 12, [3, 2])},
            {"op": "set_clip", "timeline": timeline, "id": names["$flowed"],
                "retime_process": "optical_flow"},
        ]))
        .await;
}

/// To the clip of V2: an opacity, a turn, a crop and the Screen mode, and keys of its place
/// across, its size and the top of its crop, at frames of its media (from picture 0, those of
/// the timeline). To the title: keys of its size, its place across and the blur of its shadow,
/// at its own frames.
async fn moved(client: &mut Client, names: &Value) {
    let timeline = &names["$t"];
    let (moving, title) = (&names["$moved"], &names["$title"]);
    client
        .apply(json!([
            {"op": "set_clip", "timeline": timeline, "id": moving,
                "transform": {"x": 0.0, "y": 0.0, "scale": 0.5, "rotation": 8.0,
                    "opacity": 0.85},
                "crop": {"left": 0.08, "right": 0.04, "top": 0.0, "bottom": 0.06,
                    "softness": 0.04},
                "composite": "screen"},
            {"op": "set_clip_keys", "timeline": timeline, "id": moving, "path": "transform.x",
                "param": keys(&[(0, -0.25), (59, 0.25)])},
            {"op": "splice_clip_keys", "timeline": timeline, "id": moving,
                "path": "transform.scale", "at": 0, "remove": 0,
                "keys": [key(0, 0.45), key(59, 0.6)]},
            {"op": "set_clip_keys", "timeline": timeline, "id": moving, "path": "crop.top",
                "param": keys(&[(0, 0.0), (59, 0.2)])},
            {"op": "set_clip_keys", "timeline": timeline, "id": title, "path": "title.size",
                "param": keys(&[(0, 16.0), (59, 28.0)])},
            {"op": "set_clip_keys", "timeline": timeline, "id": title, "path": "title.x",
                "param": keys(&[(0, 0.4), (59, 0.6)])},
            {"op": "splice_clip_keys", "timeline": timeline, "id": title,
                "path": "title.shadow.blur", "at": 0, "remove": 0,
                "keys": [key(0, 0.5), key(59, 3.0)]},
        ]))
        .await;
}

/// The composition of the clip of V1 at 0: a Text+ (`words`) with an outline, a border and a
/// follower, laid by a Merge (`over`) over the `MediaIn` (`in`); a mosaic (`cells`) of that,
/// limited by an ellipse (`oval`); and a Transform (`move`) of it into the `MediaOut` (`out`).
/// Its keys are set by operations of their own.
fn composition() -> Value {
    let node = |id: &str, kind: Kind| serde_json::to_value(Node::new(id, kind)).unwrap();
    let mut words = node("words", Kind::TextPlus);
    words["params"] = json!({
        "text": "PHASE FOUR", "size": 0.16, "center": {"x": 0.5, "y": 0.62},
        "fill": {"color": {"r": 1.0, "g": 0.95, "b": 0.6, "a": 1.0}},
        "outline": {"color": {"r": 0.0, "g": 0.0, "b": 0.0, "a": 1.0}, "thickness": 0.08},
        "border": {"color": {"r": 0.1, "g": 0.1, "b": 0.35, "a": 0.6}, "extend": 0.015,
            "round": 0.3},
        "follower": {"order": "left_to_right", "delay": 1.0, "delay_type": "each",
            "opacity": 1.0, "offset": {"x": 0.0, "y": 0.0}, "angle": 0.0, "scale": 1.0},
    });
    let mut oval = node("oval", Kind::Ellipse);
    oval["params"]["center"] = json!({"x": 0.3, "y": 0.42});
    oval["params"]["width"] = json!(0.25);
    oval["params"]["height"] = json!(0.25);
    oval["params"]["soft_edge"] = json!(0.01);
    let mut cells = node("cells", Kind::Mosaic);
    cells["params"]["frequency"] = json!(20.0);
    let link = |from: &str, to: &str, input: &str| json!({"from": from, "to": to, "input": input});
    json!({
        "nodes": [node("in", Kind::MediaIn), words, node("over", Kind::Merge), oval, cells,
            node("move", Kind::Transform), node("out", Kind::MediaOut)],
        "links": [link("in", "over", "background"), link("words", "over", "foreground"),
            link("over", "cells", "input"), link("oval", "cells", "mask"),
            link("cells", "move", "input"), link("move", "out", "input")],
    })
}

/// To the clip of V1 at 0: its composition, and the keys of its Write On, of its follower and of
/// its Transform, at frames of the media (the clip shows pictures 10 to 29).
async fn composed(client: &mut Client, names: &Value) {
    let timeline = &names["$t"];
    let fused = &names["$fused"];
    let set = |path: &str, pairs: &[(i64, f64)]| {
        json!({"op": "set_fusion_keys", "timeline": timeline, "clip": fused, "path": path,
            "param": keys(pairs)})
    };
    client
        .apply(json!([
            {"op": "set_fusion", "timeline": timeline, "clip": fused, "value": composition()},
            set("words.write_on.end", &[(10, 0.0), (22, 1.0)]),
            set("words.follower.offset.y", &[(10, 0.12), (18, 0.0)]),
            set("move.size", &[(10, 1.0), (29, 1.25)]),
            set("move.angle", &[(10, 0.0), (29, 6.0)]),
            {"op": "splice_fusion_keys", "timeline": timeline, "clip": fused,
                "path": "move.center.x", "at": 0, "remove": 0,
                "keys": [key(10, 0.5), key(29, 0.56)]},
        ]))
        .await;
}

/// The timeline of the test, made and edited through the protocol and checked as the project
/// holds it; the names of the transaction that made it.
async fn built(client: &mut Client) -> Value {
    let scale = project(client).await["project"]["media"][0]["facts"]["duration"]["scale"]
        .as_i64()
        .unwrap();
    let names = added(client, scale).await;
    retimed(client, &names).await;
    moved(client, &names).await;
    composed(client, &names).await;
    let made = project(client).await;
    let tracks = &made["project"]["timelines"][0]["tracks"];
    let [fused, blended, flowed] = [0, 1, 2].map(|index| &tracks[0]["clips"][index]);
    assert_eq!(
        fused["fusion"]["nodes"].as_array().unwrap().len(),
        7,
        "{fused}"
    );
    // The Transform, `move`, whose size has keys.
    assert!(
        fused["fusion"]["nodes"][5]["params"]["size"]["keys"].is_array(),
        "{fused}"
    );
    assert_eq!(blended["retime_process"], "frame_blend", "{blended}");
    assert_eq!(
        blended["retime"]["points"][1]["speed"],
        json!({"num": 3, "den": 2}),
        "{blended}"
    );
    assert_eq!(flowed["retime_process"], "optical_flow", "{flowed}");
    let moving = &tracks[1]["clips"][0];
    assert_eq!(moving["composite"], "screen", "{moving}");
    assert!(moving["transform"]["scale"]["keys"].is_array(), "{moving}");
    let title = &tracks[2]["clips"][0];
    assert!(
        title["source"]["style"]["shadow"]["blur"]["keys"].is_array(),
        "{title}"
    );
    names
}

/// That the tolerance is meaningful: what the monitor shows of a wrong frame, of the composition
/// with a node left out, and of the wrong Retime Process is beyond it from the film. Frames 6 and
/// 7 of the film are held to the monitor's 7 and 6; frame 19 to the monitor's once the Merge of
/// the Text+ is passed through; and frame 23, where the clip at Frame Blend is halfway between
/// pictures 31 and 32 of the media (the number of the block at the top left from 15 to 0), to
/// the monitor's once the clip shows the nearest picture alone.
async fn wrong_frames_fail(
    client: &mut Client,
    gallery: &Path,
    names: &Value,
    monitor: &[Shown],
    exported: &[Shown],
    tolerance: Measure,
) {
    let at = |frame: i64| FRAMES.iter().position(|&each| each == frame).unwrap();
    for (made, seen) in [(6, 7), (7, 6)] {
        let far = distance(&monitor[at(seen)], &exported[at(made)]);
        eprintln!("frame {made} of the film, frame {seen} of the monitor: {far:?}");
        assert!(
            !far.within(tolerance),
            "frame {made} of the film is within {tolerance:?} of frame {seen} of the monitor: \
             {far:?}"
        );
    }
    let timeline = &names["$t"];
    client
        .apply(json!([{"op": "set_fusion_node", "timeline": timeline,
            "clip": names["$fused"], "id": "over", "pass_through": true}]))
        .await;
    let bare = shown(client, gallery, 19).await;
    let far = distance(&bare, &exported[at(19)]);
    eprintln!("frame 19 without its Text+: {far:?}");
    assert!(
        !far.within(tolerance),
        "frame 19 without its Text+ is within {tolerance:?} of the film: {far:?}"
    );
    client
        .apply(
            json!([{"op": "set_clip", "timeline": timeline, "id": names["$blended"],
            "retime_process": "nearest"}]),
        )
        .await;
    let nearest = shown(client, gallery, 23).await;
    let far = distance(&nearest, &exported[at(23)]);
    eprintln!("frame 23 at Nearest: {far:?}");
    assert!(
        !far.within(tolerance),
        "frame 23 at Nearest is within {tolerance:?} of the film at Frame Blend: {far:?}"
    );
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "one timeline of what phase 4 draws, shown, exported and compared frame by frame"
)]
async fn what_phase_4_adds_to_the_picture_is_exported_as_the_timeline_monitor_shows_it() {
    let Some(gpu) = columbina_compose::gpu_for_tests() else {
        return;
    };
    // ProRes where the machine has it, else H.264 on the card, each with its own loss.
    let chosen = [
        (Codec::ProRes, "prores", "phase4.mov", PRORES),
        (Codec::H264, "h264", "phase4.mp4", H264),
    ]
    .into_iter()
    .find(|(codec, ..)| !columbina_media::encoders_for(*codec).is_empty());
    let Some((_, codec, leaf, tolerance)) = chosen else {
        assert!(
            std::env::var_os("COLUMBINA_REQUIRE_GPU").is_none(),
            "an encoder of ProRes or H.264 is required"
        );
        return;
    };
    let dir = project_dir();
    let output = project_dir();
    let Some((_engine, mut client)) = transport_engine_with(
        dir.path(),
        &[support::Sample::Fhd],
        &["--sound", "null", "--test-hooks"],
    )
    .await
    else {
        return;
    };
    let names = built(&mut client).await;
    let timeline = names["$t"].clone();
    client
        .result(
            "transport.load",
            Some(json!({"monitor": "timeline", "timeline": timeline})),
        )
        .await;
    // What the monitor shows at each frame, from the first seek on to another frame than the
    // one shown after the load.
    let gallery = dir.path().join("gallery");
    let mut monitor = Vec::new();
    for frame in FRAMES {
        monitor.push(shown(&mut client, &gallery, frame).await);
    }

    // The film of the timeline as it is now: each frame of it as the monitor showed it, but for
    // the loss of its codec.
    let path = output.path().join(leaf);
    let done = rendered(
        &mut client,
        json!({"timeline": timeline, "path": path, "codec": codec, "quality": "high"}),
    )
    .await;
    assert_eq!(done["state"], "done", "{done}");
    assert_eq!(done["frame"], 60, "{done}");
    let facts = columbina_media::probe(&path).unwrap();
    assert_eq!(facts.video.as_ref().unwrap().codec, codec);
    let mut decoder = VideoDecoder::open(&path, Hardware::Off).unwrap();
    let mut converter = Converter::new();
    let mut compose = Compositor::new(Arc::clone(&gpu));
    let target = Target::preview(&gpu, 320, 180).unwrap();
    let mut exported = Vec::new();
    for (frame, seen) in FRAMES.into_iter().zip(&monitor) {
        let picture = converter
            .native(&decoder.decoded_at(frame).unwrap())
            .unwrap();
        let made = Shown::of_film(picture, &mut compose, &target);
        let far = distance(seen, &made);
        eprintln!("frame {frame} of the {codec} film: {far:?}");
        assert!(
            far.within(tolerance),
            "frame {frame} of the film is {far:?} from the monitor's, beyond {tolerance:?}"
        );
        exported.push(made);
    }
    assert!(decoder.frame_at(60).is_err(), "a film of 60 frames");
    wrong_frames_fail(
        &mut client,
        &gallery,
        &names,
        &monitor,
        &exported,
        tolerance,
    )
    .await;
}
