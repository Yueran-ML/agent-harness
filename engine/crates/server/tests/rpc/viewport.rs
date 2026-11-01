use super::*;

#[tokio::test]
async fn the_viewport_is_remembered_without_a_window() {
    let dir = project_dir();
    let engine = engine(dir.path(), &[]).await;
    let mut desktop = engine.client("desktop").await;
    let before = desktop.result("project.get", None).await;
    for monitor in ["source", "timeline"] {
        assert_eq!(
            desktop
                .result("view.viewport", Some(json!({"monitor":monitor})))
                .await,
            json!({"monitor":monitor,"mode":"fit"})
        );
    }
    let custom =
        json!({"monitor":"source","mode":"scale","scale":2.0,"center":{"x":-0.25,"y":1.25}});
    assert_eq!(
        desktop.result("view.viewport", Some(custom.clone())).await,
        custom
    );
    desktop
        .result("view.detach", Some(json!({"monitor":"source"})))
        .await;
    assert_eq!(
        desktop
            .result("view.viewport", Some(json!({"monitor":"source"})))
            .await,
        custom
    );
    assert_eq!(
        desktop
            .result("view.viewport", Some(json!({"monitor":"timeline"})))
            .await,
        json!({"monitor":"timeline","mode":"fit"})
    );
    assert_eq!(desktop.result("project.get", None).await, before);
    assert_eq!(
        desktop
            .result(
                "view.viewport",
                Some(json!({"monitor":"source","mode":"fit"}))
            )
            .await,
        json!({"monitor":"source","mode":"fit"})
    );
}

#[tokio::test]
async fn viewport_parameters_and_desktop_access_are_checked() {
    let dir = project_dir();
    let engine = engine(dir.path(), &[]).await;
    let mut desktop = engine.client("desktop").await;
    let mut agent = engine.client("agent").await;
    assert_eq!(
        agent
            .error("view.viewport", Some(json!({"monitor":"source"})))
            .await["code"],
        -32600
    );
    for params in [
        json!({}),
        json!({"monitor":"unknown"}),
        json!({"monitor":"source","extra":1}),
        json!({"monitor":"source","mode":null}),
        json!({"monitor":"source","scale":1.0}),
        json!({"monitor":"source","mode":"fit","center":{"x":0.5,"y":0.5}}),
        json!({"monitor":"source","mode":"scale","scale":1.0}),
        json!({"monitor":"source","mode":"scale","scale":0,"center":{"x":0.5,"y":0.5}}),
        json!({"monitor":"source","mode":"scale","scale":-1,"center":{"x":0.5,"y":0.5}}),
        json!({"monitor":"source","mode":"scale","scale":1e-100,"center":{"x":0.5,"y":0.5}}),
        json!({"monitor":"source","mode":"scale","scale":1e-40,"center":{"x":0.5,"y":0.5}}),
        json!({"monitor":"source","mode":"scale","scale":1e34,"center":{"x":0.5,"y":0.5}}),
        json!({"monitor":"source","mode":"scale","scale":1e35,"center":{"x":0.5,"y":0.5}}),
        json!({"monitor":"source","mode":"scale","scale":1.0,"center":{"x":1e40,"y":0.5}}),
        json!({"monitor":"source","mode":"scale","scale":1.0,"center":{"x":0.5,"y":0.5,"z":0}}),
    ] {
        assert_eq!(
            desktop.error("view.viewport", Some(params.clone())).await["code"],
            -32602,
            "{params}"
        );
    }
    assert_eq!(
        desktop
            .result("view.viewport", Some(json!({"monitor":"source"})))
            .await,
        json!({"monitor":"source","mode":"fit"})
    );
    let other = project_dir();
    let headless = self::engine(other.path(), &["--headless"]).await;
    let mut desktop = headless.client("desktop").await;
    assert_eq!(
        desktop
            .error("view.viewport", Some(json!({"monitor":"source"})))
            .await["code"],
        -32601
    );
}

#[cfg(windows)]
#[tokio::test]
async fn replacing_and_reattaching_a_view_keeps_its_requested_viewport() {
    let dir = project_dir();
    let engine = engine(dir.path(), &[]).await;
    let parent = parent::Parent::open();
    let mut desktop = engine.client("desktop").await;
    let custom =
        json!({"monitor":"source","mode":"scale","scale":1.5,"center":{"x":0.25,"y":0.75}});
    desktop.result("view.viewport", Some(custom.clone())).await;
    let attach = json!({"monitor":"source","parent":parent.hwnd.to_string(),"x":0,"y":0,"width":64,"height":36});
    for replace in [false, true, false] {
        if !replace {
            desktop
                .result("view.detach", Some(json!({"monitor":"source"})))
                .await;
        }
        desktop.result("view.attach", Some(attach.clone())).await;
        assert_eq!(
            desktop
                .result("view.viewport", Some(json!({"monitor":"source"})))
                .await,
            custom
        );
    }
}
