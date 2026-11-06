//! Read-only clipboard capture. Cut and paste remain ordinary revision-checked transactions.

use columbina_model::edit::copy_clips;
use serde::Deserialize;
use serde_json::{json, value::RawValue, Value};

use super::{stale_revision, Engine};
use crate::rpc::{self, Fault};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Asked {
    timeline: String,
    revision: u64,
    clips: Vec<String>,
    at: i64,
    #[serde(default)]
    from: Option<i64>,
    #[serde(default)]
    to: Option<i64>,
    #[serde(default = "yes")]
    linked: bool,
    #[serde(default)]
    grouped: bool,
}

fn yes() -> bool {
    true
}

pub(super) async fn copy(engine: &Engine, params: Option<&RawValue>) -> Result<Value, Fault> {
    let asked: Asked = rpc::params(
        "clips.copy",
        r#"{"timeline":"t1","revision":0,"clips":["c1"],"at":25,"from"?:0,"to"?:50,"linked"?:true,"grouped"?:false}"#,
        params,
    )?;
    let range = match (asked.from, asked.to) {
        (None, None) => None,
        (Some(from), Some(to)) => Some((from, to)),
        _ => {
            return Err(Fault::params("give both from and to, or neither")
                .with_data(json!({"reason":"range"})))
        }
    };
    let state = engine.state.lock().await;
    let revision = state.history.revision();
    if asked.revision != revision {
        return Err(stale_revision(asked.revision, revision));
    }
    let timeline = state
        .history
        .project()
        .timelines
        .iter()
        .find(|t| t.id == asked.timeline)
        .ok_or_else(|| {
            Fault::params("timeline is missing")
                .with_data(json!({"reason":"missing","what":"timeline","id":asked.timeline}))
        })?;
    let clipboard = copy_clips(
        timeline,
        &asked.clips,
        asked.at,
        range,
        asked.linked,
        asked.grouped,
    )
    .map_err(|error| {
        Fault::params(error.to_string()).with_data(serde_json::to_value(error).unwrap_or_default())
    })?;
    Ok(json!({"revision": revision, "clipboard": clipboard}))
}
