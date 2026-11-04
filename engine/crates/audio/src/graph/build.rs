//! Building the graph from a snapshot of the project (plan T4b), off the thread of the mix: here
//! every buffer, node and table is made, and the latencies, the order and the settling are worked
//! out. Nothing here runs while the sound plays.

use std::collections::{BTreeSet, HashMap};

use columbina_model::audio::fx::{
    Ducker as DuckerModel, Dynamics as DynamicsModel, EqKind, EqRange, GateMode,
};
use columbina_model::audio::{BusFormat, ChannelOrder, Pan as PanModel, TrackFormat};
use columbina_model::{
    resolve, AudioParam, Effect, Equalizer, Id, Lane, Lanes, Project, SourceRef, Stage as Order,
    Strip as StripModel, Timeline, TrackKind,
};

use crate::constants::{samples, MAX_CHANNELS, STRETCH_FASTEST, STRETCH_RESET_CROSSFADE};
use crate::db_to_gain;
use crate::feed::ClipSource;
use crate::fx::ducker::{self, Ducker, Param as DuckerParam};
use crate::fx::dynamics::{self, Dynamics, Param as DynamicsParam};
use crate::fx::eq::{self, Eq, Field, Kind, Variant, Zone};
use crate::fx::pitch;
use crate::fx::stretch::{Grid, Plan, Stretcher, CHUNK};
use crate::node::{Layout, Node, ParamId, Planar, Role, Spec};
use crate::rt::{seqlock, Domain, SeqReader, Smoothed};

use super::clip::{
    cutoff, radius, ClipChain, ClipPan, ClipScratch, Mapping, Pool, StretchMode, Stretched, Window,
};
use super::pdc::{DelayLine, Delayer};
use super::route::{balance, panned, PanAt};
use super::strip::{Edge, Key, LaneOf, Pan, Stage, Strip, VcaState, VcaTerm};
use super::taps::Tap;
use super::{Control, Graph, Meter, Options, Out, OutputInfo, Owner, Purpose, StripParam, Target};

/// What a maker is told of an insert besides its effect.
#[derive(Clone, Copy, Debug)]
pub struct Context<'a> {
    /// The rate of the timeline.
    pub rate: u32,
    /// What the graph is for. The effect a maker is given has every Listen off already but in
    /// a preview.
    pub purpose: Purpose,
    /// Whether the insert is on: its own switch, and the Effects In of its strip.
    pub on: bool,
    /// The lanes of the insert, by the path after `insert.<id>.` (`ceiling_db`).
    pub lanes: &'a [(&'a str, &'a Lane)],
}

/// An effect made into a node.
pub struct Made {
    /// The node, not yet prepared.
    pub node: Box<dyn Node>,
    /// The parameter of a path of its settings (`ceiling_db`), if a lane may set it.
    pub param: fn(&str) -> Option<ParamId>,
    /// Its settling for the slowest settings that its lanes take (T8's `settle_for`).
    pub settle: u32,
    /// Whether it takes the switch of its insert itself (`Context::on`, and its `on` lane): a
    /// node that does not is replaced by a delay of its latency while the insert is off.
    pub switched: bool,
}

impl std::fmt::Debug for Made {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Made")
            .field("settle", &self.settle)
            .field("switched", &self.switched)
            .finish_non_exhaustive()
    }
}

/// What makes the node of an effect of an insert, or none where there is no node for it.
pub type Maker = fn(&Effect, &Context) -> Option<Made>;

pub use super::build_fx::node_of;

/// Why a graph could not be built.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum BuildError {
    /// The project has no such timeline.
    #[error("the project has no timeline {0}")]
    Timeline(usize),
    /// The timeline has no rate of sound, or a block of no samples was asked for.
    #[error("a graph of {rate} samples a second and blocks of {max_block}")]
    Size {
        /// The rate of the timeline.
        rate: u32,
        /// The block asked for.
        max_block: usize,
    },
    /// The sound goes round in a loop through this strip (the model refuses it).
    #[error("the sound goes round in a loop through {0}")]
    Loop(Id),
}

/// The graphs built so far: each graph's id is the count before it.
static GRAPHS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The roles of the channels of `layout` but the LFE: the channels of a sidechain from it.
fn keyed_roles(layout: Layout) -> Vec<Role> {
    layout
        .roles()
        .iter()
        .copied()
        .filter(|role| *role != Role::Lfe)
        .collect()
}

/// Where each channel of a sidechain from a strip of `from` goes in a sidechain of `to` (the
/// channels of both but the LFE, by their places there), when sources of different formats are
/// summed for a Ducker (T11's review, P3-7): to the channel of the same role; else mono and the
/// centre to the centre, or to left and right; the one surround of LCRS to the two surrounds, or
/// to left and right; a surround or rear of one side to the surround of that side, or to that
/// side; and anything else (a channel of an Adaptive track, a side into mono) to the channel of
/// its number, round the channels of `to`.
pub(crate) fn routes(from: Layout, to: Layout) -> Vec<(usize, usize)> {
    use Role::{Centre, Left, LeftRear, LeftSurround, Mono, Right, RightRear, RightSurround};
    let to_roles = keyed_roles(to);
    let find = |role: Role| to_roles.iter().position(|&r| r == role);
    let both = |a: Role, b: Role| find(a).zip(find(b)).map(|(a, b)| vec![a, b]);
    let mut routes = Vec::new();
    for (channel, role) in keyed_roles(from).into_iter().enumerate() {
        let into = find(role).map(|at| vec![at]).or_else(|| match role {
            Mono | Centre => find(Centre).map(|c| vec![c]).or_else(|| both(Left, Right)),
            Role::Surround => both(LeftSurround, RightSurround).or_else(|| both(Left, Right)),
            LeftSurround | LeftRear => find(LeftSurround).or_else(|| find(Left)).map(|c| vec![c]),
            RightSurround | RightRear => {
                find(RightSurround).or_else(|| find(Right)).map(|c| vec![c])
            }
            _ => None,
        });
        let into = into.unwrap_or_else(|| vec![channel % to_roles.len().max(1)]);
        routes.extend(into.into_iter().map(|at| (channel, at)));
    }
    routes
}

/// Where a sidechain from `source`, of `layout`, comes from: its channels but the LFE (T8's
/// review), delayed by `side` samples, and summed into a sidechain of `into`.
fn key_of(source: usize, layout: Layout, side: u32, into: Layout) -> Key {
    let mut channels = [0; MAX_CHANNELS];
    let mut count = 0;
    for (channel, role) in layout.roles().iter().enumerate() {
        if *role != Role::Lfe {
            channels[count] = channel;
            count += 1;
        }
    }
    Key {
        source,
        channels,
        count,
        routes: routes(layout, into).into_boxed_slice(),
        side: delay_line(count, usize::try_from(side).unwrap_or(0)),
    }
}

/// A node of a strip, `offset` samples behind the input of the graph, with the sidechains `keys`
/// summed into its own.
fn stage(node: Box<dyn Node>, keys: Vec<Key>, max_block: usize, offset: u32) -> Stage {
    let count = keys.iter().map(|key| key.count).max().unwrap_or(0);
    let frames = |used: bool| if used && count > 0 { max_block } else { 0 };
    let summed = keys.len() > 1;
    Stage {
        node,
        lanes: Box::new([]),
        keyed: Planar::new(count, frames(true)),
        summed: Planar::new(if summed { count } else { 0 }, frames(summed)),
        keys: keys.into_boxed_slice(),
        offset: i64::from(offset),
        processed: 0,
    }
}

