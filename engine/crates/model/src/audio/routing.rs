//! The `FlexBus` of a timeline (Reference Manual 20, Ch. 171, 177): its buses, where tracks and
//! buses send their sound, its VCAs and its groups of tracks; and the checks that keep them a
//! mix that can be played: every bus, VCA and track that is named is there, no sound goes round
//! in a loop, and no bus or VCA is nested more than six deep (p. 3721).

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use super::fx::SourceRef;
use super::param::Lanes;
use super::{Pan, Strip};
use crate::ops::OpError;
use crate::project::{Id, Rgba, Timeline, TrackKind};

/// The most buses that a timeline has (C-B3).
pub const MOST_BUSES: usize = 64;
/// The most VCAs that a timeline has (C-B5; p. 3883).
pub const MOST_VCAS: usize = 64;
/// The most buses on a path of buses, each feeding the next, and the most VCAs on a chain of
/// VCAs (p. 3721).
pub const DEEPEST: usize = 6;
/// The most buses that a strip outputs to, and the most that it sends to (p. 3721).
pub const MOST_OUTPUTS: usize = 10;
/// See [`MOST_OUTPUTS`].
pub const MOST_SENDS: usize = 10;
/// The most inserts of a strip (p. 3855).
pub const MOST_INSERTS: usize = 6;

/// The format of a bus: how many channels it has, and which (pp. 3721, 3730). The channels of a
/// format of more than two are in the order of SMPTE: L R C LFE Ls Rs, and Lsr Rsr after them
/// for 7.1 (C-F8).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BusFormat {
    /// One channel.
    #[serde(rename = "mono")]
    Mono,
    /// L R.
    #[serde(rename = "stereo")]
    Stereo,
    /// L R C.
    #[serde(rename = "3.0")]
    Lcr,
    /// L R C S.
    #[serde(rename = "4.0")]
    Lcrs,
    /// L R C Ls Rs.
    #[serde(rename = "5.0")]
    Five,
    /// L R C LFE Ls Rs.
    #[serde(rename = "5.1")]
    FiveOne,
    /// L R C Ls Rs Lsr Rsr.
    #[serde(rename = "7.0")]
    Seven,
    /// L R C LFE Ls Rs Lsr Rsr.
    #[serde(rename = "7.1")]
    SevenOne,
}

impl BusFormat {
    /// How many channels it has.
    #[must_use]
    pub fn channels(self) -> u8 {
        match self {
            Self::Mono => 1,
            Self::Stereo => 2,
            Self::Lcr => 3,
            Self::Lcrs => 4,
            Self::Five => 5,
            Self::FiveOne => 6,
            Self::Seven => 7,
            Self::SevenOne => 8,
        }
    }
}

const fn yes() -> bool {
    true
}

/// Where a strip puts its sound after its fader and pan: a bus (p. 3732). A bus that no strip
/// outputs or sends to is a main output, which may be monitored and rendered (p. 3721).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Output {
    /// The bus.
    pub bus: Id,
    /// Whether it is on: a bypassed output sends nothing. It may be left out, as on.
    #[serde(default = "yes")]
    pub on: bool,
}

/// A send of a strip to a bus, before or after its fader, at a level and a pan of its own
/// (pp. 3869–3871).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Send {
    /// The bus.
    pub bus: Id,
    /// Whether it is on (its mute, p. 3869): on for a new one.
    #[serde(default = "yes")]
    pub on: bool,
    /// Whether it is taken before the fader, after the mute (p. 3870): after, for a new one.
    #[serde(default)]
    pub pre: bool,
    /// Its level in decibels, at most 10; -100 or below is OFF. 0 for a new one.
    #[serde(default)]
    pub level_db: f64,
    /// Where it is put among the channels of the bus, apart from the strip's own pan.
    #[serde(default)]
    pub pan: Pan,
}

