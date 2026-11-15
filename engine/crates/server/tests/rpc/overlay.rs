//! `view.overlay`: the lines and dots that a page of the desktop has the view of the timeline
//! monitor draw over its picture (the motion path of a clip on the Edit page).
use super::*;

/// What `view.overlay` answers of the timeline monitor: how many lines and dots it draws.
fn drawn(lines: u64, dots: u64) -> Value {
    json!({"monitor":"timeline","lines":lines,"dots":dots})
}

/// A line of three points, two ticks, a key, no dragged key and the place at the playhead.
fn path() -> Value {
    json!({
        "canvas":[1920,1080],
        "lines":[[[0.0,0.0],[960.0,540.0],[1920.5,-20.25]]],
        "dots":[
            {"kind":"tick","at":[[0.0,0.0],[960.0,540.0]]},
            {"kind":"key","at":[[960.0,540.0]]},
            {"kind":"active","at":[]},
            {"kind":"current","at":[[1.0,2.0]]}
        ]
    })
}

#[tokio::test]
async fn an_overlay_is_kept_without_a_window_and_changes_nothing_of_the_project() {
    let dir = project_dir();
    let engine = engine(dir.path(), &[]).await;
    let mut desktop = engine.client("desktop").await;
    let before = desktop.result("project.get", None).await;
    let asked = json!({"monitor":"timeline"});
    assert_eq!(
        desktop.result("view.overlay", Some(asked.clone())).await,
        drawn(0, 0)
    );
    assert_eq!(
        desktop
            .result(
                "view.overlay",
                Some(json!({"monitor":"timeline","overlay":path()}))
            )
            .await,
        drawn(1, 4)
    );
    assert_eq!(
        desktop.result("view.overlay", Some(asked.clone())).await,
        drawn(1, 4)
    );
    // Another takes its place whole: only dots, and those of the kinds that it names.
    let dots = json!({"canvas":[128,72],"dots":[{"kind":"key","at":[[1.5,2.5],[3.5,4.5]]}]});
    assert_eq!(
        desktop
            .result(
                "view.overlay",
                Some(json!({"monitor":"timeline","overlay":dots}))
            )
            .await,
        drawn(0, 2)
    );
    // As many points as an overlay may have are taken.
    let most: Vec<[f64; 2]> = (0..32_768).map(|at| [f64::from(at), 0.0]).collect();
    assert_eq!(
        desktop
            .result(
                "view.overlay",
                Some(json!({"monitor":"timeline","overlay":{"canvas":[128,72],"lines":[most]}}))
            )
            .await,
        drawn(1, 0)
    );
    assert_eq!(
        desktop
            .result(
                "view.overlay",
                Some(json!({"monitor":"timeline","overlay":null}))
            )
            .await,
        drawn(0, 0)
    );
    assert_eq!(
        desktop.result("view.overlay", Some(asked)).await,
        drawn(0, 0)
    );
    assert_eq!(desktop.result("project.get", None).await, before);
}

#[tokio::test]
async fn only_the_desktop_of_an_engine_with_a_window_draws_an_overlay() {
    let dir = project_dir();
    let engine = engine(dir.path(), &[]).await;
    let mut agent = engine.client("agent").await;
    assert_eq!(
        agent
            .error("view.overlay", Some(json!({"monitor":"timeline"})))
            .await["code"],
        -32600
    );
    let other = project_dir();
    let headless = self::engine(other.path(), &["--headless"]).await;
    let mut desktop = headless.client("desktop").await;
    assert_eq!(
        desktop
            .error("view.overlay", Some(json!({"monitor":"timeline"})))
            .await["code"],
        -32601
    );
}

