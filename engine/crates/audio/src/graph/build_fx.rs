//! The nodes of the effects of inserts (T10, T12), made from the model's effects when the graph is
//! built: each effect's settings read from the model's, the switch of its insert given to it, its
//! lanes found by the paths of its parameters (`Param::path`), and its settling worked out for the
//! slowest values that its lanes take (T4b's review, P3-11). Pitch (T13) has no node yet.

use columbina_model::audio::fx::{EssRange, NoiseBand, NoiseMode, Reaction};
use columbina_model::{Effect, Lane};

use crate::fx::{deesser, dehummer, denoise, gain, limiter, soft_clip, width};
use crate::node::{Node, ParamId};

use super::build::{Context, Made};

#[allow(
    clippy::cast_possible_truncation,
    reason = "the settings of a node are f32"
)]
fn single(value: f64) -> f32 {
    value as f32
}

/// `settings`, the value at `path` taken, of what its lane goes through, as the one that settles
/// the slowest by `settle`.
fn slowest<S: Clone>(
    settings: S,
    lanes: &[(&str, &Lane)],
    path: &str,
    put: impl Fn(&mut S, f32),
    settle: impl Fn(&S) -> u32,
) -> S {
    let Some((_, lane)) = lanes.iter().find(|(at, _)| *at == path) else {
        return settings;
    };
    let mut best = settings.clone();
    for point in &lane.points {
        let mut tried = settings.clone();
        put(&mut tried, single(point.value));
        if settle(&tried) > settle(&best) {
            best = tried;
        }
    }
    best
}

/// A made node: `node`, its parameters by path, its settling; it takes its insert's switch.
fn made(node: Box<dyn Node>, param: fn(&str) -> Option<ParamId>, settle: u32) -> Made {
    Made {
        node,
        param,
        settle,
        switched: true,
    }
}