/// A bus of a timeline (pp. 3721, 3730): a strip of its own (with no Track FX), fed by the
/// tracks and buses that output or send to it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bus {
    /// What it is known by, among the buses of its timeline. The engine names the buses it makes
    /// `B1`, `B2` and so on.
    pub id: Id,
    /// The name that it is shown by (`Bus 1`).
    pub name: String,
    /// Its colour, as its strip and the sends to it are shown (p. 3871).
    pub color: Rgba,
    /// Its format.
    pub format: BusFormat,
    /// Its strip, whose outputs are the buses that it feeds. It may be left out, as a strip at
    /// unity that outputs to no bus.
    #[serde(default)]
    pub strip: Strip,
    /// Whether it is muted (p. 3885): what goes through it is not heard nor rendered.
    #[serde(default)]
    pub mute: bool,
    /// Whether it is soloed: while any track or bus is, only what feeds a soloed bus, a soloed
    /// track, and what is solo-safe is heard.
    #[serde(default)]
    pub solo: bool,
    /// Whether it is heard while others are soloed (Solo Safe), as a bus of returns is.
    #[serde(default)]
    pub solo_safe: bool,
    /// The automation of its settings. It may be left out, as none.
    #[serde(default)]
    pub lanes: Lanes,
}

/// The id of the bus that a new timeline has, which every track outputs to.
pub const FIRST_BUS: &str = "B1";

impl Bus {
    /// The bus that a new timeline has (C-B7, p. 3721): `B1`, "Bus 1", stereo.
    #[must_use]
    pub fn first() -> Self {
        Self {
            id: FIRST_BUS.into(),
            name: "Bus 1".into(),
            color: Rgba([0x5a, 0xa0, 0xe0, 0xff]),
            format: BusFormat::Stereo,
            strip: Strip::default(),
            mute: false,
            solo: false,
            solo_safe: false,
            lanes: Lanes::new(),
        }
    }
}

/// A VCA of a timeline (pp. 3882–3884): a fader that moves the faders of its members, which name
/// it in their `vcas`, by adding its decibels to theirs. A VCA may be a member of another.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Vca {
    /// What it is known by, among the VCAs of its timeline (`VCA1`, ...).
    pub id: Id,
    /// The name that it is shown by.
    pub name: String,
    /// Its fader, in decibels, at most 10; -100 or below is -∞.
    #[serde(default)]
    pub fader_db: f64,
    /// Whether it mutes its members.
    #[serde(default)]
    pub mute: bool,
    /// Whether it solos its members.
    #[serde(default)]
    pub solo: bool,
    /// The VCAs that it is a member of.
    #[serde(default)]
    pub vcas: Vec<Id>,
    /// The automation of its fader and mute (p. 3884).
    #[serde(default)]
    pub lanes: Lanes,
}

/// What the tracks of a group do together (p. 3919). Arm is not here: the studio records no
/// sound.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupControl {
    /// Edits of the timeline.
    Editing,
    /// Faders, by the same change of decibels.
    Fader,
    /// Solo.
    Solo,
    /// Mute.
    Mute,
    /// Send levels.
    Sends,
    /// Pans.
    Panning,
    /// Automation.
    Automation,
}

impl GroupControl {
    /// All of them, as a new group has them (§4.2).
    pub const ALL: [GroupControl; 7] = [
        Self::Editing,
        Self::Fader,
        Self::Solo,
        Self::Mute,
        Self::Sends,
        Self::Panning,
        Self::Automation,
    ];
}

fn all_controls() -> Vec<GroupControl> {
    GroupControl::ALL.to_vec()
}

/// A group of tracks of sound of a timeline (Ch. 179, pp. 3916–3921): changes to one of them, of
/// what is `linked`, are made to all while it is active.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrackGroup {
    /// What it is known by, among the groups of its timeline (`TG1`, ...).
    pub id: Id,
    /// The name that it is shown by.
    pub name: String,
    /// Its colour.
    #[serde(default = "group_colour")]
    pub color: Rgba,
    /// Its tracks of sound.
    pub members: Vec<Id>,
    /// What its tracks do together: all of it, where it is left out.
    #[serde(default = "all_controls")]
    pub linked: Vec<GroupControl>,
    /// Whether it is active: active, where it is left out.
    #[serde(default = "yes")]
    pub active: bool,
}

