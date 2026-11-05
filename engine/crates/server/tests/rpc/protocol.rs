//! The protocol as the engine answers it (plan of phase 3, T17), and the streams of sound that it
//! reads of the media of an older project (T3c).

use std::collections::BTreeSet;
use std::time::Duration;

use serde_json::{json, Value};

use super::{engine, project, project_dir, saved, support, WAIT};

/// The methods of the tables of methods of `PROTOCOL.md` (`| method | params | result |`), but
/// a row that names a kind of methods (`transport.*`).
fn methods_of_the_protocol() -> BTreeSet<String> {
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../PROTOCOL.md"))
        .expect("PROTOCOL.md is there");
    let mut methods = BTreeSet::new();
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        let header: Vec<&str> = line.split('|').map(str::trim).collect();
        // `| method | params | result |`: five pieces, the first and the last empty.
        if header.len() != 5 || header[1] != "method" {
            continue;
        }
        while let Some(row) = lines.peek() {
            if !row.trim_start().starts_with('|') {
                break;
            }
            let first = row.split('|').nth(1).unwrap_or("").trim();
            if let Some(name) = first.strip_prefix('`').and_then(|n| n.strip_suffix('`')) {
                if !name.ends_with(".*") {
                    methods.insert(name.to_owned());
                }
            }
            lines.next();
        }
    }
    methods
}

/// The methods that the engine answers `-32601` (no such method) in a desktop's engine, as the
/// protocol says it does.
const NOT_IN_A_DESKTOPS_ENGINE: [&str; 1] = [
    // An engine that is not headless ends when its stdin is closed.
    "engine.yield",
];

#[tokio::test]
async fn every_method_of_the_protocol_is_answered_by_the_engine() {
    let methods = methods_of_the_protocol();
    for method in [
        "hello",
        "transaction.apply",
        "transport.scrub_audio",
        "automation.punch",
        "window_presets.apply",
    ] {
        assert!(methods.contains(method), "{method} in {methods:?}");
    }
    let dir = project_dir();
    let desktops = engine(dir.path(), &["--sound", "null", "--test-hooks"]).await;
    let mut desktop = desktops.client("desktop").await;
    // Asked with what no method takes, so that none is done: each is refused for its params, or
    // for who asks, but none is unknown.
    let mut unknown = Vec::new();
    for method in &methods {
        let error = desktop.error(method, Some(json!({"\u{1}": true}))).await;
        let no_such = error["code"] == -32601;
        if no_such != NOT_IN_A_DESKTOPS_ENGINE.contains(&method.as_str()) {
            unknown.push(format!("{method}: {error}"));
        }
    }
    assert!(unknown.is_empty(), "{}", unknown.join("\n"));
    // A method that the protocol does not list is unknown.
    let error = desktop
        .error("automation.unheard_of", Some(json!({})))
        .await;
    assert_eq!(error["code"], -32601);
    // `engine.yield` is a method of a headless engine, which only the desktop may call.
    let dir = project_dir();
    let headless = engine(dir.path(), &["--headless"]).await;
    let mut agent = headless.client("agent").await;
    let error = agent.error("engine.yield", None).await;
    assert_eq!(error["code"], -32600, "{error}");
}

/// The next `project.changed` told to `client`, within [`WAIT`].
async fn next_change(client: &mut super::Client) -> Value {
    let kept = client.take_notifications();
    if let Some(changed) = kept.into_iter().find(|m| m["method"] == "project.changed") {
        return changed;
    }
    tokio::time::timeout(WAIT, async {
        loop {
            let message = client.message().await;
            if message["method"] == "project.changed" {
                return message;
            }
        }
    })
    .await
    .expect("a change is told")
}

#[tokio::test]
async fn the_streams_of_media_of_an_older_project_are_read_once_its_file_is_there() {
    let Some(made) = support::media(support::Sample::TwoMonoStreams) else {
        return;
    };
    let dir = project_dir();
    let source = dir.path().join("source.mov");
    std::fs::copy(made.path, &source).unwrap();
    // The facts of the sound of the file, as an import reads them now.
    let read = {
        let engine = engine(dir.path(), &["--headless", "--idle-seconds", "1"]).await;
        let mut agent = engine.client("agent").await;
        agent
            .result("media.import", Some(json!({"paths":[source]})))
            .await;
        let audio = project(&mut agent).await["project"]["media"][0]["facts"]["audio"].clone();
        drop(agent);
        assert_eq!(engine.ended().await, 0);
        audio
    };
    assert_eq!(read["streams"].as_array().map(Vec::len), Some(2), "{read}");
    // The project as an engine before schema 4 read streams wrote it, its file away.
    let mut written = saved(dir.path());
    let audio = written["media"][0]["facts"]["audio"]
        .as_object_mut()
        .unwrap();
    audio.remove("layout");
    audio.remove("streams");
    std::fs::write(
        dir.path().join("project.json"),
        serde_json::to_string_pretty(&written).unwrap(),
    )
    .unwrap();
    let away = source.with_extension("away");
    std::fs::rename(&source, &away).unwrap();

    let engine = engine(dir.path(), &["--headless"]).await;
    let mut agent = engine.client("agent").await;
    let before = project(&mut agent).await;
    let audio = &before["project"]["media"][0]["facts"]["audio"];
    assert!(audio.get("streams").is_none(), "{audio}");
    // While the file is away, nothing is read.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let still = project(&mut agent).await;
    assert_eq!(still["revision"], before["revision"]);
    assert!(agent
        .take_notifications()
        .iter()
        .all(|m| m["method"] != "project.changed"));

    // Back, it is read within the time that the engine tries again.
    std::fs::rename(&away, &source).unwrap();
    let changed = next_change(&mut agent).await;
    let params = &changed["params"];
    assert_eq!(
        (&params["by"], &params["made_by"]),
        (&json!("engine"), &json!("engine"))
    );
    assert_eq!(params["revision"], before["revision"].as_u64().unwrap() + 1);
    assert_eq!(
        params["done"],
        json!([{
            "op": "set_media_streams", "id": "m1",
            "layout": read.get("layout").cloned().unwrap_or(Value::Null),
            "streams": read["streams"],
        }])
    );
    let after = project(&mut agent).await;
    assert_eq!(after["project"]["media"][0]["facts"]["audio"], read);
    // It is no step of the history: the import was made by another engine, so nothing is undone.
    let undone = agent.result("history.undo", None).await;
    assert_eq!(undone["done"], Value::Null, "{undone}");
    // And it is saved as every change is.
    let deadline = std::time::Instant::now() + WAIT;
    loop {
        if saved(dir.path())["media"][0]["facts"]["audio"] == read {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the streams are saved"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
