//! Export uses the project snapshot, original media, and the preview compositor.

use std::path::Path;
use std::sync::Arc;

use columbina_compose::{
    Compositor, LayerColor, Scene, SceneLayer, Source, Target, Text, Transform, YuvCoding,
};
use columbina_media::{AudioDecoder, Codec, Converter, Hardware, NativePicture, VideoDecoder};
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
    json!({"timeline":timeline,"path":path,"codec":"prores","quality":"high","replace":false})
}

/// A request to render a timeline that `timeline` made, with an SRT of its track of subtitles.
fn with_srt(timeline: &Value, path: &Path) -> Value {
    let mut params = request(timeline, path);
    params["subtitle_tracks"] = json!(["S1"]);
    params
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
async fn a_refused_export_takes_no_partial_file_away() {
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
    let part = output.path().join("cut.mov.part");
    std::fs::write(&part, "left by a crash").unwrap();
    // The SRT beside the film cannot be written: the export is refused, and the partial file
    // of the film that replace would take away is still there.
    std::fs::create_dir(path.with_extension("srt")).unwrap();
    let mut params = with_srt(&timeline, &path);
    params["replace"] = json!(true);
    let error = client.error("render.start", Some(params)).await;
    assert_eq!(error["code"], -32602, "{error}");
    assert_eq!(std::fs::read_to_string(&part).unwrap(), "left by a crash");
    drop(client);
    assert_eq!(engine.ended().await, 0);
}

#[tokio::test]
async fn render_names_a_missing_timeline_and_refuses_a_revision_that_is_not_the_projects() {
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
    let part = output.path().join("cut.mov.part");
    std::fs::write(&part, "left by a crash").unwrap();
    // A timeline that is not there is named, as timeline.duration and transport.load name it.
    let mut params = request(&json!("t404"), &path);
    params["replace"] = json!(true);
    let error = client.error("render.start", Some(params)).await;
    assert_eq!(error["code"], -32602, "{error}");
    assert_eq!(
        error["data"],
        json!({"reason":"missing", "timeline":"t404"}),
        "{error}"
    );
    // The names of the SRT files were worked out at a revision that is not the project's any
    // more: refused as transaction.apply refuses it, before anything is written or taken away.
    let revision = client.result("project.get", None).await["revision"].clone();
    let actual = revision.as_u64().unwrap();
    let mut params = with_srt(&timeline, &path);
    params["replace"] = json!(true);
    params["revision"] = json!(actual - 1);
    let error = client.error("render.start", Some(params.clone())).await;
    assert_eq!(error["code"], -32001, "{error}");
    assert_eq!(
        error["data"],
        json!({"reason":"stale_revision", "expected":actual - 1, "actual":actual}),
        "{error}"
    );
    assert_eq!(std::fs::read_to_string(&part).unwrap(), "left by a crash");
    assert!(!path.with_extension("srt").exists());
    // At the project's revision it is taken, as it is with none.
    if columbina_compose::gpu_for_tests().is_some() {
        params["revision"] = revision;
        let job = client.result("render.start", Some(params)).await["job"].clone();
        let done = finished(&mut client, &job).await;
        assert_eq!(done["state"], "done", "{done}");
        assert!(!part.exists());
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
    let mut params = with_srt(&timeline, &path);
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
        "1\r\n00:00:00,000 --> 00:00:00,800\r\n旁边的字幕\r\n\r\n"
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
    let mut params = with_srt(&timeline, &path);
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

/// A timeline of `timeline`, with S1 (zh) shown, and S2 (en) and S3 (fr) hidden beside it.
async fn three_subtitles(client: &mut Client) -> Value {
    let timeline = timeline(client, 50).await;
    let subtitles = |language: &str, text: &str| {
        json!({"op":"add_track", "timeline":timeline, "track":{"id":"", "kind":"subtitle",
            "name":language, "enabled":false, "locked":false, "language":language,
            "cues":[{"id":"", "start":0, "length":25, "text":text}]}})
    };
    client
        .apply(json!([
            subtitles("en", "Beside"),
            subtitles("fr", "À côté")
        ]))
        .await;
    timeline
}

fn set_shown(timeline: &Value, id: &str, enabled: bool) -> Value {
    json!({"op":"set_track", "timeline":timeline, "id":id, "enabled":enabled})
}

#[tokio::test]
async fn a_timeline_shows_one_track_of_subtitles_at_a_time() {
    let dir = project_dir();
    let engine = engine(
        dir.path(),
        &["--headless", "--no-proxies", "--idle-seconds", "1"],
    )
    .await;
    let mut client = engine.client("agent").await;
    let timeline = three_subtitles(&mut client).await;
    // S1 is shown: S2 is not shown beside it, nor a track added shown.
    let ops = json!({"ops":[set_shown(&timeline, "S2", true)]});
    let refused = client.error("transaction.apply", Some(ops)).await;
    assert_eq!(refused["code"], -32001);
    assert_eq!(
        refused["data"],
        json!({"reason":"subtitle_shown", "track":"S1", "index":0, "message":
            "operation 0 of the transaction was refused: subtitle track S1 is shown, and a \
             timeline shows one subtitle track at a time"})
    );
    let shown = json!({"op":"add_track", "timeline":timeline, "track":{"id":"",
        "kind":"subtitle", "name":"de", "enabled":true, "locked":false, "language":"de",
        "cues":[]}});
    let refused = client
        .error("transaction.apply", Some(json!({"ops":[shown]})))
        .await;
    assert_eq!(refused["data"]["reason"], "subtitle_shown");
    // The edit adds a track of subtitles hidden while another is shown.
    let added = client
        .apply(json!([{"edit":"add_track", "timeline":timeline, "kind":"subtitle"}]))
        .await;
    assert_eq!(added["done"][0]["track"]["enabled"], false, "{added}");
    // S1 hidden, then S2 shown, in one transaction.
    client
        .apply(json!([
            set_shown(&timeline, "S1", false),
            set_shown(&timeline, "S2", true)
        ]))
        .await;
    let project = client.result("project.get", None).await["project"].clone();
    let shown: Vec<_> = project["timelines"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|t| t["tracks"].as_array().unwrap())
        .filter(|r| r["kind"] == "subtitle" && r["enabled"] == true)
        .map(|r| r["id"].clone())
        .collect();
    assert_eq!(shown, [json!("S2")]);
    drop(client);
    assert_eq!(engine.ended().await, 0);
}

#[tokio::test]
async fn render_writes_an_srt_of_each_chosen_track_of_subtitles_shown_or_hidden() {
    let dir = project_dir();
    let output = project_dir();
    let engine = engine(
        dir.path(),
        &["--headless", "--no-proxies", "--idle-seconds", "1"],
    )
    .await;
    let mut client = engine.client("agent").await;
    let timeline = three_subtitles(&mut client).await;
    client
        .apply(json!([
            set_shown(&timeline, "S1", false),
            set_shown(&timeline, "S2", true)
        ]))
        .await;
    // Names that are not a track of subtitles of the timeline, or one named twice, are refused
    // before anything is written; so is the old `subtitles`.
    let path = output.path().join("cut.mov");
    for (tracks, track) in [
        (json!(["S9"]), "S9"),
        (json!(["V1"]), "V1"),
        (json!(["S1", "S3", "S1"]), "S1"),
    ] {
        let mut params = request(&timeline, &path);
        params["subtitle_tracks"] = tracks;
        let error = client.error("render.start", Some(params)).await;
        assert_eq!(error["code"], -32602, "{error}");
        assert_eq!(
            error["data"],
            json!({"reason":"subtitle_track", "track":track})
        );
    }
    let mut params = request(&timeline, &path);
    params["subtitles"] = json!(true);
    assert_eq!(
        client.error("render.start", Some(params)).await["code"],
        -32602
    );
    assert!(std::fs::read_dir(output.path()).unwrap().next().is_none());
    if columbina_compose::gpu_for_tests().is_none() {
        return;
    }
    // S1 (zh) and S3 (fr) are chosen, both hidden; S2 (en), which is shown, is not.
    let mut params = request(&timeline, &path);
    params["subtitle_tracks"] = json!(["S3", "S1"]);
    params["range"] = json!({"from":0, "to":20});
    let job = client.result("render.start", Some(params)).await["job"].clone();
    let done = finished(&mut client, &job).await;
    assert_eq!(done["state"], "done", "{done}");
    let mut written: Vec<String> = std::fs::read_dir(output.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    written.sort();
    assert_eq!(written, ["cut.fr.srt", "cut.mov", "cut.zh.srt"]);
    assert_eq!(
        std::fs::read_to_string(output.path().join("cut.zh.srt")).unwrap(),
        "1\r\n00:00:00,400 --> 00:00:00,800\r\n旁边的字幕\r\n\r\n"
    );
    assert_eq!(
        std::fs::read_to_string(output.path().join("cut.fr.srt")).unwrap(),
        "1\r\n00:00:00,000 --> 00:00:00,800\r\nÀ côté\r\n\r\n"
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

/// The colour of a layer of a picture, as the engine decodes it: by the matrix and range that the
/// picture says.
fn coded(picture: &NativePicture) -> LayerColor {
    LayerColor {
        coding: YuvCoding::of_native(picture),
        ..LayerColor::default()
    }
}

fn reference(frame: i64, decoder: &mut VideoDecoder, style: &Value) -> Scene {
    let left = if frame < 35 {
        10 + frame
    } else if frame < 60 {
        30 + frame
    } else {
        frame - 60
    };
    let mut layer = |frame, opacity| {
        let picture = Converter::new()
            .native(&decoder.decoded_at(frame).unwrap())
            .unwrap();
        SceneLayer {
            color: coded(&picture),
            source: Source::Picture(Arc::new(picture), 0),
            transform: Transform::NONE,
            opacity,
        }
    };
    let mut scene = Scene {
        width: 320,
        height: 180,
        layers: vec![layer(left, 1.0)],
        ..Scene::default()
    };
    if (25..35).contains(&frame) {
        #[allow(clippy::cast_precision_loss)]
        let opacity = (frame - 24) as f32 / 11.0;
        scene.layers.push(layer(frame + 30, opacity));
    }
    if (40..55).contains(&frame) {
        scene.layers.push(SceneLayer {
            source: Source::Title(Text {
                text: "Columbina".into(),
                style: serde_json::from_value(style.clone()).unwrap(),
            }),
            transform: Transform::NONE,
            opacity: 1.0,
            color: LayerColor::default(),
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
        let picture = Converter::new()
            .native(&export.decoded_at(frame).unwrap())
            .unwrap();
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
                        color: coded(&picture),
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

/// The first sample, of the left channel of interleaved stereo, within `window` of `around`
/// that is louder than half of the beeps of [`support::Sample::Clicks`].
fn onset(sound: &[f32], around: usize, window: usize) -> Option<usize> {
    let pairs = sound.as_chunks::<2>().0;
    let from = around.saturating_sub(window);
    let to = (around + window).min(pairs.len());
    (from..to).find(|&at| pairs[at][0].abs() > 0.25)
}

#[tokio::test]
async fn render_puts_each_sound_at_the_frame_of_its_picture() {
    if columbina_compose::gpu_for_tests().is_none() {
        return;
    }
    let dir = project_dir();
    let output = project_dir();
    let Some((_engine, mut client)) =
        transport_engine_with(dir.path(), &[support::Sample::Clicks], &["--headless"]).await
    else {
        return;
    };
    let state = project(&mut client).await;
    let scale = state["project"]["media"][0]["facts"]["duration"]["scale"]
        .as_i64()
        .unwrap();
    let mut color = clip("$color", "unused", 0, 100);
    color["source"] = json!({"kind":"color", "color":"#202020ff"});
    // The clicks from 0.2 s of their file on at frame 10: the beeps at 1, 2 and 3 s of the
    // file are at frames 30, 55 and 80 of the timeline.
    let mut sound = clip("$sound", "m1", 10, 85);
    sound["source"]["in"] = json!({"ticks":scale / 5, "scale":scale});
    let track = |id: &str, kind: &str, clips: Value| {
        json!({"id":id,"kind":kind,"name":id,
        "enabled":true,"locked":false,"clips":clips,"transitions":[]})
    };
    let names = client
        .apply(json!([{"op":"add_timeline","timeline":{
            "id":"$t", "name":"Clicks", "width":320,"height":180,
            "rate":{"num":25,"den":1},"sample_rate":48000,
            "tracks":[track("$v","video",json!([color])),track("$a","audio",json!([sound]))]
        }}]))
        .await["names"]
        .clone();
    let path = output.path().join("clicks.mov");
    let mut params = request(&names["$t"], &path);
    params["range"] = json!({"from":5,"to":95});
    let job = client.result("render.start", Some(params)).await["job"].clone();
    let done = finished(&mut client, &job).await;
    assert_eq!(done["state"], "done", "{done}");
    let mut audio = AudioDecoder::open(&path, None).unwrap();
    let mut sound = Vec::new();
    while let Some(run) = audio.next().unwrap() {
        sound.extend(run.data);
    }
    // Exported from frame 5: the beeps are at frames 25, 50 and 75 of the file, 1920 samples
    // a frame. Each is heard within a millisecond (48 samples) of its frame.
    for frame in [25, 50, 75] {
        let expected = frame * 1920;
        let heard = onset(&sound, expected, 4800)
            .unwrap_or_else(|| panic!("no beep near frame {frame} of the export"));
        assert!(
            heard.abs_diff(expected) <= 48,
            "the beep of frame {frame} is at sample {heard}, not {expected}"
        );
    }
    // Nothing is heard between the beeps.
    assert_eq!(onset(&sound, 37 * 1920, 9 * 1920), None);
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