/// The node of `effect`, an insert's, as `context` says; none for an effect with no node yet
/// (Pitch, T13), which passes its sound as it is.
#[must_use]
#[allow(clippy::too_many_lines, reason = "one effect after another")]
pub fn node_of(effect: &Effect, context: &Context) -> Option<Made> {
    let (rate, on, lanes) = (context.rate, context.on, context.lanes);
    Some(match effect {
        Effect::Gain(fx) => {
            let settings = gain::Settings {
                on,
                gain_db: single(fx.gain_db),
            };
            made(
                Box::new(gain::Gain::new(&settings)),
                |path| {
                    gain::Param::ALL
                        .iter()
                        .find(|p| p.path() == path && p.is_automatable())
                        .map(|p| p.id())
                },
                gain::settle_for(&settings, rate),
            )
        }
        Effect::Limiter(fx) => {
            let settings = limiter::Settings {
                on,
                input_db: single(fx.input_db),
                soft: fx.soft,
                ceiling_db: single(fx.ceiling_db),
                release_ms: single(fx.release_ms),
                sidechain_on: fx.sidechain.on,
                sidechain_listen: fx.sidechain.listen,
            };
            let slow = slowest(
                settings,
                lanes,
                "release_ms",
                |s, v| s.release_ms = v,
                |s| limiter::settle_for(s, rate),
            );
            made(
                Box::new(limiter::Limiter::new(&settings)),
                |path| {
                    limiter::Param::ALL
                        .iter()
                        .find(|p| p.path() == path && p.is_automatable())
                        .map(|p| p.id())
                },
                limiter::settle_for(&slow, rate),
            )
        }
        Effect::SoftClipper(fx) => {
            let settings = soft_clip::Settings {
                on,
                threshold_db: single(fx.threshold_db),
                shape: single(fx.shape),
                output_db: single(fx.output_db),
            };
            made(
                Box::new(soft_clip::SoftClipper::new(&settings)),
                |path| {
                    soft_clip::Param::ALL
                        .iter()
                        .find(|p| p.path() == path && p.is_automatable())
                        .map(|p| p.id())
                },
                soft_clip::settle_for(&settings, rate),
            )
        }
        Effect::StereoWidth(fx) => {
            let settings = width::Settings {
                on,
                width: single(fx.width),
                diffusion: single(fx.diffusion),
                sparkle: single(fx.sparkle),
            };
            made(
                Box::new(width::StereoWidth::new(&settings)),
                |path| {
                    width::Param::ALL
                        .iter()
                        .find(|p| p.path() == path && p.is_automatable())
                        .map(|p| p.id())
                },
                width::settle_for(&settings, rate),
            )
        }
        Effect::DeEsser(fx) => {
            let settings = deesser::Settings {
                on,
                frequency_hz: single(fx.frequency_hz),
                range: match fx.range {
                    EssRange::Narrow => deesser::Range::Narrow,
                    EssRange::Wide => deesser::Range::Wide,
                    EssRange::AllHigh => deesser::Range::AllHigh,
                },
                amount: single(fx.amount),
                reaction: match fx.reaction {
                    Reaction::Slow => deesser::Reaction::Slow,
                    Reaction::Medium => deesser::Reaction::Medium,
                    Reaction::Fast => deesser::Reaction::Fast,
                },
                listen: fx.listen,
            };
            let slow = slowest(
                settings,
                lanes,
                "frequency_hz",
                |s, v| s.frequency_hz = v,
                |s| deesser::settle_for(s, rate),
            );
            made(
                Box::new(deesser::DeEsser::new(&settings)),
                |path| {
                    deesser::Param::ALL
                        .iter()
                        .find(|p| p.path() == path && p.is_automatable())
                        .map(|p| p.id())
                },
                deesser::settle_for(&slow, rate),
            )
        }
        Effect::DeHummer(fx) => {
            let settings = dehummer::Settings {
                on,
                frequency_hz: single(fx.frequency_hz),
                amount: single(fx.amount),
                slope: single(fx.slope),
                listen: fx.listen,
            };
            let slow = slowest(
                settings,
                lanes,
                "frequency_hz",
                |s, v| s.frequency_hz = v,
                |s| dehummer::settle_for(s, rate),
            );
            made(
                Box::new(dehummer::DeHummer::new(&settings)),
                |path| {
                    dehummer::Param::ALL
                        .iter()
                        .find(|p| p.path() == path && p.is_automatable())
                        .map(|p| p.id())
                },
                dehummer::settle_for(&slow, rate),
            )
        }
        Effect::NoiseReduction(fx) => {
            let settings = denoise::Settings {
                on,
                mode: match fx.mode {
                    NoiseMode::AutoSpeech => denoise::Mode::AutoSpeech,
                    NoiseMode::Manual => denoise::Mode::Manual,
                    NoiseMode::Voice => denoise::Mode::Voice,
                },
                threshold_db: single(fx.threshold_db),
                attack_ms: single(fx.attack_ms),
                sensitivity: single(fx.sensitivity),
                ratio: single(fx.ratio),
                frequency_smoothing: single(fx.frequency_smoothing),
                time_smoothing: fx.time_smoothing,
                dry_wet: single(fx.dry_wet),
                level_db: single(fx.level_db),
                listen: fx.listen,
                band: match fx.band {
                    NoiseBand::Full => denoise::Band::Full,
                    NoiseBand::Rumble => denoise::Band::Rumble,
                    NoiseBand::Hiss => denoise::Band::Hiss,
                    NoiseBand::RumbleAndHiss => denoise::Band::RumbleAndHiss,
                },
                profile: fx
                    .profile
                    .as_ref()
                    .map(|profile| profile.iter().copied().map(single).collect()),
            };
            let settle = |s: &denoise::Settings| denoise::NoiseReduction::settle_for(s, rate);
            let slow = slowest(
                settings.clone(),
                lanes,
                "attack_ms",
                |s, v| s.attack_ms = v,
                settle,
            );
            let slow = slowest(slow, lanes, "ratio", |s, v| s.ratio = v, settle);
            made(
                Box::new(denoise::NoiseReduction::new(&settings)),
                |path| {
                    denoise::Param::ALL
                        .iter()
                        .find(|p| p.path() == path && p.is_automatable())
                        .map(|p| p.id())
                },
                settle(&slow),
            )
        }
        Effect::Pitch(_) => return None,
    })
}
