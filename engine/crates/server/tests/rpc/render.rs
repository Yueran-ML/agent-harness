//! Export uses the project snapshot, original media, and the preview compositor.

use std::path::Path;
use std::sync::Arc;

use columbina_compose::{Compositor, Scene, SceneLayer, Source, Target, Text, Transform};
use columbina_media::{AudioDecoder, Codec, Hardware, VideoDecoder};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::{clip, engine, project, project_dir, support, transport_engine_with, Client};

async fn timeline(client: &mut Client, length: i64) -> Value {
    let mut color = clip("$color", "unused", 0, length);
    color["source"] = json!({"kind":"color", "color":"#ff0000ff"});
    client
        .apply(json!([{"op":"add_timeline", "timeline":{
            "id":"$t", "name":"Export", "width":320, "height":180,
            "rate":{"num":25,"den":1}, "sample_rate":48000,
            "tracks":[{"id":"$v", "kind":"video", "name":"Picture", "enabled":true,
                "locked":false, "clips":[color], "transitions":[]},
                {"id":"$s", "kind":"subtitle", "name":"Subtitles", "enabled":true,
                "locked":false, "language":"zh", "cues":[
                    {"id":"$q", "start":10,"length":40,"text":"旁边的字幕"}]}]
        }}]))
        .await["names"]["$t"]
        .clone()
}

fn request(timeline: &Value, path: &Path) -> Value {
    json!({"timeline":timeline,"path":path,"codec":"prores","quality":"high",
        "subtitles":true,"replace":false})
}

async fn finished(client: &mut Client, job: &Value) -> Value {
    let mut pending = client.take_notifications().into_iter();
    loop {
        let message = match pending.next() {
            Some(value) => value,
            None => client.message().await,
        };
        if message["method"] == "render.progress"
            && message["params"]["job"] == *job
            && message["params"]["state"] != "running"
        {
            return message["params"].clone();
        }
    }
}

#[tokio::test]
async fn render_refuses_unsafe_paths_ranges_and_existing_files_before_starting() {
    let dir = project_dir();
    let output = project_dir();
    let engine = engine(
        dir.path(),
        &["--headless", "--no-proxies", "--idle-seconds", "1"],
    )
    .await;
    let mut client = engine.client("agent").await;
    let timeline = timeline(&mut client, 100).await;
    let path = output.path().join("cut.mov");
    std::fs::write(&path, "keep").unwrap();
    let error = client
        .error("render.start", Some(request(&timeline, &path)))
        .await;
    assert_eq!(error["code"], -32001);
    assert_eq!(error["data"]["reason"], "exists");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "keep");
    for bad in [
        Path::new("relative.mov").to_owned(),
        output.path().join("x.mp4"),
        output.path().join("missing/x.mov"),
        dir.path().join("cache/x.mov"),
    ] {
        let error = client
            .error("render.start", Some(request(&timeline, &bad)))
            .await;
        assert_eq!(error["code"], -32602, "{error}");
    }
    for range in [
        json!({"from":10,"to":10}),
        json!({"from":-1,"to":10}),
        json!({"from":0,"to":101}),
    ] {
        let mut params = request(&timeline, &output.path().join("new.mov"));
        params["range"] = range;
        assert_eq!(
            client.error("render.start", Some(params)).await["code"],
            -32602
        );
    }
    drop(client);
    assert_eq!(engine.ended().await, 0);
}

#[tokio::test]
async fn render_works_headlessly_with_clipped_srt_and_no_burned_subtitles() {
    if columbina_compose::gpu_for_tests().is_none() {
        return;
    }
    let dir = project_dir();
    let output = project_dir();
    let engine = engine(
        dir.path(),
        &["--headless", "--no-proxies", "--idle-seconds", "1"],
    )
    .await;
    let mut client = engine.client("agent").await;
    let timeline = timeline(&mut client, 100).await;
    let path = output.path().join("cut.mov");
    let mut params = request(&timeline, &path);
    params["range"] = json!({"from":20,"to":40});
    let job = client.result("render.start", Some(params)).await["job"].clone();
    let done = finished(&mut client, &job).await;
    assert_eq!(done["state"], "done", "{done}");
    assert_eq!(done["frames"], 20);
    assert_eq!(done["frame"], 20);
    assert_eq!(done["encoder"], "prores_ks");
    assert_eq!(done["path"], json!(path));
    assert_eq!(
        std::fs::read_to_string(path.with_extension("srt")).unwrap(),
        "1\n00:00:00,000 --> 00:00:00,800\n旁边的字幕\n\n"
    );
    assert!(!output.path().join("cut.mov.part").exists());
    let mut decoder =
        columbina_media::VideoDecoder::open(&path, columbina_media::Hardware::Off).unwrap();
    for frame in 0..20 {
        let raw = decoder.decoded_at(frame).unwrap();
        let rgba = columbina_media::Converter::new()
            .rgba_scaled(&raw, 320, 180)
            .unwrap();
        assert!(rgba
            .data
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| p[0] > 240 && p[1] < 12 && p[2] < 12));
    }
    assert!(decoder.frame_at(20).is_err());
    drop(client);
    assert_eq!(engine.ended().await, 0);
}