/// A line that delays `channels` by `length` samples.
fn delay_line(channels: usize, length: usize) -> DelayLine {
    DelayLine {
        lines: vec![0.0; channels * length].into_boxed_slice(),
        length,
        at: 0,
    }
}

/// A node of a strip before the latencies are known: the node, what it is, and the strips that
/// its sidechain comes from.
type Unplaced = (Box<dyn Node>, StageKind, Vec<usize>);

/// What a node of a strip is, to find the parameters that lanes and controls set.
#[derive(Clone, Copy, Debug)]
pub(crate) enum StageKind {
    Ducker,
    Dynamics,
    Eq,
    Insert(usize, fn(&str) -> Option<ParamId>),
    Bypass,
}

/// What a strip has that its settings are found by: its outputs and sends (the bus, and whether
/// it is a send), and its nodes (with the ids of its inserts).
#[derive(Clone, Debug, Default)]
pub(crate) struct Meta {
    pub(crate) edges: Vec<(Id, bool)>,
    pub(crate) stages: Vec<StageKind>,
    pub(crate) inserts: Vec<Id>,
}

/// The setting of a strip at `path`, the path of a lane (T1: `model/src/audio.rs`,
/// `automatable`), if the graph has it.
pub(crate) fn param_of(meta: &Meta, path: &str) -> Option<StripParam> {
    let simple = match path {
        "trim_db" => Some(StripParam::Trim),
        "invert" => Some(StripParam::Invert),
        "fader_db" => Some(StripParam::Fader),
        "mute" => Some(StripParam::Mute),
        "pan.x" => Some(StripParam::PanX),
        "pan.y" => Some(StripParam::PanY),
        "pan.divergence" => Some(StripParam::PanDivergence),
        _ => None,
    };
    if simple.is_some() {
        return simple;
    }
    let stage = |wanted: fn(&StageKind) -> bool| meta.stages.iter().position(wanted);
    if let Some(rest) = path.strip_prefix("track_fx.") {
        let stage = stage(|kind| matches!(kind, StageKind::Ducker))?;
        let param = DuckerParam::ALL
            .iter()
            .find(|param| param.path() == rest && param.is_automatable())?;
        return Some(StripParam::Node(stage, param.id()));
    }
    if let Some(rest) = path.strip_prefix("dynamics.") {
        let stage = stage(|kind| matches!(kind, StageKind::Dynamics))?;
        let param = DynamicsParam::ALL
            .iter()
            .find(|param| param.path() == rest && param.is_automatable())?;
        return Some(StripParam::Node(stage, param.id()));
    }
    if let Some(rest) = path.strip_prefix("eq.") {
        let stage = stage(|kind| matches!(kind, StageKind::Eq))?;
        let param = match rest {
            "on" => eq::ON,
            "gain_db" => eq::GAIN,
            _ => {
                let (band, field) = rest.split_once('.')?;
                let band: usize = band.parse().ok()?;
                let field = match field {
                    "on" => Field::On,
                    "freq" => Field::Freq,
                    "gain_db" => Field::Gain,
                    "q" => Field::Q,
                    _ => return None,
                };
                if band >= Variant::Strip.bands() {
                    return None;
                }
                eq::param(band, field)
            }
        };
        return Some(StripParam::Node(stage, param));
    }
    for (prefix, send) in [("send.", true), ("output.", false)] {
        if let Some(rest) = path.strip_prefix(prefix) {
            for (edge, (bus, is_send)) in meta.edges.iter().enumerate() {
                if *is_send != send {
                    continue;
                }
                let Some(field) = rest
                    .strip_prefix(bus.as_str())
                    .and_then(|r| r.strip_prefix('.'))
                else {
                    continue;
                };
                return match (field, send) {
                    ("on", _) => Some(StripParam::On(edge)),
                    ("level_db", true) => Some(StripParam::Level(edge)),
                    ("pan.x", true) => Some(StripParam::EdgePanX(edge)),
                    ("pan.y", true) => Some(StripParam::EdgePanY(edge)),
                    _ => None,
                };
            }
            return None;
        }
    }
    if let Some(rest) = path.strip_prefix("insert.") {
        for (at, kind) in meta.stages.iter().enumerate() {
            if let StageKind::Insert(insert, param) = kind {
                let id = &meta.inserts[*insert];
                if let Some(field) = rest
                    .strip_prefix(id.as_str())
                    .and_then(|r| r.strip_prefix('.'))
                {
                    return Some(StripParam::Node(at, param(field)?));
                }
            }
        }
    }
    None
}

/// `effect` with every Listen off: what an export or an analysis hears of it (Listen is only
/// for the monitor, T8; the model's `unlisten` does the same to the lanes).
pub(crate) fn unlistened(effect: &Effect) -> Effect {
    let mut effect = effect.clone();
    match &mut effect {
        Effect::Limiter(fx) => fx.sidechain.listen = false,
        Effect::DeEsser(fx) => fx.listen = false,
        Effect::DeHummer(fx) => fx.listen = false,
        Effect::NoiseReduction(fx) => fx.listen = false,
        Effect::Gain(_) | Effect::SoftClipper(_) | Effect::StereoWidth(_) | Effect::Pitch(_) => {}
    }
    effect
}

/// The layout of a track of `format` and `channels`.
fn track_layout(format: TrackFormat, channels: u8) -> Layout {
    match format {
        TrackFormat::Mono => Layout::Mono,
        TrackFormat::Stereo => Layout::Stereo,
        TrackFormat::FiveOne => Layout::Surround51,
        TrackFormat::Adaptive => Layout::discrete(usize::from(channels)),
    }
}

/// The layout of a bus of `format`.
fn bus_layout(format: BusFormat) -> Layout {
    match format {
        BusFormat::Mono => Layout::Mono,
        BusFormat::Stereo => Layout::Stereo,
        BusFormat::Lcr => Layout::Lcr,
        BusFormat::Lcrs => Layout::Lcrs,
        BusFormat::Five => Layout::Surround50,
        BusFormat::FiveOne => Layout::Surround51,
        BusFormat::Seven => Layout::Surround70,
        BusFormat::SevenOne => Layout::Surround71,
    }
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "the settings of a node are f32"
)]
fn single(value: f64) -> f32 {
    value as f32
}

/// The settings of an equalizer of the model as the node of `variant` takes them.
pub(crate) fn eq_settings(model: &Equalizer, variant: Variant) -> eq::Settings {
    let mut settings = eq::Settings::of(variant);
    settings.on = model.on;
    settings.gain_db = single(model.gain_db);
    for (band, model) in settings
        .bands
        .iter_mut()
        .zip(&model.bands)
        .take(variant.bands())
    {
        *band = eq::Band {
            on: model.on,
            kind: match model.kind {
                EqKind::LowShelf => Kind::LowShelf,
                EqKind::Bell => Kind::Bell,
                EqKind::HighShelf => Kind::HighShelf,
                EqKind::HighPass => Kind::HighPass,
                EqKind::LowPass => Kind::LowPass,
                EqKind::Notch => Kind::Notch,
            },
            freq: single(model.freq),
            gain_db: single(model.gain_db),
            q: single(model.q),
            slope: model.slope,
            zone: model.range.map(|range| match range {
                EqRange::Low => Zone::Low,
                EqRange::MediumLow => Zone::MediumLow,
                EqRange::MediumHigh => Zone::MediumHigh,
                EqRange::High => Zone::High,
            }),
        };
    }
    settings.sanitized()
}

