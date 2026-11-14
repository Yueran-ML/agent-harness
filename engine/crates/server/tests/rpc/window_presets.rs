//! Window presets through the real engine (RM20 pp. 3198–3199; plan Q10): a library of this
//! user's that every project shares, saved from the windows of a node as they are on the frame
//! of the playhead, recalled onto a node of another project as one undo step that replaces the
//! node's windows, and deleted without touching any project.

use std::path::Path;

use serde_json::{json, Value};

use super::{clip, project, project_dir, start_in, Client, Engine, Started, Stderr};

async fn engine_with(dir: &Path, presets: &Path) -> Engine {
    match start_in(
        dir,
        &[],
        Stderr::Read,
        &[("COLUMBINA_WINDOW_PRESETS", presets.as_os_str())],
    )
    .await
    {
        Started::Listening(engine) => *engine,
        Started::Ended { code, stderr } => panic!("the engine ended with {code}: {stderr}"),
    }
}

/// A timeline with a colour clip whose grade has one corrector holding `windows`: the target of
/// that grade and the node's id.
async fn graded(client: &mut Client, windows: Value) -> (Value, Value) {
    let mut color = clip("$clip", "unused", 0, 100);
    color["source"] = json!({"kind":"color","color":"#dc5028ff"});
    let made = client
        .apply(json!([{"op":"add_timeline","timeline":{
            "id":"$timeline","name":"Presets","width":128,"height":72,
            "rate":{"num":25,"den":1},"sample_rate":48000,
            "tracks":[{"id":"$track","name":"Picture","kind":"video","enabled":true,"locked":false,
                "clips":[color],"transitions":[]}]
        }}]))
        .await;
    let target =
        json!({"level":"clip","timeline":made["names"]["$timeline"],"clip":made["names"]["$clip"]});
    let mut node = serde_json::to_value(columbina_model::Node::corrector("$node")).unwrap();
    node["params"]["windows"] = windows;
    let added = client
        .apply(json!([{"op":"add_node","grade":target,"node":node,"links":[
            {"from":"source","to":"$node","kind":"rgb","input":0},
            {"from":"$node","to":"output","kind":"rgb","input":0}]}]))
        .await;
    (target, added["names"]["$node"].clone())
}

fn windows_of(project: &Value) -> Value {
    project["project"]["timelines"][0]["tracks"][0]["clips"][0]["grade"]["nodes"][0]["params"]
        ["windows"]
        .clone()
}

fn circle(id: &str, size: &Value) -> Value {
    json!({"id":id,"label":"","enabled":true,"invert":false,"combine":"add","opacity":100,
        "transform":{"size":size,"aspect":50,"pan":50,"tilt":50,"rotate":0},
        "shape":{"kind":"circular","softness":1}})
}