#[tokio::test]
async fn the_parameters_of_an_overlay_are_checked_and_one_refused_changes_nothing() {
    let dir = project_dir();
    let engine = engine(dir.path(), &[]).await;
    let mut desktop = engine.client("desktop").await;
    desktop
        .result(
            "view.overlay",
            Some(json!({"monitor":"timeline","overlay":path()})),
        )
        .await;
    let too_many: Vec<[f64; 2]> = (0..32_769).map(|at| [f64::from(at), 0.0]).collect();
    for (overlay, reason) in [
        (json!({}), None),
        (json!({"canvas":[1920]}), None),
        (json!({"canvas":[1920,1080],"extra":1}), None),
        (json!({"canvas":[1920,1080],"lines":[[[0.0]]]}), None),
        (
            json!({"canvas":[1920,1080],"lines":[[[0.0,1.0,2.0]]]}),
            None,
        ),
        (json!({"canvas":[1920,1080],"lines":[[[0.0,null]]]}), None),
        (
            json!({"canvas":[1920,1080],"dots":[{"kind":"handle","at":[]}]}),
            None,
        ),
        (json!({"canvas":[1920,1080],"dots":[{"kind":"key"}]}), None),
        (
            json!({"canvas":[1920,1080],"dots":[{"kind":"key","at":[],"size":3}]}),
            None,
        ),
        (json!({"canvas":[0,1080]}), Some("overlay")),
        (json!({"canvas":[1920,16385]}), Some("overlay")),
        (
            json!({"canvas":[1920,1080],"lines":[[[0.0,1.0e7],[1.0e7 + 1.0,0.0]]]}),
            Some("overlay"),
        ),
        (
            json!({"canvas":[1920,1080],"dots":[{"kind":"tick","at":[[-2.0e7,0.0]]}]}),
            Some("overlay"),
        ),
        (
            json!({"canvas":[1920,1080],"lines":[too_many]}),
            Some("overlay"),
        ),
    ] {
        let error = desktop
            .error(
                "view.overlay",
                Some(json!({"monitor":"timeline","overlay":overlay.clone()})),
            )
            .await;
        assert_eq!(error["code"], -32602, "{overlay}: {error}");
        if let Some(reason) = reason {
            assert_eq!(error["data"]["reason"], reason, "{overlay}: {error}");
        }
    }
    for params in [
        json!({}),
        json!({"monitor":"unknown"}),
        json!({"monitor":"timeline","extra":1}),
        json!({"overlay":null}),
    ] {
        assert_eq!(
            desktop.error("view.overlay", Some(params.clone())).await["code"],
            -32602,
            "{params}"
        );
    }
    // The source monitor shows media: nothing is drawn over it, nor asked of it.
    for params in [
        json!({"monitor":"source"}),
        json!({"monitor":"source","overlay":null}),
        json!({"monitor":"source","overlay":path()}),
    ] {
        let error = desktop.error("view.overlay", Some(params.clone())).await;
        assert_eq!(
            (&error["code"], &error["data"]["reason"]),
            (&json!(-32602), &json!("monitor")),
            "{params}"
        );
    }
    // What was refused changed nothing of what is drawn.
    assert_eq!(
        desktop
            .result("view.overlay", Some(json!({"monitor":"timeline"})))
            .await,
        drawn(1, 4)
    );
}

