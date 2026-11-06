//! Read-only static processor curves; deliberately separate from playback telemetry.

use columbina_audio::graph::{dynamics_curve, eq_response};
use columbina_model::{Project, Strip, Timeline, TrackKind};
use serde::Deserialize;
use serde_json::{json, value::RawValue, Value};

use super::{Connection, Engine, Reply};
use crate::rpc::{self, Fault};

/// A curve is drawn in a few hundred pixels. Bound repeated control reads and result allocation.
const MAX_POINTS: usize = 512;

#[derive(Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum Target {
    Track(String),
    Bus(String),
    Clip(String),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EqRequest {
    timeline: String,
    target: Target,
    frequencies: Vec<f32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DynamicsRequest {
    timeline: String,
    target: Target,
    inputs_db: Vec<f32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SoftClipRequest {
    timeline: String,
    target: Target,
    slot: usize,
    inputs: Vec<f32>,
}

fn missing(what: &str, id: &str) -> Fault {
    Fault::params(format!(
        "there is no {what} {id} of sound in the requested timeline"
    ))
    .with_data(json!({"reason":"missing", "what":what, "id":id}))
}

fn timeline_of<'a>(project: &'a Project, id: &str) -> Result<&'a Timeline, Fault> {
    project
        .timelines
        .iter()
        .find(|t| t.id == id)
        .ok_or_else(|| missing("timeline", id))
}

fn strip_of<'a>(timeline: &'a Timeline, target: &Target) -> Result<&'a Strip, Fault> {
    match target {
        Target::Track(id) => timeline
            .tracks
            .iter()
            .find(|t| t.id == *id && t.kind == TrackKind::Audio)
            .map(|t| &t.audio.strip)
            .ok_or_else(|| missing("track", id)),
        Target::Bus(id) => timeline
            .buses
            .iter()
            .find(|b| b.id == *id)
            .map(|b| &b.strip)
            .ok_or_else(|| missing("bus", id)),
        Target::Clip(_) => Err(Fault::params("Dynamics belongs to an audio track or bus")
            .with_data(json!({"reason":"wrong_kind", "what":"target"}))),
    }
}

fn checked_points(points: &[f32], low: f32, high: f32, what: &str) -> Result<(), Fault> {
    if points.is_empty()
        || points.len() > MAX_POINTS
        || points
            .iter()
            .any(|p| !p.is_finite() || *p < low || *p > high)
    {
        return Err(Fault::params(format!(
            "{what} must contain 1..={MAX_POINTS} finite values in {low}..={high}"
        ))
        .with_data(json!({"reason":"out_of_range", "what":what})));
    }
    Ok(())
}

/// Only borrows the snapshot: errors and successful reads cannot create a history step.
#[allow(
    clippy::cast_precision_loss,
    reason = "supported sample rates are small integers"
)]
fn compute(
    project: &Project,
    revision: u64,
    method: &str,
    params: Option<&RawValue>,
) -> Result<Value, Fault> {
    match method {
        "audio.soft_clip_curve" => {
            let request: SoftClipRequest = rpc::params(
                method,
                r#"{"timeline":"t1","target":{"track":"A1"},"slot":0,"inputs":[-1,0,1]}"#,
                params,
            )?;
            let t = timeline_of(project, &request.timeline)?;
            checked_points(&request.inputs, -2.0, 2.0, "inputs")?;
            let inserts = match &request.target {
                Target::Clip(id) => {
                    &t.tracks
                        .iter()
                        .filter(|t| t.kind == TrackKind::Audio)
                        .flat_map(|t| &t.clips)
                        .find(|c| c.id == *id)
                        .ok_or_else(|| missing("clip", id))?
                        .effects
                }
                _ => &strip_of(t, &request.target)?.inserts,
            };
            let insert = inserts
                .get(request.slot)
                .ok_or_else(|| Fault::params("missing insert"))?;
            let columbina_model::audio::fx::Effect::SoftClipper(fx) = &insert.effect else {
                return Err(Fault::params("the insert is not Soft Clipper"));
            };
            let outputs: Vec<f64> = request
                .inputs
                .iter()
                .map(|x| {
                    if insert.on {
                        columbina_audio::fx::soft_clip::curve(
                            f64::from(*x) * 10.0_f64.powf(fx.threshold_db / 20.0),
                            fx.shape / 100.0,
                        ) * 10.0_f64.powf(fx.output_db / 20.0)
                    } else {
                        f64::from(*x)
                    }
                })
                .collect();
            Ok(json!({"revision":revision,"inputs":request.inputs,"outputs":outputs}))
        }
        "audio.eq_response" => {
            let request: EqRequest = rpc::params(
                method,
                r#"{"timeline":"t1","target":{"track":"A1"},"frequencies":[20,1000,20000]}"#,
                params,
            )?;
            let timeline = timeline_of(project, &request.timeline)?;
            checked_points(
                &request.frequencies,
                20.0,
                (timeline.sample_rate as f32 * 0.45).min(20000.0),
                "frequencies",
            )?;
            let eq = if let Target::Clip(id) = &request.target {
                &timeline
                    .tracks
                    .iter()
                    .filter(|t| t.kind == TrackKind::Audio)
                    .flat_map(|t| &t.clips)
                    .find(|c| c.id == *id)
                    .ok_or_else(|| missing("clip", id))?
                    .eq
            } else {
                &strip_of(timeline, &request.target)?.eq
            };
            let gain_db = eq_response(eq, timeline.sample_rate, &request.frequencies);
            Ok(
                json!({"revision": revision, "sample_rate":timeline.sample_rate, "frequencies":request.frequencies, "gain_db":gain_db}),
            )
        }
        "audio.dynamics_curve" => {
            let request: DynamicsRequest = rpc::params(
                method,
                r#"{"timeline":"t1","target":{"bus":"B1"},"inputs_db":[-60,-20,0]}"#,
                params,
            )?;
            let timeline = timeline_of(project, &request.timeline)?;
            checked_points(&request.inputs_db, -60.0, 0.0, "inputs_db")?;
            let outputs_db = dynamics_curve(
                &strip_of(timeline, &request.target)?.dynamics,
                &request.inputs_db,
            );
            Ok(json!({"revision":revision, "inputs_db":request.inputs_db, "outputs_db":outputs_db}))
        }
        _ => Err(Fault::new(
            rpc::METHOD_NOT_FOUND,
            format!("no method {method}"),
        )),
    }
}