#[tokio::test]
async fn a_preset_saved_in_one_project_replaces_the_windows_of_a_node_in_another_as_one_step() {
    let library = tempfile::tempdir().unwrap();
    let first_dir = project_dir();
    let first = engine_with(first_dir.path(), library.path()).await;
    let mut desktop = first.client("desktop").await;
    let mut other = first.client("agent").await;
    let keyed = json!({"keys":[{"frame":0,"value":20,"interp":"linear"},{"frame":10,"value":40,"interp":"linear"}]});
    let mut face = circle("w1", &keyed);
    face["label"] = json!("face");
    let (target, node) = graded(&mut desktop, json!([face, circle("w2", &json!(70))])).await;

    let listed = desktop.result("window_presets.list", None).await;
    assert_eq!(listed, json!({"presets": []}));
    let saved = desktop
        .result(
            "window_presets.save",
            Some(json!({"name":"Face","grade":target,"node":node,"frame":5})),
        )
        .await;
    assert_eq!(saved["id"], "wp1");
    assert_eq!(saved["name"], "Face");
    // Static shapes at the playhead's interpolated values: no keys, no tracking data.
    assert_eq!(saved["windows"][0]["transform"]["size"], 30.0);
    assert_eq!(saved["windows"][0]["label"], "face");
    assert_eq!(saved["windows"][1]["transform"]["size"], 70.0);
    assert!(!saved.to_string().contains("keys"), "{saved}");
    other.result("window_presets.list", None).await;
    assert!(other
        .take_notifications()
        .iter()
        .any(|note| note["method"] == "window_presets.changed"));
    let taken = desktop
        .error(
            "window_presets.save",
            Some(json!({"name":"Face","grade":target,"node":node,"frame":0})),
        )
        .await;
    assert_eq!(taken["code"], -32001);
    assert_eq!(taken["data"]["reason"], "name_taken");
    // Nothing of the library went into the project.
    assert!(!project(&mut desktop).await.to_string().contains("wp1"));
    drop((desktop, other));

    // Another project of the same user lists it, and recalling it replaces all of a node's windows.
    let second_dir = project_dir();
    let second = engine_with(second_dir.path(), library.path()).await;
    let mut desktop = second.client("desktop").await;
    let (target, node) = graded(
        &mut desktop,
        json!([
            circle("w1", &json!(10)),
            circle("w7", &json!(11)),
            circle("w8", &json!(12))
        ]),
    )
    .await;
    let listed = desktop.result("window_presets.list", None).await;
    assert_eq!(listed["presets"][0]["id"], "wp1");
    let before = project(&mut desktop).await;
    let stale = desktop
        .error(
            "window_presets.apply",
            Some(json!({"preset":"wp1","grade":target,"node":node,"revision":0})),
        )
        .await;
    assert_eq!(stale["code"], -32001);
    let applied = desktop
        .result(
            "window_presets.apply",
            Some(json!({"preset":"wp1","grade":target,"node":node,"revision":before["revision"]})),
        )
        .await;
    assert_eq!(
        applied["revision"],
        before["revision"].as_u64().unwrap() + 1
    );
    let after = project(&mut desktop).await;
    assert_eq!(windows_of(&after), saved["windows"]);
    // Deleting the preset leaves the windows written into the project.
    let deleted = desktop
        .result("window_presets.delete", Some(json!({"preset":"wp1"})))
        .await;
    assert_eq!(deleted, json!({"deleted":"wp1"}));
    assert_eq!(
        desktop.result("window_presets.list", None).await,
        json!({"presets": []})
    );
    assert_eq!(windows_of(&project(&mut desktop).await), saved["windows"]);
    // One undo takes the whole recall back.
    desktop.result("history.undo", None).await;
    assert_eq!(
        windows_of(&project(&mut desktop).await),
        windows_of(&before)
    );
    let gone = desktop
        .error(
            "window_presets.apply",
            Some(json!({"preset":"wp1","grade":target,"node":node})),
        )
        .await;
    assert_eq!(gone["code"], -32602);
    assert_eq!(gone["data"]["reason"], "missing");
}

#[tokio::test]
async fn presets_refuse_a_node_without_windows_another_kind_or_a_locked_one() {
    let library = tempfile::tempdir().unwrap();
    let dir = project_dir();
    let engine = engine_with(dir.path(), library.path()).await;
    let mut desktop = engine.client("desktop").await;
    let (target, node) = graded(&mut desktop, json!([])).await;
    let empty = desktop
        .error(
            "window_presets.save",
            Some(json!({"name":"None","grade":target,"node":node,"frame":0})),
        )
        .await;
    assert_eq!(empty["data"]["reason"], "no_windows");
    let source = desktop
        .error(
            "window_presets.save",
            Some(json!({"name":"Src","grade":target,"node":"source","frame":0})),
        )
        .await;
    assert_eq!(source["code"], -32001);
    assert_eq!(source["data"]["reason"], "missing");
    desktop
        .apply(json!([{"op":"set_node","grade":target,"id":node,"params":{"windows":[circle("w1", &json!(40))]}}]))
        .await;
    let preset = desktop
        .result(
            "window_presets.save",
            Some(json!({"name":"One","grade":target,"node":node,"frame":0})),
        )
        .await;
    // Update Preset overwrites its windows, keeping its name.
    desktop
        .apply(json!([{"op":"set_node","grade":target,"id":node,"params":{"windows":[circle("w1", &json!(60))]}}]))
        .await;
    let updated = desktop
        .result(
            "window_presets.update",
            Some(json!({"preset":preset["id"],"grade":target,"node":node,"frame":0})),
        )
        .await;
    assert_eq!(updated["name"], "One");
    assert_eq!(updated["windows"][0]["transform"]["size"], 60.0);
    desktop
        .apply(json!([{"op":"set_node","grade":target,"id":node,"locked":true}]))
        .await;
    let locked = desktop
        .error(
            "window_presets.apply",
            Some(json!({"preset":preset["id"],"grade":target,"node":node})),
        )
        .await;
    assert_eq!(locked["code"], -32001);
    let unknown = desktop
        .error(
            "window_presets.rename",
            Some(json!({"preset":preset["id"]})),
        )
        .await;
    assert_eq!(unknown["code"], -32601);
}