#[tokio::test]
async fn an_overlay_is_taken_by_the_connection_that_asks_last_and_goes_as_its_connection_closes() {
    let dir = project_dir();
    let engine = engine(dir.path(), &[]).await;
    let mut desktop = engine.client("desktop").await;
    let mut page = engine.client("desktop").await;
    let asked = json!({"monitor":"timeline"});
    desktop
        .result(
            "view.overlay",
            Some(json!({"monitor":"timeline","overlay":path()})),
        )
        .await;
    // The page's takes its place, and is the page's from then on.
    let dots = json!({"canvas":[128,72],"dots":[{"kind":"key","at":[[1.5,2.5]]}]});
    assert_eq!(
        page.result(
            "view.overlay",
            Some(json!({"monitor":"timeline","overlay":dots}))
        )
        .await,
        drawn(0, 1)
    );
    assert_eq!(
        desktop.result("view.overlay", Some(asked.clone())).await,
        drawn(0, 1)
    );
    drop(page);
    let deadline = Instant::now() + WAIT;
    loop {
        let kept = desktop.result("view.overlay", Some(asked.clone())).await;
        if kept == drawn(0, 0) {
            break;
        }
        assert!(Instant::now() < deadline, "the overlay stays: {kept}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    // One that is the desktop's own stays when another connection closes.
    desktop
        .result(
            "view.overlay",
            Some(json!({"monitor":"timeline","overlay":path()})),
        )
        .await;
    let agent = engine.client("agent").await;
    drop(agent);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        desktop.result("view.overlay", Some(asked)).await,
        drawn(1, 4)
    );
}

/// The grey of the pixel of the parent's window at `(x, y)`, once `wanted` holds of it.
#[cfg(windows)]
async fn until_grey(
    parent: &parent::Parent,
    (x, y): (usize, usize),
    what: &str,
    wanted: impl Fn(u8) -> bool,
) -> u8 {
    let deadline = Instant::now() + WAIT;
    loop {
        let (pixels, width) = parent.grey();
        let grey = pixels[y * width + x];
        if wanted(grey) {
            return grey;
        }
        assert!(Instant::now() < deadline, "{what}: {grey}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[cfg(windows)]
#[tokio::test]
async fn an_overlay_is_drawn_over_the_picture_by_each_view_until_its_connection_closes() {
    // The pixels of the view are looked at, as those of the outline of a window are: on a card.
    if columbina_compose::card_for_tests().is_none() {
        return;
    }
    let dir = project_dir();
    let engine = engine(dir.path(), &[]).await;
    let parent = parent::Parent::shown();
    let mut desktop = engine.client("desktop").await;
    let mut page = engine.client("desktop").await;
    let mut color = clip("$clip", "unused", 0, 100);
    color["source"] = json!({"kind":"color","color":"#dc5028ff"});
    let made = desktop.apply(json!([{"op":"add_timeline","timeline":{
        "id":"$timeline","name":"Path","width":128,"height":72,
        "rate":{"num":25,"den":1},"sample_rate":48000,
        "tracks":[{"id":"$track","name":"Picture","kind":"video","enabled":true,"locked":false,"clips":[color],"transitions":[]}]
    }}])).await;
    let timeline = made["names"]["$timeline"].clone();
    desktop
        .result("view.display", Some(json!({"hdr":false})))
        .await;
    let attach = json!({"monitor":"timeline","parent":parent.hwnd.to_string(),"x":0,"y":0,"width":128,"height":72});
    // An overlay asked for before there is a view is drawn by the view once it is made.
    let key = json!({"canvas":[128,72],"dots":[{"kind":"key","at":[[64.5,36.5]]}]});
    page.result(
        "view.overlay",
        Some(json!({"monitor":"timeline","overlay":key})),
    )
    .await;
    desktop.result("view.attach", Some(attach.clone())).await;
    desktop
        .result(
            "transport.load",
            Some(json!({"monitor":"timeline","timeline":timeline})),
        )
        .await;
    until_views(&parent, "overlay view", |views| views.len() == 1).await;
    let picture = |grey: u8| (41..150).contains(&grey);
    until_grey(&parent, (20, 36), "the picture is shown", picture).await;
    until_grey(&parent, (64, 36), "the disk of the key is white", |grey| {
        grey > 220
    })
    .await;
    // Marks made for a picture of another size are not drawn over this one.
    let other = json!({"canvas":[1920,1080],"dots":[{"kind":"key","at":[[64.5,36.5]]}]});
    page.result(
        "view.overlay",
        Some(json!({"monitor":"timeline","overlay":other})),
    )
    .await;
    until_grey(&parent, (64, 36), "another size is not drawn", picture).await;
    // Drawn again as it is given again, with no new picture; and by a view made in place of this one.
    page.result(
        "view.overlay",
        Some(json!({"monitor":"timeline","overlay":key})),
    )
    .await;
    until_grey(&parent, (64, 36), "the key is drawn again", |grey| {
        grey > 220
    })
    .await;
    desktop.result("view.attach", Some(attach)).await;
    until_views(&parent, "the view made in its place", |views| {
        views.len() == 1
    })
    .await;
    until_grey(&parent, (20, 36), "the picture is shown again", picture).await;
    until_grey(&parent, (64, 36), "the next view draws the key", |grey| {
        grey > 220
    })
    .await;
    // It goes with the connection that asked for it.
    drop(page);
    until_grey(&parent, (64, 36), "the key goes with its page", picture).await;
}