/// The settings of Dynamics of the model as the node takes them; Listen only where `listen`.
pub(crate) fn dynamics_settings(model: &DynamicsModel, listen: bool) -> dynamics::Settings {
    let (e, c, l) = (&model.expander, &model.compressor, &model.limiter);
    dynamics::Settings {
        on: model.on,
        expander: dynamics::ExpanderSettings {
            on: e.on,
            gate: e.mode == GateMode::Gate,
            threshold_db: single(e.threshold_db),
            range_db: single(e.range_db),
            ratio: single(e.ratio),
            attack_ms: single(e.attack_ms),
            hold_ms: single(e.hold_ms),
            release_ms: single(e.release_ms),
        },
        compressor: dynamics::CompressorSettings {
            on: c.on,
            threshold_db: single(c.threshold_db),
            ratio: single(c.ratio),
            knee_db: single(c.knee_db),
            mix: single(c.mix),
            attack_ms: single(c.attack_ms),
            hold_ms: single(c.hold_ms),
            release_ms: single(c.release_ms),
            sidechain: c.sidechain.on,
            listen: listen && c.sidechain.listen,
        },
        limiter: dynamics::LimiterSettings {
            on: l.on,
            threshold_db: single(l.threshold_db),
            attack_ms: single(l.attack_ms),
            hold_ms: single(l.hold_ms),
            release_ms: single(l.release_ms),
        },
        makeup_db: single(model.makeup_db),
    }
}

/// The values that a lane takes.
/// The settings of the Ducker of the model.
pub(crate) fn ducker_settings(model: &DuckerModel) -> ducker::Settings {
    ducker::Settings {
        on: model.on,
        amount_db: single(model.amount_db),
        lookahead_ms: single(model.lookahead_ms),
        rise_ms: single(model.rise_ms),
        hold_ms: single(model.hold_ms),
        recovery_ms: single(model.recovery_ms),
    }
}

/// `settings` with the longest rise, hold and recovery that its lanes (`track_fx.…`) take: those
/// that settle the slowest (T8's review).
fn slowest_ducker(mut settings: ducker::Settings, lanes: &Lanes) -> ducker::Settings {
    for (path, value) in [
        ("track_fx.rise_ms", &mut settings.rise_ms),
        ("track_fx.hold_ms", &mut settings.hold_ms),
        ("track_fx.recovery_ms", &mut settings.recovery_ms),
    ] {
        if let Some(lane) = lanes.get(path) {
            for point in points(lane) {
                *value = value.max(single(point));
            }
        }
    }
    settings
}

fn points(lane: &Lane) -> impl Iterator<Item = f64> + '_ {
    lane.points.iter().map(|point| point.value)
}

/// Whether the switch of `path` is on, as set or anywhere its lane goes.
fn ever_on(on: bool, lanes: &Lanes, path: &str) -> bool {
    on || lanes
        .get(path)
        .is_some_and(|lane| points(lane).any(|value| value >= 0.5))
}

/// The slowest settings of Dynamics that `settings` and its lanes take: every switch on that is
/// on anywhere, every time at its longest (T8: `settle_for`).
fn slowest_dynamics(mut settings: dynamics::Settings, lanes: &Lanes) -> dynamics::Settings {
    for param in DynamicsParam::ALL {
        let Some(lane) = lanes.get(&format!("dynamics.{}", param.path())) else {
            continue;
        };
        let path = param.path();
        let slowest = path.ends_with("_ms") || param.is_switch();
        if slowest {
            let most = points(lane).fold(f64::from(settings.value(param)), f64::max);
            settings.set(param, single(most));
        }
    }
    settings
}

/// The slowest settings of an equalizer that `settings` and its lanes take: of each band the
/// lowest frequency, the highest Q and the largest gain in size (T7: `settle_for`).
fn slowest_eq(mut settings: eq::Settings, lanes: &Lanes) -> eq::Settings {
    for (index, band) in settings.bands.iter_mut().enumerate() {
        let lane = |field: &str| lanes.get(&format!("eq.{index}.{field}"));
        if let Some(lane) = lane("freq") {
            band.freq = points(lane).map(single).fold(band.freq, f32::min);
        }
        if let Some(lane) = lane("q") {
            band.q = points(lane).map(single).fold(band.q, f32::max);
        }
        if let Some(lane) = lane("gain_db") {
            band.gain_db =
                points(lane)
                    .map(single)
                    .fold(band.gain_db, |a, b| if b.abs() > a.abs() { b } else { a });
        }
    }
    settings
}

/// A parameter at `value`, ramped over 10 ms at `rate`.
fn smoothed(value: f32, rate: u32) -> Smoothed {
    let mut smoothed = Smoothed::new(value, Domain::Linear);
    smoothed.prepare(rate);
    smoothed
}

fn switch(on: bool) -> f32 {
    if on {
        1.0
    } else {
        0.0
    }
}

/// A pan of the model as the graph reads it.
fn pan_at(pan: &PanModel) -> PanAt {
    PanAt {
        on: pan.on,
        x: single(pan.x),
        y: single(pan.y),
        divergence: single(pan.divergence / 100.0),
        boom: if pan.lfe_on {
            db_to_gain(single(pan.lfe_db))
        } else {
            0.0
        },
    }
}

/// A point where the level is taken.
fn tap() -> Tap {
    let (writer, reader) = seqlock((0, [0.0; MAX_CHANNELS], [0.0; MAX_CHANNELS]));
    Tap {
        writer,
        reader,
        frames: 0,
    }
}

/// A strip of the model, and what it is in the timeline.
#[allow(clippy::struct_excessive_bools, reason = "the buttons of a strip")]
struct Source<'a> {
    owner: Owner,
    strip: &'a StripModel,
    lanes: &'a Lanes,
    layout: Layout,
    mute: bool,
    solo: bool,
    solo_safe: bool,
    /// A track whose ADC is off.
    uncompensated: bool,
}

/// The VCAs that `vcas` name, and those that they are members of, each once, by their places.
fn chain(timeline: &Timeline, vcas: &[Id], places: &HashMap<&str, usize>) -> Vec<usize> {
    let mut found = Vec::new();
    let mut next: Vec<&Id> = vcas.iter().collect();
    while let Some(id) = next.pop() {
        let Some(&at) = places.get(id.as_str()) else {
            continue;
        };
        if found.contains(&at) {
            continue;
        }
        found.push(at);
        next.extend(timeline.vcas[at].vcas.iter());
    }
    found.sort_unstable();
    found
}

/// Every strip that `from` reaches along `edges`, `from` and all.
fn reach(edges: &[Vec<usize>], from: &[usize]) -> Vec<bool> {
    let mut reached = vec![false; edges.len()];
    let mut next = from.to_vec();
    while let Some(at) = next.pop() {
        if reached[at] {
            continue;
        }
        reached[at] = true;
        next.extend(&edges[at]);
    }
    reached
}