#[tokio::test]
async fn render_cancellation_keeps_previous_files_and_releases_the_single_job_slot() {
    if columbina_compose::gpu_for_tests().is_none() {
        return;
    }
    let dir = project_dir();
    let output = project_dir();
    let engine = engine(
        dir.path(),
        &["--headless", "--no-proxies", "--idle-seconds", "1"],
    )
    .await;
    let mut client = engine.client("agent").await;
    let timeline = timeline(&mut client, 10_000).await;
    let path = output.path().join("cut.mov");
    std::fs::write(&path, b"old movie").unwrap();
    std::fs::write(path.with_extension("srt"), b"old subtitles").unwrap();
    let mut params = request(&timeline, &path);
    params["replace"] = json!(true);
    let job = client.result("render.start", Some(params.clone())).await["job"].clone();
    let busy = client.error("render.start", Some(params.clone())).await;
    assert_eq!(busy["code"], -32001);
    assert_eq!(busy["data"]["reason"], "busy");
    client
        .result("render.cancel", Some(json!({"job":job})))
        .await;
    let done = finished(&mut client, &job).await;
    assert_eq!(done["state"], "cancelled", "{done}");
    assert_eq!(std::fs::read(&path).unwrap(), b"old movie");
    assert_eq!(
        std::fs::read(path.with_extension("srt")).unwrap(),
        b"old subtitles"
    );
    assert!(!output.path().join("cut.mov.part").exists());
    assert!(!output.path().join("cut.srt.part").exists());
    params["range"] = json!({"from":0,"to":3});
    let second = client.result("render.start", Some(params)).await["job"].clone();
    assert_ne!(job, second);
    let done = finished(&mut client, &second).await;
    assert_eq!(done["state"], "done", "{done}");
    assert!(std::fs::metadata(&path).unwrap().len() > 100);
    assert_eq!(std::fs::read(path.with_extension("srt")).unwrap(), b"");
    assert_eq!(
        client
            .error("render.cancel", Some(json!({"job":"missing"})))
            .await["code"],
        -32602
    );
    drop(client);
    assert_eq!(engine.ended().await, 0);
}

async fn cut(client: &mut Client, scale: i64) -> (Value, Value) {
    let mut first = clip("$one", "m1", 0, 30);
    first["source"]["in"] = json!({"ticks":10*scale/25,"scale":scale});
    let mut second = clip("$two", "m1", 30, 30);
    second["source"]["in"] = json!({"ticks":60*scale/25,"scale":scale});
    let mut third = clip("$three", "m1", 60, 25);
    third["source"]["in"] = json!({"ticks":0,"scale":scale});
    let style = json!({"font":"", "size":28,"color":"#ffffffff","x":0.5,"y":0.5,"align":"center"});
    let mut title = clip("$title", "unused", 40, 15);
    title["source"] = json!({"kind":"title","text":"Columbina","style":style});
    let mut left = clip("$left", "m1", 0, 85);
    left["source"]["in"] = json!({"ticks":0,"scale":scale});
    left["gain_db"] = json!(-6.0);
    left["pan"] = json!(-1.0);
    let mut right = left.clone();
    right["id"] = json!("$right");
    right["gain_db"] = json!(-12.0);
    right["pan"] = json!(1.0);
    let track = |id: &str, kind: &str, clips: Value| {
        json!({"id":id,"kind":kind,"name":id,
        "enabled":true,"locked":false,"clips":clips,"transitions":[]})
    };
    let names = client.apply(json!([{"op":"add_timeline","timeline":{
        "id":"$t", "name":"Three clips", "width":320,"height":180,
        "rate":{"num":25,"den":1},"sample_rate":48000,
        "tracks":[track("$v","video",json!([first,second,third])),track("$title_track","video",json!([title])),
            track("$a1","audio",json!([left])),track("$a2","audio",json!([right]))]
    }}])).await["names"].clone();
    client
        .apply(
            json!([{"op":"add_transition","timeline":names["$t"],"track":names["$v"],
        "transition":{"id":"$d","kind":"dissolve","left":names["$one"],"right":names["$two"],
            "length":10,"align":"center"}}]),
        )
        .await;
    (names, style)
}

