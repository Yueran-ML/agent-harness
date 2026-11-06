//! Readers of the currently playing graph. No object here is accessed by the real-time thread.
use columbina_audio::graph::{FxGraph, FxOwner, FxReader, Graph, Owner};
use serde_json::{Value, json};
use std::sync::{Mutex, OnceLock};
fn graphs() -> &'static Mutex<Vec<(String, FxGraph)>> {
    static GRAPHS: OnceLock<Mutex<Vec<(String, FxGraph)>>> = OnceLock::new();
    GRAPHS.get_or_init(|| Mutex::new(Vec::new()))
}
pub(crate) fn offer(timeline: &str, graph: &Graph, restart: bool) {
    let mut graphs = graphs()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if restart || graphs.last().is_some_and(|(id, _)| id != timeline) {
        graphs.clear();
    }
    // Keep the graph playing and the bounded queue of graphs waiting to replace it.
    if graphs.len() >= 8 {
        if let Some(at) = graphs.iter().position(|(_, g)| !g.state.read().0) {
            graphs.remove(at);
        }
    }
    graphs.push((timeline.to_owned(), graph.fx_readings()));
}
pub(crate) fn shown(timeline: &str) -> Value {
    let graphs = graphs()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let current = graphs.iter().rev().find(|(id, graph)| {
        let state = graph.state.read();
        id == timeline && state.0 && state.1 > 0
    });
    let Some((_, graph)) = current else {
        return json!({"timeline":timeline,"playing":false,"generation":null,"sample_rate":null,"readings":[]});
    };
    let readings: Vec<_> = graph.taps.iter().map(|tap| {
        let owner = match &tap.owner { FxOwner::Clip(id) => json!({"clip":id}), FxOwner::Strip(Owner::Track(id)) => json!({"track":id}), FxOwner::Strip(Owner::Bus(id)) => json!({"bus":id}), FxOwner::Strip(Owner::Vca(id)) => json!({"vca":id}) };
        let reading = match &tap.reader {
            FxReader::Dynamics(reader) => { let r=reader.read(); json!({"kind":"dynamics","input_db":r.input_db,"output_db":r.output_db,"expander_db":r.expander_db,"compressor_db":r.compressor_db,"limiter_db":r.limiter_db}) },
            FxReader::Ducker(reader) => { let r=reader.read(); json!({"kind":"ducker","input_db":r.0,"source_db":r.1,"reduction_db":r.2}) },
            FxReader::Limiter(reader) => { let r=reader.read(); json!({"kind":"limiter","input_db":r.0,"reduction_db":r.1,"output_db":r.2}) },
            FxReader::DeEsser(reader) => { let r=reader.read(); json!({"kind":"de_esser","reduction_db":r.0,"output_db":r.1}) },
            FxReader::Noise(reader) => { let r=reader.read(); json!({"kind":"noise_reduction","input_spectrum_db":&r[..128],"noise_spectrum_db":&r[128..256],"input_db":r[256],"output_db":r[257]}) },
            FxReader::Width(reader) => { let r=reader.read(); json!({"kind":"stereo_width","scope":&r[..128],"input_db":r[128],"output_db":r[129]}) },
        };
        json!({"owner":owner,"insert":tap.insert,"slot":tap.slot,"reading":reading})
    }).collect();
    json!({"timeline":timeline,"playing":true,"generation":graph.generation,"sample_rate":graph.rate,"sequence":graph.state.read().1,"readings":readings})
}

#[cfg(test)]
mod tests {
    use super::*;
    use columbina_audio::graph::{Options, Purpose};
    use columbina_model::audio::fx::{LimiterFx, NoiseReductionFx, StereoWidthFx};
    use columbina_model::{Effect, Insert, Project, TrackKind};
    #[test]
    fn reads_only_current_playing_graph_and_preserves_insert_identity() {
        let mut p = Project::sample();
        let t = &mut p.timelines[0];
        let id = t.id.clone();
        let a = t
            .tracks
            .iter_mut()
            .find(|t| t.kind == TrackKind::Audio)
            .unwrap();
        a.audio.strip.inserts = vec![
            Insert {
                id: "nr".into(),
                on: true,
                effect: Effect::NoiseReduction(NoiseReductionFx::default()),
            },
            Insert {
                id: "wide".into(),
                on: true,
                effect: Effect::StereoWidth(StereoWidthFx::default()),
            },
            Insert {
                id: "limit".into(),
                on: true,
                effect: Effect::Limiter(LimiterFx::default()),
            },
        ];
        a.audio.strip.dynamics.on = true;
        let mut graph = Graph::build(&p, 0, Options::new(4096, Purpose::Preview)).unwrap();
        offer(&id, &graph, true);
        assert_eq!(shown(&id)["playing"], false);
        let mut feed = |_: usize,
                        _: std::ops::Range<i64>,
                        out: &mut [&mut [f32]]|
         -> Result<(), columbina_audio::feed::FeedError> {
            for c in out {
                c.fill(0.2);
            }
            Ok(())
        };
        graph.start(0, &mut feed).unwrap();
        graph.set_metering(true);
        graph.process(&mut feed, 4096).unwrap();
        let result = shown(&id);
        assert_eq!(result["playing"], true);
        let readings = result["readings"].as_array().unwrap();
        for insert in ["nr", "wide", "limit"] {
            assert!(readings.iter().any(|r| r["insert"] == insert));
        }
        assert!(readings.iter().any(|r| r["reading"]["kind"] == "dynamics"));
        graph.set_metering(false);
        assert_eq!(shown(&id)["playing"], false);
        assert_eq!(shown(&id)["readings"], json!([]));
        graph.set_metering(true);
        assert_eq!(shown(&id)["playing"], false);
        graph.set_metering(false);
        let next = Graph::build(&p, 0, Options::new(4096, Purpose::Preview)).unwrap();
        offer(&id, &next, false);
        assert_eq!(shown(&id)["playing"], false);
        assert_eq!(shown("other")["readings"], json!([]));
    }
}