impl Graph {
    /// The graph of timeline `timeline` of `project`, as `options` say.
    ///
    /// # Errors
    ///
    /// There is no such timeline; it has no rate of sound or the block is empty; its routing
    /// loops (which the model refuses); or a clip reads beyond the samples a count holds.
    #[allow(
        clippy::too_many_lines,
        reason = "the graph is built in one pass, step by step"
    )]
    pub fn build(
        project: &Project,
        timeline: usize,
        options: Options,
    ) -> Result<Graph, BuildError> {
        let t = project
            .timelines
            .get(timeline)
            .ok_or(BuildError::Timeline(timeline))?;
        let (rate, max_block) = (t.sample_rate, options.max_block);
        if rate == 0 || max_block == 0 {
            return Err(BuildError::Size { rate, max_block });
        }
        // The strips: the tracks of sound that are on, in their order, then the buses.
        let mut sources: Vec<Source> = Vec::new();
        for track in t
            .tracks
            .iter()
            .filter(|r| r.kind == TrackKind::Audio && r.enabled)
        {
            let audio = &track.audio;
            sources.push(Source {
                owner: Owner::Track(track.id.clone()),
                strip: &audio.strip,
                lanes: &audio.lanes,
                layout: track_layout(audio.format, audio.channels),
                mute: audio.mute,
                solo: audio.solo,
                solo_safe: audio.solo_safe,
                uncompensated: !audio.adc,
            });
        }
        let tracks = sources.len();
        for bus in &t.buses {
            sources.push(Source {
                owner: Owner::Bus(bus.id.clone()),
                strip: &bus.strip,
                lanes: &bus.lanes,
                layout: bus_layout(bus.format),
                mute: bus.mute,
                solo: bus.solo,
                solo_safe: bus.solo_safe,
                uncompensated: false,
            });
        }
        let count = sources.len();
        let track_at: HashMap<&str, usize> = t
            .tracks
            .iter()
            .filter(|r| r.kind == TrackKind::Audio && r.enabled)
            .enumerate()
            .map(|(at, track)| (track.id.as_str(), at))
            .collect();
        let bus_at: HashMap<&str, usize> = t
            .buses
            .iter()
            .enumerate()
            .map(|(at, bus)| (bus.id.as_str(), tracks + at))
            .collect();
        let vca_at: HashMap<&str, usize> = t
            .vcas
            .iter()
            .enumerate()
            .map(|(at, vca)| (vca.id.as_str(), at))
            .collect();
        let members: Vec<Vec<usize>> = sources
            .iter()
            .map(|source| chain(t, &source.strip.vcas, &vca_at))
            .collect();

        // The connections: outputs, then sends, to buses that there are.
        let mut metas: Vec<Meta> = vec![Meta::default(); count];
        let mut feeds: Vec<Vec<usize>> = vec![Vec::new(); count];
        let mut connections: Vec<Vec<(usize, bool, Option<&columbina_model::Send>)>> =
            vec![Vec::new(); count];
        for (at, source) in sources.iter().enumerate() {
            for output in &source.strip.outputs {
                if let Some(&dest) = bus_at.get(output.bus.as_str()) {
                    connections[at].push((dest, false, None));
                    metas[at].edges.push((output.bus.clone(), false));
                    feeds[at].push(dest);
                }
            }
            for send in &source.strip.sends {
                if let Some(&dest) = bus_at.get(send.bus.as_str()) {
                    connections[at].push((dest, true, Some(send)));
                    metas[at].edges.push((send.bus.clone(), true));
                    feeds[at].push(dest);
                }
            }
        }
        // Sidechains: the sources of the Compressor of Dynamics that is built and of the Limiters
        // of the inserts.
        let built_dynamics: Vec<bool> = sources
            .iter()
            .map(|s| ever_on(s.strip.dynamics.on, s.lanes, "dynamics.on"))
            .collect();
        let strip_of = |reference: &SourceRef| match reference {
            SourceRef::Track(id) => track_at.get(id.as_str()).copied(),
            SourceRef::Bus(id) => bus_at.get(id.as_str()).copied(),
        };
        let side_channels = |source: usize| {
            let layout = sources[source].layout;
            layout.channels() - usize::from(layout.lfe().is_some())
        };
        // The Ducker of a track (T11): built where some of its sources are in the graph, on or
        // off, so that its latency stays when it is switched (C-X12; T11's review, P2-2); the
        // strips of its sources, each once.
        let duckers: Vec<Option<(&DuckerModel, Vec<usize>)>> = sources
            .iter()
            .enumerate()
            .map(|(at, source)| {
                let model = source.strip.track_fx.as_ref().filter(|_| at < tracks)?;
                let mut keyed: Vec<usize> = Vec::new();
                for strip in model.sources.iter().filter_map(strip_of) {
                    if !keyed.contains(&strip) {
                        keyed.push(strip);
                    }
                }
                (!keyed.is_empty()).then_some((model, keyed))
            })
            .collect();
        let keys: Vec<Vec<usize>> = sources
            .iter()
            .zip(&built_dynamics)
            .zip(&duckers)
            .map(|((source, &built), ducker)| {
                let dynamics = if built {
                    source.strip.dynamics.compressor.sidechain.source.as_ref()
                } else {
                    None
                };
                let inserts = source
                    .strip
                    .inserts
                    .iter()
                    .filter_map(|insert| insert.effect.sidechain()?.source.as_ref());
                let mut keys: Vec<usize> = dynamics
                    .into_iter()
                    .chain(inserts)
                    .filter_map(strip_of)
                    .collect();
                keys.extend(ducker.iter().flat_map(|(_, keyed)| keyed));
                keys
            })
            .collect();

        // The order: Kahn's, the lowest place first, so that tracks keep their order.
        let mut before: Vec<Vec<usize>> = feeds.clone();
        for (at, keys) in keys.iter().enumerate() {
            for &source in keys {
                before[source].push(at);
            }
        }
        let mut waiting = vec![0usize; count];
        for next in &before {
            for &to in next {
                waiting[to] += 1;
            }
        }
        let mut ready: BTreeSet<usize> = (0..count).filter(|&at| waiting[at] == 0).collect();
        let mut order = Vec::with_capacity(count);
        while let Some(at) = ready.pop_first() {
            order.push(at);
            for &to in &before[at] {
                waiting[to] -= 1;
                if waiting[to] == 0 {
                    ready.insert(to);
                }
            }
        }
        if order.len() != count {
            let stuck = (0..count).find(|&at| waiting[at] > 0).unwrap_or(0);
            let id = match &sources[stuck].owner {
                Owner::Track(id) | Owner::Bus(id) | Owner::Vca(id) => id.clone(),
            };
            return Err(BuildError::Loop(id));
        }

        // Solo (p. 3885; the interaction reference §3.6): while anything is soloed, what is
        // heard is what is soloed or solo-safe, the buses it feeds, and what feeds a soloed bus.
        let soloed: Vec<bool> = sources
            .iter()
            .zip(&members)
            .map(|(source, members)| source.solo || members.iter().any(|&v| t.vcas[v].solo))
            .collect();
        let audible: Vec<bool> = if soloed.iter().any(|&s| s) {
            let kept: Vec<usize> = (0..count)
                .filter(|&at| soloed[at] || sources[at].solo_safe)
                .collect();
            let down = reach(&feeds, &kept);
            let mut back = vec![Vec::new(); count];
            for (from, to) in feeds.iter().enumerate() {
                for &to in to {
                    back[to].push(from);
                }
            }
            let soloed_buses: Vec<usize> = (tracks..count).filter(|&at| soloed[at]).collect();
            let up = reach(&back, &soloed_buses);
            (0..count).map(|at| down[at] || up[at]).collect()
        } else {
            vec![true; count]
        };

        // The nodes of each strip: its Ducker, then the rest in its Order.
        let listen = options.purpose == Purpose::Preview;
        let mut stages: Vec<Vec<Unplaced>> = Vec::with_capacity(count);
        let mut settles: Vec<u64> = vec![0; count];
        let mut ducker_meters: Vec<(usize, SeqReader<ducker::Meter>)> = Vec::new();
        for (at, source) in sources.iter().enumerate() {
            let layout = source.layout;
            let spec = Spec::new(rate, max_block, layout);
            let mut list: Vec<Unplaced> = Vec::new();
            let meta = &mut metas[at];
            if let Some((model, keyed)) = &duckers[at] {
                let settings = ducker_settings(model);
                let mut node = Ducker::new(&settings);
                let channels = keyed.iter().map(|&s| side_channels(s)).max().unwrap_or(0);
                if node.prepare(&spec.with_sidechain(channels)).is_ok() {
                    let slowest = slowest_ducker(settings, source.lanes);
                    settles[at] += u64::from(ducker::settle_for(&slowest, rate));
                    ducker_meters.push((at, node.meter()));
                    list.push((Box::new(node), StageKind::Ducker, keyed.clone()));
                }
            }
            for stage in &source.strip.order {
                match stage {
                    Order::Fx => {
                        for insert in &source.strip.inserts {
                            let effect = if listen {
                                insert.effect.clone()
                            } else {
                                unlistened(&insert.effect)
                            };
                            let prefix = format!("insert.{}.", insert.id);
                            let lanes: Vec<(&str, &Lane)> = source
                                .lanes
                                .iter()
                                .filter_map(|(path, lane)| {
                                    Some((path.strip_prefix(prefix.as_str())?, lane))
                                })
                                .collect();
                            let on = insert.on && source.strip.inserts_on;
                            let context = Context {
                                rate,
                                purpose: options.purpose,
                                on,
                                lanes: &lanes,
                            };
                            let Some(made) = (options.maker)(&effect, &context) else {
                                continue;
                            };
                            let key = effect
                                .sidechain()
                                .and_then(|side| side.source.as_ref())
                                .and_then(strip_of);
                            let mut node = made.node;
                            let keyed = spec.with_sidechain(key.map_or(0, side_channels));
                            if node.prepare(&keyed).is_err() {
                                continue;
                            }
                            if on || made.switched {
                                meta.inserts.push(insert.id.clone());
                                settles[at] += u64::from(made.settle);
                                let kind = StageKind::Insert(meta.inserts.len() - 1, made.param);
                                list.push((node, kind, key.into_iter().collect()));
                            } else {
                                let length = usize::try_from(node.latency()).unwrap_or(0);
                                let mut bypass = Delayer::new(length);
                                if bypass.prepare(&spec).is_ok() {
                                    list.push((Box::new(bypass), StageKind::Bypass, Vec::new()));
                                }
                            }
                        }
                    }
                    Order::Dynamics if built_dynamics[at] => {
                        let settings = dynamics_settings(&source.strip.dynamics, listen);
                        let mut node = Dynamics::new(&settings);
                        let key = source
                            .strip
                            .dynamics
                            .compressor
                            .sidechain
                            .source
                            .as_ref()
                            .and_then(strip_of);
                        let spec = spec.with_sidechain(key.map_or(0, side_channels));
                        if node.prepare(&spec).is_err() {
                            continue;
                        }
                        let slowest = slowest_dynamics(settings, source.lanes);
                        settles[at] += u64::from(Dynamics::settle_for(&slowest, rate));
                        let key = key.into_iter().collect();
                        list.push((Box::new(node) as Box<dyn Node>, StageKind::Dynamics, key));
                    }
                    Order::Eq if ever_on(source.strip.eq.on, source.lanes, "eq.on") => {
                        let settings = eq_settings(&source.strip.eq, Variant::Strip);
                        let mut node = Eq::new(&settings);
                        if node.prepare(&spec).is_err() {
                            continue;
                        }
                        let slowest = slowest_eq(settings, source.lanes);
                        settles[at] += u64::from(eq::settle_for(&slowest, rate));
                        list.push((Box::new(node) as Box<dyn Node>, StageKind::Eq, Vec::new()));
                    }
                    Order::Dynamics | Order::Eq => {}
                }
            }
            stages.push(list);
        }

        // The latencies, strip by strip in their order: at the input of a bus the longest of what
        // feeds it, the tracks whose ADC is off aside; through a strip its nodes'; a node with a
        // sidechain has each of its sources delayed to the sound it acts on, or, where a source is
        // the later, the strip delayed before the node to the latest (T4b's review, P3-6).
        let mut feeders: Vec<Vec<usize>> = vec![Vec::new(); count];
        for (from, to) in feeds.iter().enumerate() {
            for &to in to {
                feeders[to].push(from);
            }
        }
        let mut arrival = vec![0u32; count];
        let mut latencies = vec![0u32; count];
        let mut built: Vec<Vec<Stage>> = (0..count).map(|_| Vec::new()).collect();
        let mut lists: Vec<Option<Vec<_>>> = stages.into_iter().map(Some).collect();
        for &at in &order {
            arrival[at] = feeders[at]
                .iter()
                .filter(|&&from| !sources[from].uncompensated)
                .map(|&from| arrival[from] + latencies[from])
                .max()
                .unwrap_or(0);
            let spec = Spec::new(rate, max_block, sources[at].layout);
            let mut offset = arrival[at];
            let mut kinds = Vec::new();
            for (node, kind, keyed) in lists[at].take().unwrap_or_default() {
                let from = |source: usize| arrival[source] + latencies[source];
                if let Some(latest) = keyed.iter().map(|&source| from(source)).max() {
                    if latest > offset {
                        let mut align = Delayer::new(usize::try_from(latest - offset).unwrap_or(0));
                        if align.prepare(&spec).is_ok() {
                            built[at].push(stage(Box::new(align), Vec::new(), max_block, offset));
                            kinds.push(StageKind::Bypass);
                            offset = latest;
                        }
                    }
                }
                let latency = node.latency();
                // The sidechain is of the format of the source of the most channels, the first
                // of them; it comes first, the others summed into it by their roles.
                let mut keyed = keyed;
                let channels = |source: usize| keyed_roles(sources[source].layout).len();
                if let Some(widest) = (0..keyed.len()).max_by_key(|&k| (channels(keyed[k]), !k)) {
                    keyed[..=widest].rotate_right(1);
                }
                let into = keyed.first().map_or(Layout::Empty, |&s| sources[s].layout);
                let keys = keyed
                    .iter()
                    .map(|&source| {
                        let side = offset.saturating_sub(from(source));
                        key_of(source, sources[source].layout, side, into)
                    })
                    .collect();
                built[at].push(stage(node, keys, max_block, offset));
                kinds.push(kind);
                offset += latency;
            }
            metas[at].stages = kinds;
            latencies[at] = offset - arrival[at];
        }
        let out = |at: usize| arrival[at] + latencies[at];
        let mains: Vec<usize> = (tracks..count)
            .filter(|&at| sources[at].strip.outputs.is_empty())
            .collect();
        let monitor_strip = t
            .audio
            .monitor
            .as_deref()
            .and_then(|id| bus_at.get(id).copied());
        let mut outs: Vec<usize> = mains.clone();
        if let Some(monitor) = monitor_strip {
            if !outs.contains(&monitor) {
                outs.push(monitor);
            }
        }
        let latency = outs.iter().map(|&at| out(at)).max().unwrap_or(0);

        // The settling: of each path, its nodes' summed; the clips' equalizers start the paths
        // of their tracks.
        let mut settle_in = vec![0u64; count];

        // What the mix needs of each track: that it is heard, or that it drives a sidechain or a
        // Ducker, itself or through the buses it feeds.
        let mut driving: Vec<usize> = Vec::new();
        for source in &sources {
            let strip = source.strip;
            let refs =
                strip
                    .dynamics
                    .compressor
                    .sidechain
                    .source
                    .iter()
                    .chain(strip.inserts.iter().filter_map(|insert| {
                        insert.effect.sidechain().and_then(|s| s.source.as_ref())
                    }))
                    .chain(strip.track_fx.iter().flat_map(|ducker| &ducker.sources));
            for reference in refs {
                let at = match reference {
                    SourceRef::Track(id) => track_at.get(id.as_str()),
                    SourceRef::Bus(id) => bus_at.get(id.as_str()),
                };
                driving.extend(at.copied());
            }
        }
        let needed: Vec<bool> = t
            .tracks
            .iter()
            .filter(|r| r.kind == TrackKind::Audio && r.enabled)
            .enumerate()
            .map(|(at, track)| {
                resolve::heard(t, track) || {
                    let reached = reach(&feeds, &[at]);
                    driving.iter().any(|&strip| reached[strip])
                }
            })
            .collect();

        // The clips.
        let mut chains: Vec<ClipChain> = Vec::new();
        let mut clip_sources: Vec<ClipSource> = Vec::new();
        let mut unplayable: Vec<Id> = Vec::new();
        let mut window = 0usize;
        let mut stretching = Stretching::default();
        for piece in resolve::sources_between(t, project, i64::MIN, i64::MAX) {
            let Some(&strip) = track_at.get(piece.track.as_str()) else {
                continue;
            };
            let Some(track) = t.tracks.iter().find(|r| r.id == piece.track) else {
                continue;
            };
            let Some(clip) = track.clips.iter().find(|c| c.id == piece.clip) else {
                continue;
            };
            let cut = cutoff(piece.speed);
            let reach = radius(cut);
            let fits = |mapping: &Mapping, sample: i64| {
                mapping.checked_at(sample).is_some_and(|(at, _)| {
                    at.checked_sub(reach).is_some() && at.checked_add(reach + 1).is_some()
                })
            };
            // A clip that reads beyond what a count of samples holds is silent, and told of.
            let Some(mapping) = Mapping::new(clip, t)
                .filter(|mapping| fits(mapping, piece.from) && fits(mapping, piece.to - 1))
            else {
                unplayable.push(clip.id.clone());
                continue;
            };
            let layout = sources[strip].layout;
            let channels = layout.channels();
            let speed = (i128::from(piece.speed.0), i128::from(piece.speed.1));
            let span = (i128::try_from(max_block).unwrap_or(i128::MAX) * speed.0 + speed.1 - 1)
                / speed.1
                + 2 * i128::from(reach)
                + 2;
            window = window.max(usize::try_from(span).unwrap_or(usize::MAX));
            let pan = match (&clip.pan, layout) {
                (AudioParam::Fixed(_), Layout::Stereo) => ClipPan::Fixed(balance(piece.pan)),
                (AudioParam::Keys(_), Layout::Stereo) => ClipPan::Keyed,
                _ => ClipPan::None,
            };
            let eq = if clip.eq.on {
                let settings = eq_settings(&clip.eq, Variant::Clip);
                let mut node = Eq::new(&settings);
                node.prepare(&Spec::new(rate, max_block, layout))
                    .ok()
                    .map(|()| {
                        settle_in[strip] =
                            settle_in[strip].max(u64::from(eq::settle_for(&settings, rate)));
                        node
                    })
            } else {
                None
            };
            let stretch = stretching.of(clip, &piece, &mapping, (rate, channels), reach);
            if let Some(stretch) = &stretch {
                if stretch.mode == StretchMode::Source {
                    window = window.max(stretching.seek_length());
                }
            }
            clip_sources.push(ClipSource {
                clip: clip.id.clone(),
                track: track.id.clone(),
                media: piece.media.clone(),
                channels,
                heard: needed[strip],
            });
            chains.push(ClipChain {
                strip,
                from: piece.from,
                to: piece.to,
                mapping,
                cutoff: cut,
                radius: reach,
                pan,
                channels,
                film: track.audio.format == TrackFormat::FiveOne
                    && track.audio.order == ChannelOrder::Film
                    && clip.channels.is_some(),
                eq,
                processed: 0,
                piece,
                stretch,
            });
        }
        let mut clips_of = vec![0..0; count];
        let mut start = 0;
        for (strip, range) in clips_of.iter_mut().enumerate() {
            let end = start
                + chains[start..]
                    .iter()
                    .take_while(|c| c.strip == strip)
                    .count();
            *range = start..end;
            start = end;
        }

        let mut settle_out = vec![0u64; count];
        for &at in &order {
            let mut from = settle_in[at];
            for &feeder in &feeders[at] {
                from = from.max(settle_out[feeder]);
            }
            for &source in &keys[at] {
                from = from.max(settle_out[source]);
            }
            settle_out[at] = from.saturating_add(settles[at]);
        }
        let settle = outs
            .iter()
            .map(|&at| settle_out[at])
            .max()
            .unwrap_or(0)
            .min(u64::from(u32::MAX));

        // The strips themselves, their lanes found and their buffers taken.
        let mut strips = Vec::with_capacity(count);
        let mut lists = built.into_iter();
        for (at, source) in sources.iter().enumerate() {
            let meta = &metas[at];
            let layout = source.layout;
            let channels = layout.channels();
            let model = source.strip;
            let list = lists.next().unwrap_or_default();
            let after = i64::from(out(at));
            let strip_pan = pan_at(&model.pan);
            let mut edges = Vec::new();
            for &(dest, send, model_send) in &connections[at] {
                let delay = if source.uncompensated {
                    0
                } else {
                    arrival[dest].saturating_sub(out(at))
                };
                let pan = model_send.map_or(strip_pan, |send| pan_at(&send.pan));
                let to = sources[dest].layout;
                let (level, on, pre) = if let Some(send) = model_send {
                    (db_to_gain(single(send.level_db)), send.on, send.pre)
                } else {
                    let bus = &metas[at].edges[edges.len()].0;
                    let on = model.outputs.iter().any(|o| &o.bus == bus && o.on);
                    (1.0, on, false)
                };
                edges.push(Edge {
                    dest,
                    send,
                    pre,
                    level: smoothed(level, rate),
                    level_lane: None,
                    on: smoothed(switch(on), rate),
                    on_lane: None,
                    pan: Pan {
                        base: pan,
                        x: smoothed(pan.x, rate),
                        y: smoothed(pan.y, rate),
                        divergence: smoothed(pan.divergence, rate),
                        lanes: [None; 3],
                        last: pan,
                    },
                    from: layout,
                    to,
                    matrix: panned(layout, to, &pan),
                    delay: delay_line(channels, usize::try_from(delay).unwrap_or(0)),
                    stale: 0,
                });
            }
            let mut strip = Strip {
                layout,
                trim: smoothed(db_to_gain(single(model.trim_db)), rate),
                trim_lane: None,
                sign: smoothed(if model.invert { -1.0 } else { 1.0 }, rate),
                sign_lane: None,
                stages: list,
                fader: smoothed(db_to_gain(single(model.fader_db)), rate),
                fader_lane: None,
                mute: smoothed(switch(!source.mute), rate),
                mute_lane: None,
                audible: smoothed(switch(audible[at]), rate),
                vcas: members[at]
                    .iter()
                    .map(|&vca| VcaTerm {
                        vca,
                        fader: None,
                        mute: None,
                    })
                    .collect(),
                edges,
                lanes: Vec::new(),
                values: Box::new([]),
                max_block,
                heard: vec![0.0; max_block].into_boxed_slice(),
                gains: vec![0.0; max_block].into_boxed_slice(),
                levels: vec![0.0; max_block].into_boxed_slice(),
                work: Planar::new(channels, max_block),
                spare: Planar::new(channels, max_block),
                post: Planar::new(channels, max_block),
                pre_tap: tap(),
                post_tap: tap(),
            };
            // The lanes of the strip.
            let mut node_lanes: Vec<Vec<(ParamId, usize)>> = vec![Vec::new(); strip.stages.len()];
            for (path, lane) in source.lanes {
                let Some(param) = param_of(meta, path) else {
                    continue;
                };
                let delay = match param {
                    StripParam::Trim | StripParam::Invert => i64::from(arrival[at]),
                    // The switch and the amount of a Ducker act on the sound that leaves it, a
                    // lookahead after the sound that comes in (T11's review, P3-4).
                    StripParam::Node(stage, param)
                        if matches!(meta.stages[stage], StageKind::Ducker)
                            && [DuckerParam::On.id(), DuckerParam::AmountDb.id()]
                                .contains(&param) =>
                    {
                        let stage = &strip.stages[stage];
                        stage.offset + i64::from(stage.node.latency())
                    }
                    StripParam::Node(stage, _) => strip.stages[stage].offset,
                    _ => after,
                };
                let index = strip.lanes.len();
                strip.lanes.push(LaneOf {
                    lane: lane.clone(),
                    delay,
                });
                match param {
                    StripParam::Trim => strip.trim_lane = Some(index),
                    StripParam::Invert => strip.sign_lane = Some(index),
                    StripParam::Fader => strip.fader_lane = Some(index),
                    StripParam::Mute => strip.mute_lane = Some(index),
                    StripParam::Audible => {}
                    StripParam::PanX | StripParam::PanY | StripParam::PanDivergence => {
                        let which = match param {
                            StripParam::PanX => 0,
                            StripParam::PanY => 1,
                            _ => 2,
                        };
                        // A pan acts after the delay of its connection: each reads its lane that
                        // much later (T4b's review, P3-7).
                        let mut first = true;
                        for edge in strip.edges.iter_mut().filter(|edge| !edge.send) {
                            let mut at = index;
                            if !first {
                                at = strip.lanes.len();
                                strip.lanes.push(LaneOf {
                                    lane: lane.clone(),
                                    delay: 0,
                                });
                            }
                            first = false;
                            strip.lanes[at].delay =
                                after + i64::try_from(edge.delay.length()).unwrap_or(0);
                            edge.pan.lanes[which] = Some(at);
                        }
                    }
                    StripParam::Level(edge) => strip.edges[edge].level_lane = Some(index),
                    StripParam::On(edge) => strip.edges[edge].on_lane = Some(index),
                    StripParam::EdgePanX(edge) | StripParam::EdgePanY(edge) => {
                        let which = usize::from(matches!(param, StripParam::EdgePanY(_)));
                        let edge = &mut strip.edges[edge];
                        strip.lanes[index].delay =
                            after + i64::try_from(edge.delay.length()).unwrap_or(0);
                        edge.pan.lanes[which] = Some(index);
                    }
                    StripParam::Node(stage, param) => node_lanes[stage].push((param, index)),
                }
            }
            for (stage, lanes) in strip.stages.iter_mut().zip(node_lanes) {
                stage.lanes = lanes.into_boxed_slice();
            }
            // The lanes of its VCAs, read at its fader.
            for term in &mut strip.vcas {
                let vca = &t.vcas[term.vca];
                for (path, slot) in [("fader_db", &mut term.fader), ("mute", &mut term.mute)] {
                    if let Some(lane) = vca.lanes.get(path) {
                        *slot = Some(strip.lanes.len());
                        strip.lanes.push(LaneOf {
                            lane: lane.clone(),
                            delay: after,
                        });
                    }
                }
            }
            strip.values = vec![0.0; strip.lanes.len() * max_block].into_boxed_slice();
            strips.push(strip);
        }

        let mut keyed_sources = vec![false; count];
        for &source in keys.iter().flatten() {
            keyed_sources[source] = true;
        }
        let outputs: Vec<Out> = outs
            .iter()
            .map(|&at| Out {
                strip: at,
                bus: match &sources[at].owner {
                    Owner::Track(id) | Owner::Bus(id) | Owner::Vca(id) => id.clone(),
                },
                main: mains.contains(&at),
                buffer: Planar::new(sources[at].layout.channels(), max_block),
                delay: delay_line(
                    sources[at].layout.channels(),
                    usize::try_from(latency - out(at)).unwrap_or(0),
                ),
            })
            .collect();
        let monitor = monitor_strip.and_then(|strip| outs.iter().position(|&at| at == strip));
        let mut graph = Graph {
            id: GRAPHS.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            rate,
            max_block,
            purpose: options.purpose,
            owners: sources.iter().map(|source| source.owner.clone()).collect(),
            order,
            clips_of,
            clips: chains,
            sources: clip_sources,
            unplayable,
            scratch: ClipScratch {
                window: Window {
                    data: vec![0.0; MAX_CHANNELS * window].into_boxed_slice(),
                    capacity: window,
                },
                before: Planar::new(MAX_CHANNELS, max_block),
                after: Planar::new(MAX_CHANNELS, max_block),
                pools: stretching.pools(rate, max_block),
            },
            inputs: sources
                .iter()
                .map(|source| Planar::new(source.layout.channels(), max_block))
                .collect(),
            pres: sources
                .iter()
                .zip(&keyed_sources)
                .map(|(source, &keyed)| {
                    if keyed {
                        Planar::new(source.layout.channels(), max_block)
                    } else {
                        Planar::new(0, 0)
                    }
                })
                .collect(),
            keys: keyed_sources,
            duckers: ducker_meters
                .into_iter()
                .map(|(at, meter)| (sources[at].owner.clone(), meter))
                .collect(),
            edge: Planar::new(MAX_CHANNELS, max_block),
            vca_ids: t.vcas.iter().map(|vca| vca.id.clone()).collect(),
            vcas: t
                .vcas
                .iter()
                .map(|vca| VcaState {
                    fader: smoothed(db_to_gain(single(vca.fader_db)), rate),
                    mute: smoothed(switch(!vca.mute), rate),
                    gain: vec![0.0; max_block].into_boxed_slice(),
                    heard: vec![0.0; max_block].into_boxed_slice(),
                })
                .collect(),
            metas,
            strips,
            outputs,
            monitor,
            latency,
            settle: u32::try_from(settle).unwrap_or(u32::MAX),
            position: 0,
            frames: 0,
        };
        graph.reset(0);
        Ok(graph)
    }

    /// The setting of `owner` at `path`, the path of a lane of it (`fader_db`, `pan.x`,
    /// `send.B2.level_db`, `dynamics.compressor.threshold_db`, `eq.2.gain_db`, ...; a VCA's
    /// `fader_db` and `mute`), if the graph has it; and `solo`, whether a track or a bus is heard
    /// while others are soloed (1) or silenced (0), which [`Graph::set`] takes as the outcome of
    /// the solos, not as a solo button.
    #[must_use]
    pub fn control(&self, owner: &Owner, path: &str) -> Option<Control> {
        if let Owner::Vca(id) = owner {
            let vca = self.vca_ids.iter().position(|v| v == id)?;
            return match path {
                "fader_db" => Some(Control(self.id, Target::VcaFader(vca))),
                "mute" => Some(Control(self.id, Target::VcaMute(vca))),
                _ => None,
            };
        }
        let strip = self.owners.iter().position(|o| o == owner)?;
        let param = if path == "solo" {
            StripParam::Audible
        } else {
            param_of(&self.metas[strip], path)?
        };
        Some(Control(self.id, Target::Strip(strip, param)))
    }

    /// Its outputs: the main outputs, then the bus that is monitored if it is not one.
    #[must_use]
    pub fn outputs(&self) -> Vec<OutputInfo> {
        self.outputs
            .iter()
            .map(|out| OutputInfo {
                bus: out.bus.clone(),
                layout: self.strips[out.strip].layout,
                main: out.main,
            })
            .collect()
    }

    /// The levels of each track and bus.
    #[must_use]
    pub fn meters(&self) -> Vec<Meter> {
        self.owners
            .iter()
            .zip(&self.strips)
            .map(|(owner, strip)| Meter {
                owner: owner.clone(),
                channels: strip.channels(),
                pre: strip.pre_tap.reader.clone(),
                post: strip.post_tap.reader.clone(),
            })
            .collect()
    }
}