fn group_colour() -> Rgba {
    Rgba([0xe0, 0x8a, 0x3a, 0xff])
}

/// What names a track of a timeline, where something does: a group of tracks that it is a
/// member of, or a track or a bus whose sidechain or Ducker it drives.
#[must_use]
pub fn user_of_track(timeline: &Timeline, id: &str) -> Option<Id> {
    if let Some(group) = timeline
        .track_groups
        .iter()
        .find(|g| g.members.iter().any(|m| m == id))
    {
        return Some(group.id.clone());
    }
    let named = |strip: &Strip| {
        strip
            .track_fx
            .iter()
            .flat_map(|ducker| &ducker.sources)
            .chain(strip.dynamics.compressor.sidechain.source.iter())
            .chain(
                strip
                    .inserts
                    .iter()
                    .filter_map(|insert| insert.effect.sidechain())
                    .filter_map(|sidechain| sidechain.source.as_ref()),
            )
            .any(|source| matches!(source, SourceRef::Track(track) if track == id))
    };
    timeline
        .tracks
        .iter()
        .filter(|r| r.kind == TrackKind::Audio)
        .map(|r| (&r.id, &r.audio.strip))
        .chain(timeline.buses.iter().map(|b| (&b.id, &b.strip)))
        .find(|(_, strip)| named(strip))
        .map(|(by, _)| by.clone())
}

/// Check that a list names each thing once, and each by an id that is there and does not start
/// with `$`.
fn ids<'a>(
    what: &'static str,
    list: impl Iterator<Item = &'a Id>,
) -> Result<HashSet<&'a str>, (Id, OpError)> {
    let mut seen = HashSet::new();
    for id in list {
        let error = if id.is_empty() {
            OpError::NoId
        } else if id.starts_with('$') {
            OpError::Dollar { id: id.clone() }
        } else if !seen.insert(id.as_str()) {
            OpError::Taken {
                what,
                id: id.clone(),
            }
        } else {
            continue;
        };
        return Err((id.clone(), error));
    }
    Ok(seen)
}

/// A strip of a timeline: of a track or of a bus.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Node<'a> {
    Track(&'a str),
    Bus(&'a str),
}

impl Node<'_> {
    fn id(self) -> Id {
        match self {
            Self::Track(id) | Self::Bus(id) => id.into(),
        }
    }
}