fn reference(frame: i64, decoder: &mut VideoDecoder, style: &Value) -> Scene {
    let left = if frame < 35 {
        10 + frame
    } else if frame < 60 {
        30 + frame
    } else {
        frame - 60
    };
    let layer = |picture, opacity| SceneLayer {
        source: Source::Picture(Arc::new(picture), 0),
        transform: Transform::NONE,
        opacity,
    };
    let mut scene = Scene {
        width: 320,
        height: 180,
        layers: vec![layer(decoder.frame_at(left).unwrap(), 1.0)],
        ..Scene::default()
    };
    if (25..35).contains(&frame) {
        #[allow(clippy::cast_precision_loss)]
        let opacity = (frame - 24) as f32 / 11.0;
        scene
            .layers
            .push(layer(decoder.frame_at(frame + 30).unwrap(), opacity));
    }
    if (40..55).contains(&frame) {
        scene.layers.push(SceneLayer {
            source: Source::Title(Text {
                text: "Columbina".into(),
                style: serde_json::from_value(style.clone()).unwrap(),
            }),
            transform: Transform::NONE,
            opacity: 1.0,
        });
    }
    scene
}

fn check_audio(path: &Path) {
    let mut audio = AudioDecoder::open(path, None).unwrap();
    let mut sound = Vec::new();
    while let Some(run) = audio.next().unwrap() {
        sound.extend(run.data);
    }
    assert!(
        (163_200..=164_224).contains(&(sound.len() / 2)),
        "{} stereo samples",
        sound.len() / 2
    );
    let pairs = &sound.as_chunks::<2>().0[24_000..72_000];
    let crossings = pairs
        .windows(2)
        .filter(|pair| pair[0][0] < 0.0 && pair[1][0] >= 0.0)
        .count();
    assert!(crossings.abs_diff(440) <= 2, "{crossings} crossings");
    let energy = pairs.iter().fold([0.0_f64; 2], |mut sum, pair| {
        sum[0] += f64::from(pair[0]).powi(2);
        sum[1] += f64::from(pair[1]).powi(2);
        sum
    });
    let ratio = (energy[0] / energy[1]).sqrt();
    assert!(
        (1.9..2.1).contains(&ratio),
        "two independent gains/pans: {ratio}"
    );
    assert!(energy[0] > 1.0);
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "one complete export compared frame by frame with independent source positions and preview"
)]
async fn render_h264_matches_preview_original_frames_dissolve_titles_and_stereo_mix() {
    let Some(gpu) = columbina_compose::gpu_for_tests() else {
        return;
    };
    if columbina_media::encoders_for(Codec::H264).is_empty() {
        assert!(
            std::env::var_os("COLUMBINA_REQUIRE_GPU").is_none(),
            "a hardware H264 encoder is required"
        );
        return;
    }
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
    let state = project(&mut client).await;
    let media = &state["project"]["media"][0];
    let (names, style) = cut(
        &mut client,
        media["facts"]["duration"]["scale"].as_i64().unwrap(),
    )
    .await;
    client
        .result(
            "transport.load",
            Some(json!({"monitor":"timeline","timeline":names["$t"]})),
        )
        .await;
    let path = output.path().join("film.mp4");
    let mut params = request(&names["$t"], &path);
    params["codec"] = json!("h264");
    let job = client.result("render.start", Some(params)).await["job"].clone();
    // Preview remains interactive while the independent export is running.
    client
        .result(
            "transport.seek",
            Some(json!({"monitor":"timeline","frame":30})),
        )
        .await;
    client
        .until_state("preview during export", |state| {
            state["monitor"] == "timeline" && state["shown"] == 30
        })
        .await;
    let done = finished(&mut client, &job).await;
    assert_eq!(done["state"], "done", "{done}");
    assert_eq!(done["frame"], 85);
    let facts = columbina_media::probe(&path).unwrap();
    assert_eq!(facts.video.as_ref().unwrap().codec, "h264");
    assert_eq!(facts.audio.as_ref().unwrap().rate, 48000);
    let mut original =
        VideoDecoder::open(Path::new(media["path"].as_str().unwrap()), Hardware::Auto).unwrap();
    let mut export = VideoDecoder::open(&path, Hardware::Off).unwrap();
    let mut compose = Compositor::new(Arc::clone(&gpu));
    let target = Target::preview(&gpu, 320, 180).unwrap();
    for frame in 0..85 {
        let scene = reference(frame, &mut original, &style);
        compose.render(&scene, &target).unwrap();
        let expected = target.read_rgba().unwrap();
        client.take_notifications();
        client
            .result(
                "transport.seek",
                Some(json!({"monitor":"timeline","frame":frame})),
            )
            .await;
        client
            .until_state("exact preview", |s| {
                s["monitor"] == "timeline" && s["shown"] == frame && s["exact"] == true
            })
            .await;
        let preview = client
            .result(
                "transport.frame_digest",
                Some(json!({"monitor":"timeline"})),
            )
            .await;
        assert_eq!(
            preview["hash"],
            format!("{:x}", Sha256::digest(&expected)),
            "preview frame {frame}"
        );
        let picture = export.frame_at(frame).unwrap();
        if !(25..35).contains(&frame) {
            let source_frame = if frame < 25 {
                frame + 10
            } else if frame < 60 {
                frame + 30
            } else {
                frame - 60
            };
            assert_eq!(
                support::number_in_blocks(&picture.luma, picture.luma_stride, picture.bits, 10),
                Some(source_frame),
                "encoded frame {frame}"
            );
        }
        compose
            .render(
                &Scene {
                    width: 320,
                    height: 180,
                    layers: vec![SceneLayer {
                        source: Source::Picture(Arc::new(picture), 0),
                        transform: Transform::NONE,
                        opacity: 1.0,
                    }],
                    ..Scene::default()
                },
                &target,
            )
            .unwrap();
        let actual = target.read_rgba().unwrap();
        for x in [5, 16, 27] {
            let at = (5 * 320 + x) * 4;
            for channel in 0..3 {
                assert!(
                    actual[at + channel].abs_diff(expected[at + channel]) <= 6,
                    "frame {frame}, x {x}, channel {channel}: {} != {}",
                    actual[at + channel],
                    expected[at + channel]
                );
            }
        }
        let difference: u64 = actual
            .iter()
            .zip(&expected)
            .map(|(a, b)| u64::from(a.abs_diff(*b)))
            .sum();
        assert!(
            difference < u64::try_from(actual.len()).unwrap() * 4,
            "frame {frame}: mean pixel error >= 4"
        );
    }
    assert!(export.frame_at(85).is_err());
    check_audio(&path);
}