/// The stretches of the clips of a graph as they are made (T13): the library's configuration at
/// the rate of the timeline, asked once, and for each count of channels the spans over which
/// clips hold a stretcher and the most input that one is fed by a chunk.
#[derive(Default)]
struct Stretching {
    config: Option<columbina_stretch::Config>,
    pools: Vec<PoolOf>,
}

/// What a pool of stretchers is made for: its channels, the spans of the timeline over which its
/// clips hold a stretcher, and the most input a chunk is fed.
struct PoolOf {
    channels: usize,
    spans: Vec<(i64, i64)>,
    most: usize,
}

impl Stretching {
    /// The samples of a seek of the library (0 before any clip is stretched).
    fn seek_length(&self) -> usize {
        self.config.map_or(0, |config| config.seek_length)
    }

    /// The stretch of `clip`, which sounds as `piece` with `mapping` on a track of `channels` at
    /// `rate`, its filter reaching `radius`: Pitch Correction where its speed is not 1 (and at
    /// most [`STRETCH_FASTEST`]), else its pitch moved alone where it is moved, else none, as
    /// before (p. 3827, p. 3826).
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_possible_wrap,
        clippy::cast_sign_loss,
        reason = "a speed, and samples within the timeline"
    )]
    fn of(
        &mut self,
        clip: &columbina_model::Clip,
        piece: &resolve::AudioPiece,
        mapping: &Mapping,
        (rate, channels): (u32, usize),
        radius: i64,
    ) -> Option<Stretched> {
        let (num, den) = piece.speed;
        let speed = f64::from(num) / f64::from(den.max(1));
        let corrected = clip.pitch_correction && num != den && speed <= STRETCH_FASTEST;
        let moved = clip.pitch.semitones != 0 || clip.pitch.cents != 0.0;
        let mode = if corrected {
            StretchMode::Source
        } else if moved {
            StretchMode::Resampled
        } else {
            return None;
        };
        let config = if let Some(config) = self.config {
            config
        } else {
            let config = columbina_stretch::Config::of(rate).ok()?;
            self.config = Some(config);
            config
        };
        let (lin, lout, seek) = (
            config.input_latency as i64,
            config.output_latency as i64,
            config.seek_length as i64,
        );
        let warm = Stretcher::warm_of(&config);
        let fade = samples(STRETCH_RESET_CROSSFADE, rate) as i64;
        let grid = Grid::clip(piece.from, mapping.speed(), rate, fade, warm);
        let busy_from = piece.from - warm;
        let chunk = CHUNK as i64;
        let reach = (speed * (warm + chunk + lout + lin + seek) as f64).ceil() as i64
            + lin
            + seek
            + radius
            + 2;
        let max_input = match mode {
            StretchMode::Source => CHUNK * speed.ceil() as usize + 2,
            StretchMode::Resampled => CHUNK,
        };
        let pool = if let Some(at) = self.pools.iter().position(|p| p.channels == channels) {
            at
        } else {
            self.pools.push(PoolOf {
                channels,
                spans: Vec::new(),
                most: 0,
            });
            self.pools.len() - 1
        };
        let of = &mut self.pools[pool];
        of.spans.push((busy_from, piece.to));
        of.most = of.most.max(max_input);
        Some(Stretched {
            mode,
            plan: Plan {
                grid,
                segments: (0, grid.segment(piece.to - 1)),
                floor: i64::MIN,
                rate: match mode {
                    StretchMode::Source => speed,
                    StretchMode::Resampled => 1.0,
                },
            },
            transpose: pitch::factor(clip.pitch.semitones as f32, clip.pitch.cents as f32),
            latencies: (lin, lout),
            busy_from,
            reach,
            pool,
            held: None,
        })
    }

    /// The pools: for each count of channels, as many stretchers as the most of its clips that
    /// hold one at once, a block either side of each span (a clip takes its stretcher in the block
    /// where it starts warming, and gives it back in the block where it ends). A pool whose
    /// stretchers cannot be made has none, and its clips are silent.
    fn pools(&self, rate: u32, max_block: usize) -> Vec<Pool> {
        let block = i64::try_from(max_block).unwrap_or(i64::MAX);
        self.pools
            .iter()
            .map(
                |PoolOf {
                     channels,
                     spans,
                     most,
                 }| {
                    let mut events: Vec<(i64, i32)> = spans
                        .iter()
                        .flat_map(|&(from, to)| {
                            [
                                (from.saturating_sub(block), 1),
                                (to.saturating_add(block), -1),
                            ]
                        })
                        .collect();
                    events.sort_unstable();
                    let (mut now, mut size) = (0i32, 0i32);
                    for (_, change) in events {
                        now += change;
                        size = size.max(now);
                    }
                    let size = usize::try_from(size).unwrap_or(0);
                    let stretchers: Vec<Stretcher> = (0..size)
                        .map_while(|_| Stretcher::new(*channels, rate, *most).ok())
                        .collect();
                    let stretchers = if stretchers.len() == size {
                        stretchers
                    } else {
                        Vec::new()
                    };
                    let mut pool = Pool {
                        free: Vec::with_capacity(stretchers.len()),
                        stretchers,
                    };
                    pool.reset();
                    pool
                },
            )
            .collect()
    }
}
