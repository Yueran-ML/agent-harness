//! Offline prefix render: the measured signal is the input of the selected NR insert.
use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use columbina_audio::analysis::{alone, run_strips};
use columbina_audio::graph::{Graph, Options, Owner, Point, Purpose};
use columbina_model::audio::fx::{Effect, NoiseMode};
use columbina_model::{AudioParam, Insert, Lanes, Project, Stage, Strip, Timeline, TrackKind};
use serde::Deserialize;
use serde_json::{json, value::RawValue, Value};

use super::{
    clip_of, frames, progress, refused, sample_at, timeline_of, Frames, Kind, Target, Teller,
    MAX_BLOCK,
};
use crate::play::mix::Feed;
use crate::rpc::{self, Fault};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    timeline: String,
    target: Target,
    slot: usize,
    range: Frames,
    revision: u64,
}
pub(super) struct Work {
    timeline: usize,
    target: Target,
    slot: usize,
    range: Range<i64>,
    pub revision: u64,
}

fn inserts<'a>(t: &'a Timeline, target: &Target) -> Option<&'a [Insert]> {
    match target {
        Target::Sum(_) => None, // A summed analysis target has no insert chain.
        Target::Clip(id) => t
            .tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Audio)
            .flat_map(|t| &t.clips)
            .find(|c| c.id == *id)
            .map(|c| c.effects.as_slice()),
        Target::Track(id) => t
            .tracks
            .iter()
            .find(|t| t.kind == TrackKind::Audio && t.id == *id && t.enabled)
            .map(|t| t.audio.strip.inserts.as_slice()),
        Target::Bus(id) => t
            .buses
            .iter()
            .find(|b| b.id == *id)
            .map(|b| b.strip.inserts.as_slice()),
    }
}
pub(super) fn prepare(
    method: &str,
    params: Option<&RawValue>,
    project: &Project,
) -> Result<Work, Fault> {
    let r: Request = rpc::params(
        method,
        r#"{"timeline":"t1","target":{"track":"A1"},"slot":0,"range":{"from":0,"to":25},"revision":0}"#,
        params,
    )?;
    let (timeline, t) = timeline_of(project, &r.timeline)?;
    let found = inserts(t, &r.target).and_then(|chain| chain.get(r.slot));
    if !matches!(found.map(|i| &i.effect), Some(Effect::NoiseReduction(_))) {
        return Err(refused(
            "the selected insert is not Noise Reduction",
            json!({"reason":"missing_insert"}),
        ));
    }
    if r.range.from < 0 || r.range.to <= r.range.from {
        return Err(refused(
            "invalid learn range",
            json!({"reason":"out_of_range"}),
        ));
    }
    let from = sample_at(t, r.range.from);
    let to = sample_at(t, r.range.to);
    // A noise print needs one FFT window; thirty seconds bounds offline storage for multichannel audio.
    if r.range.from < 0
        || to - from < 2048
        || to - from > i64::from(t.sample_rate) * 30
        || to > crate::play::mix::end_of(t)
    {
        return Err(refused(
            "learn a range of at least 2048 samples and at most 30 seconds within the timeline",
            json!({"reason":"out_of_range","what":"range"}),
        ));
    }
    if let Target::Clip(id) = &r.target {
        let (_, c) = clip_of(t, id)?;
        if from < sample_at(t, c.start) || to > sample_at(t, c.end()) {
            return Err(refused(
                "the learn range must be within the clip",
                json!({"reason":"out_of_range"}),
            ));
        }
    }
    Ok(Work {
        timeline,
        target: r.target,
        slot: r.slot,
        range: from..to,
        revision: r.revision,
    })
}