pub(super) async fn call(
    engine: &Engine,
    connection: &Connection,
    id: &RawValue,
    method: &str,
    params: Option<&RawValue>,
) -> Result<Reply, Reply> {
    let state = engine.state.lock().await;
    let result = compute(
        state.history.project(),
        state.history.revision(),
        method,
        params,
    )?;
    Ok(match connection.queue(rpc::result_line(id, &result)) {
        Ok(()) => Reply::Queued,
        Err(refused) => Reply::Refused(refused),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use columbina_model::{History, Project};
    use serde_json::{json, Value};

    fn project() -> Project {
        let mut project = Project::sample();
        project.timelines[0].tracks.push(
            serde_json::from_value(json!({
                "id": "curve-audio", "name": "Voice", "kind": "audio", "enabled": true,
                "locked": false, "clips": [], "transitions": []
            }))
            .unwrap(),
        );
        project
    }
    fn request(project: &Project, method: &str, params: Value) -> Result<Value, crate::rpc::Fault> {
        let raw = serde_json::value::to_raw_value(&params).unwrap();
        compute(project, 37, method, Some(&raw))
    }
    #[test]
    fn curves_read_the_requested_owner_and_revision_without_history_writes() {
        let history = History::new(project());
        let before = history.project().clone();
        let result = request(history.project(), "audio.eq_response", json!({"timeline":"t1", "target":{"track":"curve-audio"}, "frequencies":[20,1000,20000]})).unwrap();
        assert_eq!(result["revision"], 37);
        assert_eq!(result["sample_rate"], before.timelines[0].sample_rate);
        assert_eq!(result["gain_db"], json!([0.0, 0.0, 0.0]));
        let result = request(
            history.project(),
            "audio.dynamics_curve",
            json!({"timeline":"t1", "target":{"bus":"B1"}, "inputs_db":[-60,-20,0]}),
        )
        .unwrap();
        assert_eq!(result["outputs_db"], json!([-60.0, -20.0, 0.0]));
        assert_eq!(history.project(), &before);
        assert_eq!(history.revision(), 0);
    }
    #[test]
    fn invalid_owners_points_and_fields_are_refused() {
        let project = project();
        for target in [
            json!({"track":"V1"}),
            json!({"track":"missing"}),
            json!({"bus":"missing"}),
            json!({"clip":"missing"}),
            json!({"track":"curve-audio","bus":"B1"}),
        ] {
            assert!(request(
                &project,
                "audio.eq_response",
                json!({"timeline":"t1", "target":target, "frequencies":[1000]})
            )
            .is_err());
        }
        for frequencies in [
            json!([]),
            json!([19]),
            json!([20001]),
            json!([null]),
            json!(vec![1000; 513]),
        ] {
            assert!(request(&project, "audio.eq_response", json!({"timeline":"t1", "target":{"track":"curve-audio"}, "frequencies":frequencies})).is_err());
        }
        for inputs in [
            json!([]),
            json!([-61]),
            json!([1]),
            json!([null]),
            json!(vec![0; 513]),
        ] {
            assert!(request(
                &project,
                "audio.dynamics_curve",
                json!({"timeline":"t1", "target":{"track":"curve-audio"}, "inputs_db":inputs})
            )
            .is_err());
        }
        assert!(request(
            &project,
            "audio.dynamics_curve",
            json!({"timeline":"t1", "target":{"clip":"c1"}, "inputs_db":[0]})
        )
        .is_err());
        assert!(request(
            &project,
            "audio.eq_response",
            json!({"timeline":"t1", "target":{"track":"curve-audio"}, "frequencies":[1000],"at":0})
        )
        .is_err());
        assert!(request(
            &project,
            "audio.eq_response",
            json!({"timeline":"missing", "target":{"track":"curve-audio"}, "frequencies":[1000]})
        )
        .is_err());
    }
    #[test]
    fn cap_and_non_finite_points_are_checked_and_clip_has_four_bands() {
        assert!(checked_points(&[f32::NAN], 20.0, 20000.0, "frequencies").is_err());
        assert!(checked_points(&[f32::INFINITY], -60.0, 0.0, "inputs_db").is_err());
        let mut project = project();
        project.timelines[0].sample_rate = 32000;
        assert!(request(
            &project,
            "audio.eq_response",
            json!({"timeline":"t1", "target":{"track":"curve-audio"}, "frequencies":[16000]})
        )
        .is_err());
        let clip = project.timelines[0].tracks[0].clips[0].clone();
        project.timelines[0]
            .tracks
            .last_mut()
            .unwrap()
            .clips
            .push(clip);
        let id = &project.timelines[0].tracks.last().unwrap().clips[0].id;
        assert!(request(
            &project,
            "audio.eq_response",
            json!({"timeline":"t1", "target":{"clip":id}, "frequencies":[20,1000,14400]})
        )
        .is_ok());
    }
}