/// Why the routing of a timeline is refused: what is wrong (`track`, `bus`, `vca`,
/// `track_group` or `timeline`), its id, and the error.
pub type Wrong = (&'static str, Id, OpError);

/// Check the routing of a timeline whose strips are checked: the ids of its buses, VCAs and
/// groups, and that every bus, VCA, track and group that is named is there; that no sound goes
/// round in a loop, through outputs, sends, sidechains and duckers; that no path of buses is more
/// than [`DEEPEST`] buses long, nor a chain of VCAs more than [`DEEPEST`] VCAs; that the bus
/// monitored is there; and that there are at most [`MOST_BUSES`] buses and [`MOST_VCAS`] VCAs.
///
/// # Errors
/// What is wrong, as [`Wrong`].
#[allow(clippy::too_many_lines, reason = "one check after another")]
pub fn check_routing(timeline: &Timeline) -> Result<(), Wrong> {
    let at = |what: &'static str| move |(id, error): (Id, OpError)| (what, id, error);
    let whole = |error| ("timeline", timeline.id.clone(), error);
    if timeline.buses.len() > MOST_BUSES {
        return Err(whole(OpError::OutOfRange { what: "buses" }));
    }
    if timeline.vcas.len() > MOST_VCAS {
        return Err(whole(OpError::OutOfRange { what: "vcas" }));
    }
    let buses = ids("bus", timeline.buses.iter().map(|b| &b.id)).map_err(at("bus"))?;
    let vcas = ids("vca", timeline.vcas.iter().map(|v| &v.id)).map_err(at("vca"))?;
    ids("track_group", timeline.track_groups.iter().map(|g| &g.id)).map_err(at("track_group"))?;
    let tracks: HashSet<&str> = timeline
        .tracks
        .iter()
        .filter(|r| r.kind == TrackKind::Audio)
        .map(|r| r.id.as_str())
        .collect();
    let strips = timeline
        .tracks
        .iter()
        .filter(|r| r.kind == TrackKind::Audio)
        .map(|r| (Node::Track(&r.id), &r.audio.strip))
        .chain(timeline.buses.iter().map(|b| (Node::Bus(&b.id), &b.strip)));
    // Each strip, what feeds it (by sidechains and duckers) and what it feeds.
    let mut feeds: HashMap<Node, Vec<Node>> = HashMap::new();
    for (node, strip) in strips {
        let what = match node {
            Node::Track(_) => "track",
            Node::Bus(_) => "bus",
        };
        let wrong = |error| (what, node.id(), error);
        let bus = |id: &Id| {
            if buses.contains(id.as_str()) {
                Ok(())
            } else {
                Err(wrong(OpError::UnknownBus { id: id.clone() }))
            }
        };
        for output in &strip.outputs {
            bus(&output.bus)?;
            feeds.entry(node).or_default().push(Node::Bus(&output.bus));
        }
        for send in &strip.sends {
            bus(&send.bus)?;
            feeds.entry(node).or_default().push(Node::Bus(&send.bus));
        }
        let sidechains = strip
            .track_fx
            .iter()
            .flat_map(|ducker| &ducker.sources)
            .chain(strip.dynamics.compressor.sidechain.source.iter())
            .chain(
                strip
                    .inserts
                    .iter()
                    .filter_map(|insert| insert.effect.sidechain())
                    .filter_map(|sidechain| sidechain.source.as_ref()),
            );
        for driving in sidechains {
            let from = match driving {
                SourceRef::Track(id) if tracks.contains(id.as_str()) => Node::Track(id),
                SourceRef::Track(id) => {
                    return Err(wrong(OpError::Missing {
                        what: "track",
                        id: id.clone(),
                    }))
                }
                SourceRef::Bus(id) => {
                    bus(id)?;
                    Node::Bus(id)
                }
            };
            if from == node {
                return Err(wrong(OpError::BusCycle { id: node.id() }));
            }
            feeds.entry(from).or_default().push(node);
        }
        for vca in &strip.vcas {
            if !vcas.contains(vca.as_str()) {
                return Err(wrong(OpError::UnknownVca { id: vca.clone() }));
            }
        }
    }
    if let Some(node) = in_a_loop(&feeds) {
        let what = match node {
            Node::Track(_) => "track",
            Node::Bus(_) => "bus",
        };
        return Err((what, node.id(), OpError::BusCycle { id: node.id() }));
    }
    // The longest path of buses that ends at each bus, by outputs and sends only.
    let mut into: HashMap<&str, Vec<&str>> = HashMap::new();
    for bus in &timeline.buses {
        for target in bus
            .strip
            .outputs
            .iter()
            .map(|o| &o.bus)
            .chain(bus.strip.sends.iter().map(|s| &s.bus))
        {
            into.entry(target.as_str()).or_default().push(&bus.id);
        }
    }
    let mut depths = HashMap::new();
    for bus in &timeline.buses {
        if depth(&bus.id, &into, &mut depths) > DEEPEST {
            return Err((
                "bus",
                bus.id.clone(),
                OpError::BusDepth { id: bus.id.clone() },
            ));
        }
    }
    // The VCAs: what each is a member of is there, with no loop and no chain too long.
    let mut members: HashMap<&str, Vec<&str>> = HashMap::new();
    for vca in &timeline.vcas {
        for of in &vca.vcas {
            if !vcas.contains(of.as_str()) {
                return Err((
                    "vca",
                    vca.id.clone(),
                    OpError::UnknownVca { id: of.clone() },
                ));
            }
            members.entry(of.as_str()).or_default().push(&vca.id);
        }
    }
    let vca_feeds: HashMap<Node, Vec<Node>> = timeline
        .vcas
        .iter()
        .map(|v| {
            (
                Node::Bus(&v.id),
                v.vcas.iter().map(|of| Node::Bus(of)).collect(),
            )
        })
        .collect();
    if let Some(node) = in_a_loop(&vca_feeds) {
        return Err(("vca", node.id(), OpError::VcaCycle { id: node.id() }));
    }
    let mut depths = HashMap::new();
    for vca in &timeline.vcas {
        if depth(&vca.id, &members, &mut depths) > DEEPEST {
            return Err((
                "vca",
                vca.id.clone(),
                OpError::VcaDepth { id: vca.id.clone() },
            ));
        }
    }
    for group in &timeline.track_groups {
        let wrong = |error| ("track_group", group.id.clone(), error);
        let mut seen = HashSet::new();
        for member in &group.members {
            if !tracks.contains(member.as_str()) {
                return Err(wrong(OpError::Missing {
                    what: "track",
                    id: member.clone(),
                }));
            }
            if !seen.insert(member) {
                return Err(wrong(OpError::OutOfRange { what: "members" }));
            }
        }
        let linked: HashSet<_> = group.linked.iter().collect();
        if linked.len() != group.linked.len() {
            return Err(wrong(OpError::OutOfRange { what: "linked" }));
        }
    }
    if let Some(monitor) = &timeline.audio.monitor {
        if !buses.contains(monitor.as_str()) {
            return Err(whole(OpError::UnknownBus {
                id: monitor.clone(),
            }));
        }
    }
    Ok(())
}

/// How many buses long the longest path is that ends at `bus`, `bus` itself counted, where
/// `into` lists the buses that feed each, which go round in no loop.
fn depth<'a>(
    bus: &'a str,
    into: &HashMap<&'a str, Vec<&'a str>>,
    depths: &mut HashMap<&'a str, usize>,
) -> usize {
    if let Some(&known) = depths.get(bus) {
        return known;
    }
    let mut deepest = 0;
    for &from in into.get(bus).into_iter().flatten() {
        deepest = deepest.max(depth(from, into, depths));
    }
    depths.insert(bus, deepest + 1);
    deepest + 1
}