fn truncate(strip: &mut Strip, lanes: &mut Lanes, slot: usize) {
    strip.inserts.truncate(slot);
    let after_fx = strip
        .order
        .iter()
        .skip_while(|s| **s != Stage::Fx)
        .skip(1)
        .copied()
        .collect::<Vec<_>>();
    if after_fx.contains(&Stage::Eq) {
        strip.eq.on = false;
        lanes.retain(|p, _| !p.starts_with("eq."));
    }
    if after_fx.contains(&Stage::Dynamics) {
        strip.dynamics.on = false;
        lanes.retain(|p, _| !p.starts_with("dynamics."));
    }
    lanes.retain(|p, _| {
        !p.starts_with("insert.")
            || strip
                .inserts
                .iter()
                .any(|i| p.starts_with(&format!("insert.{}.", i.id)))
    });
}

fn prefix(project: &Project, work: &Work) -> Result<(Project, Owner), String> {
    match &work.target {
        Target::Sum(_) => Err("a sum has no Noise Reduction insert".into()),
        Target::Clip(id) => {
            let (mut p, track, _) = alone(project, work.timeline, id).ok_or("missing clip")?;
            let c = p.timelines[work.timeline]
                .tracks
                .iter_mut()
                .find(|t| t.id == track)
                .unwrap()
                .clips
                .first_mut()
                .unwrap();
            c.effects.truncate(work.slot);
            c.gain_db = AudioParam::Fixed(0.0);
            Ok((p, Owner::Track(track)))
        }
        Target::Track(id) => {
            let mut p = project.clone();
            let t = p.timelines[work.timeline]
                .tracks
                .iter_mut()
                .find(|t| t.id == *id)
                .ok_or("missing track")?;
            truncate(&mut t.audio.strip, &mut t.audio.lanes, work.slot);
            Ok((p, Owner::Track(id.clone())))
        }
        Target::Bus(id) => {
            let mut p = project.clone();
            let b = p.timelines[work.timeline]
                .buses
                .iter_mut()
                .find(|b| b.id == *id)
                .ok_or("missing bus")?;
            truncate(&mut b.strip, &mut b.lanes, work.slot);
            Ok((p, Owner::Bus(id.clone())))
        }
    }
}

pub(super) fn run(
    teller: &Teller,
    kind: Kind,
    original: &Arc<Project>,
    work: &Work,
    stop: &AtomicBool,
) -> Result<Option<Value>, String> {
    if stop.load(Ordering::Acquire) {
        return Ok(None);
    }
    let (project, owner) = prefix(original, work)?;
    let mut graph = Graph::build(
        &project,
        work.timeline,
        Options::new(MAX_BLOCK, Purpose::Analysis),
    )
    .map_err(|e| e.to_string())?;
    let strip = graph
        .strip_of(&owner)
        .ok_or("the insert's strip is not in the graph")?;
    let project = Arc::new(project);
    let mut feed = Feed::new(Arc::clone(&project), work.timeline, graph.clips(), false);
    let mut progress = progress(
        teller,
        stop,
        project.timelines[work.timeline].sample_rate,
        frames(&work.range),
    );
    let mut pcm: Vec<Vec<f32>> = Vec::new();
    let samples = usize::try_from(work.range.end - work.range.start)
        .map_err(|_| "the learn range does not fit in memory")?;
    let completed = run_strips(
        &mut graph,
        &mut feed,
        &[(strip, Point::BeforeFader)],
        work.range.clone(),
        &mut |_, channels| {
            if pcm.is_empty() {
                pcm = channels
                    .iter()
                    .map(|_| Vec::with_capacity(samples))
                    .collect();
            }
            for (dst, src) in pcm.iter_mut().zip(channels) {
                dst.extend_from_slice(src);
            }
            progress.more(channels.first().map_or(0, |c| c.len()))
        },
    )
    .map_err(|e| e.to_string())?;
    if !completed || stop.load(Ordering::Acquire) {
        return Ok(None);
    }
    let borrowed: Vec<&[f32]> = pcm.iter().map(Vec::as_slice).collect();
    let profile = columbina_audio::fx::denoise::learn(&borrowed)
        .ok_or("the selected range has no complete FFT window")?;
    if stop.load(Ordering::Acquire) {
        return Ok(None);
    }
    let t = &original.timelines[work.timeline];
    let mut chain = inserts(t, &work.target).ok_or("missing target")?.to_vec();
    let Effect::NoiseReduction(nr) = &mut chain[work.slot].effect else {
        return Err("missing Noise Reduction".into());
    };
    let profile: Vec<f64> = profile.into_iter().map(f64::from).collect();
    nr.profile = Some(profile.clone());
    nr.mode = NoiseMode::Manual;
    let edit = match &work.target {
        Target::Sum(_) => return Err("a sum has no Noise Reduction insert".into()),
        Target::Clip(id) => json!({"op":"set_clip","timeline":t.id,"id":id,"effects":chain}),
        Target::Track(id) => {
            json!({"edit":"set_track_audio","timeline":t.id,"track":id,"follow_groups":false,"patch":{"strip":{"inserts":chain}}})
        }
        Target::Bus(id) => {
            json!({"edit":"set_bus","timeline":t.id,"id":id,"patch":{"strip":{"inserts":chain}}})
        }
    };
    let applied = teller.apply(kind, Some(work.revision), vec![edit]);
    Ok(Some(
        json!({"state":"done","method":"audio.noise_learn","target":work.target.json(),"slot":work.slot,"profile":profile,"result":applied}),
    ))
}