#[tokio::test]
async fn render_uses_originals_and_keeps_its_snapshot_when_the_timeline_is_deleted() {
    if columbina_compose::gpu_for_tests().is_none() {
        return;
    }
    let dir = project_dir();
    let output = project_dir();
    let Some((_engine, mut client)) =
        transport_engine_with(dir.path(), &[support::Sample::Fhd], &["--headless"]).await
    else {
        return;
    };
    let state = project(&mut client).await;
    let media = &state["project"]["media"][0];
    let (names, _) = cut(
        &mut client,
        media["facts"]["duration"]["scale"].as_i64().unwrap(),
    )
    .await;
    // A valid but deliberately different proxy exposes accidental proxy use, unlike a
    // corrupt proxy that could silently fall back to the original and make this test pass.
    let proxies = dir.path().join("cache/proxies");
    std::fs::create_dir_all(&proxies).unwrap();
    let digest = media["identity"]["digest"].as_str().unwrap();
    let proxy = proxies.join(format!("m1-{}.mov", &digest[..12]));
    let mut writer = columbina_media::Writer::create(
        &proxy,
        columbina_media::VideoSettings {
            codec: Codec::ProRes,
            width: 64,
            height: 36,
            rate: columbina_model::Rate { num: 25, den: 1 },
            bits: 8,
            quality: columbina_media::Quality::Small,
        },
        None,
    )
    .unwrap();
    let black = columbina_media::Picture {
        width: 64,
        height: 36,
        bits: 8,
        luma: vec![16; 64 * 36],
        chroma: vec![128; 64 * 18],
        luma_stride: 64,
        chroma_stride: 64,
        ..Default::default()
    };
    for _ in 0..100 {
        writer.video(&black).unwrap();
    }
    writer.finish().unwrap();
    client
        .apply(json!([{"op":"set_proxy","media":"m1","proxy":{
        "path":proxy,"width":64,"height":36,"codec":"prores",
        "digest_of_source":media["identity"]["digest"]}}]))
        .await;
    let path = output.path().join("original.mov");
    let mut params = request(&names["$t"], &path);
    params["range"] = json!({"from":0,"to":25});
    let job = client.result("render.start", Some(params)).await["job"].clone();
    client
        .apply(json!([{"op":"remove_timeline","id":names["$t"]}]))
        .await;
    assert_eq!(
        project(&mut client).await["project"]["timelines"],
        json!([])
    );
    let done = finished(&mut client, &job).await;
    assert_eq!(done["state"], "done", "{done}");
    let mut decoder = VideoDecoder::open(&path, Hardware::Off).unwrap();
    for frame in [0, 12, 24] {
        let picture = decoder.frame_at(frame).unwrap();
        assert_eq!(
            support::number_in_blocks(&picture.luma, picture.luma_stride, picture.bits, 10),
            Some(frame + 10),
            "snapshot frame {frame}"
        );
    }
}