/// A node on a loop of `feeds`, if there is one: the first one found going round.
fn in_a_loop<'a>(feeds: &HashMap<Node<'a>, Vec<Node<'a>>>) -> Option<Node<'a>> {
    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        Open,
        Done,
    }
    fn visit<'a>(
        node: Node<'a>,
        feeds: &HashMap<Node<'a>, Vec<Node<'a>>>,
        marks: &mut HashMap<Node<'a>, Mark>,
    ) -> Option<Node<'a>> {
        match marks.get(&node) {
            Some(Mark::Open) => return Some(node),
            Some(Mark::Done) => return None,
            None => {}
        }
        marks.insert(node, Mark::Open);
        for &next in feeds.get(&node).into_iter().flatten() {
            if let Some(found) = visit(next, feeds, marks) {
                return Some(found);
            }
        }
        marks.insert(node, Mark::Done);
        None
    }
    let mut marks = HashMap::new();
    let mut nodes: Vec<_> = feeds.keys().copied().collect();
    // In a set order, so that the same loop is always named by the same node.
    nodes.sort_by_key(|n| match n {
        Node::Track(id) => (0, *id),
        Node::Bus(id) => (1, *id),
    });
    nodes
        .into_iter()
        .find_map(|node| visit(node, feeds, &mut marks))
}