#[cfg(test)]
mod tests {
    use super::super::check_revision;
    use super::*;
    use columbina_model::audio::fx::{GainFx, NoiseReductionFx};
    fn chain() -> Vec<Insert> {
        vec![
            Insert {
                id: "before".into(),
                on: true,
                effect: Effect::Gain(GainFx { gain_db: 6.0 }),
            },
            Insert {
                id: "learn".into(),
                on: true,
                effect: Effect::NoiseReduction(NoiseReductionFx::default()),
            },
            Insert {
                id: "after".into(),
                on: true,
                effect: Effect::Gain(GainFx { gain_db: -20.0 }),
            },
        ]
    }
    fn settings() -> (Project, Work) {
        let mut p = Project::sample();
        let t = p.timelines[0]
            .tracks
            .iter_mut()
            .find(|t| t.kind == TrackKind::Audio)
            .unwrap();
        t.audio.strip.inserts = chain();
        t.audio.strip.fader_db = -40.0;
        let id = t.id.clone();
        (
            p,
            Work {
                timeline: 0,
                target: Target::Track(id),
                slot: 1,
                range: 4800..9600,
                revision: 4,
            },
        )
    }
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_wrap,
        reason = "the fixture renders samples 4800 to 9600, exact in f32 and within i64"
    )]
    fn measured(p: &Project, w: &Work) -> Vec<f32> {
        let (p, owner) = prefix(p, w).unwrap();
        let mut graph = Graph::build(&p, 0, Options::new(1024, Purpose::Analysis)).unwrap();
        let strip = graph.strip_of(&owner).unwrap();
        let mut feed = |_: usize,
                        range: Range<i64>,
                        out: &mut [&mut [f32]]|
         -> Result<(), columbina_audio::feed::FeedError> {
            for channel in out {
                for (n, x) in channel.iter_mut().enumerate() {
                    *x = ((range.start + n as i64) as f32 * 0.071).sin() * 0.05;
                }
            }
            Ok(())
        };
        let mut pcm = vec![];
        run_strips(
            &mut graph,
            &mut feed,
            &[(strip, Point::BeforeFader)],
            w.range.clone(),
            &mut |_, channels| {
                pcm.extend_from_slice(channels[0]);
                true
            },
        )
        .unwrap();
        columbina_audio::fx::denoise::learn(&[&pcm]).unwrap()
    }
    #[test]
    fn summed_analysis_targets_cannot_be_learned() {
        let p = Project::sample();
        let raw = RawValue::from_string(json!({"timeline":"t1","target":{"sum":["a1"]},"slot":0,"range":{"from":0,"to":25},"revision":0}).to_string()).unwrap();
        assert_eq!(
            prepare("audio.noise_learn", Some(&raw), &p)
                .err()
                .unwrap()
                .data
                .unwrap()["reason"],
            "missing_insert"
        );
    }

    #[test]
    fn track_learn_hears_prefix_not_nr_output_or_fader() {
        let (mut p, w) = settings();
        let before = p.clone();
        let learned = measured(&p, &w);
        assert_eq!(p, before);
        let t = p.timelines[0]
            .tracks
            .iter_mut()
            .find(|t| t.kind == TrackKind::Audio)
            .unwrap();
        t.audio.strip.inserts[0].effect = Effect::Gain(GainFx { gain_db: 0.0 });
        t.audio.strip.fader_db = 10.0;
        t.audio.strip.inserts[2].effect = Effect::Gain(GainFx { gain_db: 24.0 });
        let raw = measured(&p, &w);
        let ratio = learned.iter().sum::<f32>() / raw.iter().sum::<f32>();
        assert!((ratio - 10.0_f32.powf(6.0 / 10.0)).abs() < 0.001, "{ratio}");
    }
    #[test]
    fn clip_and_bus_prefix_keep_prior_fx_and_exclude_later_stages() {
        let (mut project, mut work) = settings();
        let track = project.timelines[0]
            .tracks
            .iter_mut()
            .find(|t| t.kind == TrackKind::Audio)
            .unwrap();
        track.clips[0].effects = chain();
        track.clips[0].gain_db = AudioParam::Fixed(-30.0);
        work.target = Target::Clip(track.clips[0].id.clone());
        let (clip, _) = prefix(&project, &work).unwrap();
        let c = clip.timelines[0]
            .tracks
            .iter()
            .find(|t| t.kind == TrackKind::Audio)
            .unwrap();
        assert_eq!(c.clips[0].effects.len(), 1);
        assert_eq!(c.clips[0].gain_db, AudioParam::Fixed(0.0));
        assert_eq!(c.audio.strip.inserts, [] as [Insert; 0]);
        let b = &mut project.timelines[0].buses[0];
        b.strip.inserts = chain();
        b.strip.order = vec![Stage::Eq, Stage::Fx, Stage::Dynamics];
        b.strip.eq.on = true;
        b.strip.dynamics.on = true;
        work.target = Target::Bus(b.id.clone());
        let (bus, _) = prefix(&project, &work).unwrap();
        let b = &bus.timelines[0].buses[0];
        assert!(b.strip.eq.on);
        assert!(!b.strip.dynamics.on);
        assert_eq!(b.strip.inserts.len(), 1);
        assert!(measured(&project, &work).iter().any(|x| *x > 0.0));
    }
    #[test]
    fn learn_requires_current_revision_valid_nr_and_bounded_range() {
        let (p, w) = settings();
        let t = &p.timelines[0];
        let request = json!({"timeline":t.id,"target":w.target.json(),"slot":1,"range":{"from":0,"to":25},"revision":4});
        let raw = RawValue::from_string(request.to_string()).unwrap();
        let work = super::super::prepare("audio.noise_learn", Some(&raw), &p).unwrap();
        check_revision(&work, 4).unwrap();
        assert!(check_revision(&work, 5).is_err());
        for field in [
            json!({"slot":0}),
            json!({"range":{"from":0,"to":1}}),
            json!({"range":{"from":0,"to":10000}}),
        ] {
            let mut request = request.clone();
            for (k, v) in field.as_object().unwrap() {
                request[k] = v.clone();
            }
            let raw = RawValue::from_string(request.to_string()).unwrap();
            assert!(prepare("audio.noise_learn", Some(&raw), &p).is_err());
        }
    }
}
