//! Add Serial Node + CPW, LPW, PPW and PCW (Option-C, -Q, -G and -B; RM20 p. 3187) through the
//! real engine: `add_serial` with `windows`, after the default node that the same transaction
//! makes, is one step to undo and one to redo.

use serde_json::{json, Value};

use super::{clip, engine, project, project_dir};

fn nodes(project: &Value) -> Value {
    project["project"]["timelines"][0]["tracks"][0]["clips"][0]["grade"]["nodes"].clone()
}

#[tokio::test]
async fn a_node_added_with_a_window_after_the_default_node_is_one_undo_step() {
    let dir = project_dir();
    let engine = engine(dir.path(), &[]).await;
    let mut desktop = engine.client("desktop").await;
    let mut color = clip("$clip", "unused", 0, 100);
    color["source"] = json!({"kind":"color","color":"#dc5028ff"});
    let made = desktop
        .apply(json!([{"op":"add_timeline","timeline":{
            "id":"$timeline","name":"Windows","width":128,"height":72,
            "rate":{"num":25,"den":1},"sample_rate":48000,
            "tracks":[{"id":"$track","name":"Picture","kind":"video","enabled":true,"locked":false,
                "clips":[color],"transitions":[]}]
        }}]))
        .await;
    let target =
        json!({"level":"clip","timeline":made["names"]["$timeline"],"clip":made["names"]["$clip"]});
    let before = project(&mut desktop).await;
    assert_eq!(nodes(&before), json!([]));
    // The desktop's default node 01 is `n<next_id>`, made first in the same transaction.
    let first = format!("n{}", before["project"]["next_id"]);
    let circle = json!({"id":"w1","label":"","enabled":true,"invert":false,"combine":"add",
        "opacity":100.0,"transform":{"size":50.0,"aspect":50.0,"pan":50.0,"tilt":50.0,
        "rotate":0.0},"shape":{"kind":"circular","softness":1.0}});
    let applied = desktop
        .result(
            "transaction.apply",
            Some(json!({"revision":before["revision"],"ops":[
                {"edit":"add_serial","grade":target,"node":"source"},
                {"edit":"add_serial","grade":target,"node":first,"windows":[circle]},
            ]})),
        )
        .await;
    let done = applied["done"].as_array().unwrap();
    assert_eq!(done.len(), 2, "{applied}");
    // The corrector is added with its window, which the client reads from what was done.
    assert_eq!(done[1]["op"], "add_node");
    assert_eq!(done[1]["node"]["params"]["windows"], json!([circle]));
    let after = project(&mut desktop).await;
    let added = nodes(&after);
    assert_eq!(added.as_array().unwrap().len(), 2);
    assert_eq!(added[0]["id"], json!(first));
    assert_eq!(
        added[1]["params"]["windows"][0]["shape"]["kind"],
        "circular"
    );
    assert_eq!(added[0]["params"]["windows"], json!([]));

    desktop.result("history.undo", None).await;
    let undone = project(&mut desktop).await;
    assert_eq!(nodes(&undone), json!([]));
    assert_eq!(
        undone["project"]["timelines"][0]["tracks"][0]["clips"][0]["grade"],
        before["project"]["timelines"][0]["tracks"][0]["clips"][0]["grade"]
    );
    desktop.result("history.redo", None).await;
    assert_eq!(nodes(&project(&mut desktop).await), added);

    // An invalid window refuses the whole transaction.
    let mut wrong = circle.clone();
    wrong["transform"]["size"] = json!(-1.0);
    let current = project(&mut desktop).await;
    let refused = desktop
        .error(
            "transaction.apply",
            Some(json!({"revision":current["revision"],"ops":[
                {"edit":"add_serial","grade":target,"node":first,"windows":[wrong]},
            ]})),
        )
        .await;
    assert_eq!(refused["code"], -32001, "{refused}");
    assert_eq!(project(&mut desktop).await["revision"], current["revision"]);
}
