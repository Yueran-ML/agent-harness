//! Names that live for one transaction.
//!
//! A client that adds a thing does not know the id that the engine will give it, and so could
//! not name it in a later operation of the same transaction. Instead it may give the thing an id
//! of the form `$name` (a `$` and 1 to 32 of `A`-`Z`, `a`-`z`, `0`-`9` and `_`): the thing is
//! added as if its id were empty, and given one by the engine, and every later operation of the
//! transaction that names `$name` where it names a thing is made to name that id before it is
//! applied. The ids that the names were given are answered in `names`.
//!
//! An id that starts with `$` is always such a name, so no thing is ever given one. A name is
//! given to one thing only, and may be named only after the operation that gives it.
//!
//! The id of a track names it only among the tracks of its timeline, so a name of a track is
//! named only with its timeline: an operation that names it must name the timeline as the
//! operation that gave it did, by the same name or the same id.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::convert::Infallible;
use std::fmt;

use columbina_model::edit::Created;
use columbina_model::history::ChangeMapper;
use columbina_model::{
    Change, Edit, Expansion, Grade, Id, InsertsOf, LanesOf, Link, Material, Op, OpError, Source,
    Target, Track, TrackKind,
};
use columbina_model::{Effect, Insert, SourceRef, Strip};
use serde::{Deserialize, Deserializer};
use serde_json::Value;

/// One wire change, with optional bindings for the outputs of a creating edit.
#[derive(Debug)]
pub struct Input {
    pub change: Change,
    bindings: Vec<(Id, Output)>,
}

/// A selector of an edit's intentional output, never an incidental split.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Output {
    Primary,
    Clip,
    Video,
    Audio,
    Link,
    Transition,
    Cue,
}

impl Output {
    fn id(self, created: &[Created]) -> Option<&str> {
        created.iter().find_map(|item| match (self, item) {
            (Self::Primary | Self::Clip, Created::Clip { id, .. })
            | (
                Self::Video,
                Created::Clip {
                    id,
                    kind: TrackKind::Video,
                    ..
                },
            )
            | (
                Self::Audio,
                Created::Clip {
                    id,
                    kind: TrackKind::Audio,
                    ..
                },
            )
            | (Self::Primary | Self::Transition, Created::Transition(id))
            | (Self::Primary | Self::Cue, Created::Cue(id)) => Some(id.as_str()),
            (Self::Link, Created::Clip { link, .. }) => link.as_deref(),
            _ => None,
        })
    }
}

impl<'de> Deserialize<'de> for Input {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut value = serde_json::Value::deserialize(deserializer)?;
        let mut bindings = vec![];
        // add_track.name remains its display name. Only these creating commands bind outputs.
        if matches!(
            value.get("edit").and_then(serde_json::Value::as_str),
            Some(
                "insert"
                    | "overwrite"
                    | "place_on_top"
                    | "append"
                    | "smart_insert"
                    | "ripple_overwrite"
                    | "add_transition"
                    | "add_cue"
            )
        ) {
            if let Some(name) = value.as_object_mut().and_then(|v| v.remove("name")) {
                match name {
                    serde_json::Value::String(name) => bindings.push((name, Output::Primary)),
                    serde_json::Value::Object(names) if !names.is_empty() => {
                        for (selector, name) in names {
                            let output =
                                serde_json::from_value(serde_json::Value::String(selector))
                                    .map_err(serde::de::Error::custom)?;
                            let name =
                                serde_json::from_value(name).map_err(serde::de::Error::custom)?;
                            bindings.push((name, output));
                        }
                    }
                    _ => {
                        return Err(serde::de::Error::custom(
                            "name must be a transaction name or a nonempty object of output names",
                        ))
                    }
                }
            }
        }
        let change = serde_json::from_value(value).map_err(serde::de::Error::custom)?;
        Ok(Self { change, bindings })
    }
}

impl From<Change> for Input {
    /// A change that the engine made, which binds no names.
    fn from(change: Change) -> Self {
        Self {
            change,
            bindings: vec![],
        }
    }
}

/// The most characters of a name after its `$`.
const LONGEST: usize = 32;

/// Why the names of a transaction cannot be used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NameError {
    /// The place of the operation in the transaction.
    pub index: usize,
    /// The name, as it was written.
    pub name: String,
    /// What is wrong with it: `invalid_name`, `unknown_name`, `duplicate_name` or
    /// `name_elsewhere`.
    pub reason: &'static str,
}

impl fmt::Display for NameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self { index, name, .. } = self;
        match self.reason {
            "invalid_name" => write!(
                f,
                "operation {index} names {name}, which is not a name of a transaction: a $ and \
                 1 to {LONGEST} letters, digits or _"
            ),
            "unknown_name" => write!(
                f,
                "operation {index} names {name}, which no operation before it in the \
                 transaction gives to a thing"
            ),
            "name_elsewhere" => write!(
                f,
                "operation {index} names the track {name} in another timeline than the one that \
                 it was given in"
            ),
            _ => write!(
                f,
                "operation {index} gives the name {name}, which an operation before it gives \
                 already"
            ),
        }
    }
}

/// Where a thing that is given a name is, in the operation that adds it.
#[derive(Clone, Debug)]
enum Place {
    Media,
    Bin,
    Timeline,
    /// A track: the one that is added, or the one at this place in a timeline that is added.
    Track(Option<usize>),
    /// A clip: the one that is put on a track, or the one that starts at this frame on the
    /// track that is added, or on the track at this place in a timeline that is added. Clips
    /// on a track cover no frame in common, so no two start at one frame.
    Clip(Option<usize>, i64),
    Link(Option<usize>, i64),
    Transition(Option<usize>, Id),
    Cue(Option<usize>, i64),
    /// A node that is added to a grade.
    Node,
    /// A group of clips graded together.
    Group,
    Output(Output),
}

/// What an id of an operation is to it.
#[derive(Clone, Debug)]
enum Role {
    /// It names a thing that is there, other than a track.
    Names,
    /// It names a track of the timeline that the operation names, as it is written there.
    NamesTrack(Id),
    /// It is the id of a thing that the operation adds, other than a track.
    Gives(Place),
    /// It is the id of a track that the operation adds, to the timeline written so; none for
    /// a timeline that is added without an id, which no later operation can name.
    GivesTrack(Place, Option<Id>),
    /// An existing link group, or a newly named one on its first use.
    Link(Place),
}

/// The names of a transaction, and the ids that the engine gave them as it was applied.
#[derive(Debug, Default)]
pub struct Names {
    /// For each operation, the names that it gives and where.
    given: Vec<Vec<(String, Place)>>,
    /// The ids of the names of the operations that have been applied.
    ids: BTreeMap<String, Id>,
    /// How many input changes have been mapped, independently of their expanded operations.
    applied: usize,
}

/// Check the names of a transaction, and make each thing that is given one have an empty id,
/// for the engine to give it one.
///
/// # Errors
/// A name is not of the form of a name, is named before it is given, or is given twice.
pub fn prepare(ops: &mut [Input]) -> Result<Names, NameError> {
    let mut known = HashSet::new();
    // The timeline of each track that is given a name, as it is written.
    let mut tracks: HashMap<String, Option<Id>> = HashMap::new();
    let mut given = Vec::with_capacity(ops.len());
    for (index, op) in ops.iter_mut().enumerate() {
        let mut here = Vec::new();
        let mut check = |id: &mut Id, role| {
            if !id.starts_with('$') {
                return Ok(());
            }
            let error = |reason| NameError {
                index,
                name: id.clone(),
                reason,
            };
            if !is_name(id) {
                return Err(error("invalid_name"));
            }
            let role = match role {
                Role::Link(_) if known.contains(id.as_str()) => Role::Names,
                Role::Link(place) => Role::Gives(place),
                role => role,
            };
            match role {
                Role::Names | Role::NamesTrack(_) if here.iter().any(|(name, _)| name == id) => {
                    Err(error("unknown_name"))
                }
                Role::Names | Role::NamesTrack(_) if !known.contains(id.as_str()) => {
                    Err(error("unknown_name"))
                }
                Role::NamesTrack(of) => match tracks.get(id.as_str()) {
                    Some(given_in) if given_in.as_deref() != Some(of.as_str()) => {
                        Err(error("name_elsewhere"))
                    }
                    _ => Ok(()),
                },
                Role::Names => Ok(()),
                Role::Gives(_) | Role::GivesTrack(..) if known.contains(id.as_str()) => {
                    Err(error("duplicate_name"))
                }
                Role::Gives(place) => {
                    known.insert(id.clone());
                    here.push((std::mem::take(id), place));
                    Ok(())
                }
                Role::GivesTrack(place, of) => {
                    known.insert(id.clone());
                    tracks.insert(id.clone(), of);
                    here.push((std::mem::take(id), place));
                    Ok(())
                }
                Role::Link(_) => unreachable!("link role was resolved above"),
            }
        };
        visit_change(&mut op.change, &mut check)?;
        for (name, output) in &mut op.bindings {
            if !is_name(name) {
                return Err(NameError {
                    index,
                    name: name.clone(),
                    reason: "invalid_name",
                });
            }
            check(name, Role::Gives(Place::Output(*output)))?;
        }
        given.push(here);
    }
    Ok(Names {
        given,
        ..Names::default()
    })
}

impl ChangeMapper for Names {
    /// Make an operation name the ids that the names it names were given, from the operations
    /// before it as they were done. It is the `map` of
    /// [`History::change`](columbina_model::History::change).
    fn map(&mut self, mut op: Change, _done: &[Op]) -> Change {
        let ids = &self.ids;
        let _ = visit_change(&mut op, &mut |id, role| {
            if let (Role::Names | Role::NamesTrack(_) | Role::Link(_), Some(given)) =
                (role, ids.get(id.as_str()))
            {
                id.clone_from(given);
            }
            Ok::<(), Infallible>(())
        });
        op
    }

    fn applied(&mut self, expansion: &Expansion) -> Result<(), OpError> {
        for (name, place) in &self.given[self.applied] {
            let id = match place {
                Place::Output(output) => output.id(&expansion.created),
                place => expansion.done.first().and_then(|op| id_at(op, place)),
            }
            .ok_or_else(|| OpError::Missing {
                what: "named output",
                id: name.clone(),
            })?;
            self.ids.insert(name.clone(), id.to_owned());
        }
        self.applied += 1;
        Ok(())
    }
}

impl Names {
    /// The concrete ids bound by this transaction, after it has succeeded.
    pub fn finish(self) -> BTreeMap<String, Id> {
        self.ids
    }
}

/// Whether an id is a name of a transaction.
fn is_name(id: &str) -> bool {
    id.strip_prefix('$').is_some_and(|rest| {
        (1..=LONGEST).contains(&rest.len())
            && rest.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
    })
}

fn visit_change<E>(
    change: &mut Change,
    f: &mut impl FnMut(&mut Id, Role) -> Result<(), E>,
) -> Result<(), E> {
    match change {
        Change::Op(op) => visit(op, f),
        Change::Edit(edit) => visit_edit(edit, f),
    }
}

/// Only id fields are visited; text, display names and generated titles may contain `$`.
#[allow(
    clippy::too_many_lines,
    reason = "the id fields of every edit, exhaustively"
)]
fn visit_edit<E>(
    edit: &mut Edit,
    f: &mut impl FnMut(&mut Id, Role) -> Result<(), E>,
) -> Result<(), E> {
    let timeline = match edit {
        // The edits of grades name grades and nodes, and no timeline of their own.
        // The windows of `add_serial` have ids of their own node, not of the project.
        Edit::AddSerial { grade, node, .. }
        | Edit::AddSerialBefore { grade, node }
        | Edit::AddParallel { grade, node }
        | Edit::AddLayer { grade, node }
        | Edit::AddOutside { grade, node }
        | Edit::ExtractNode { grade, node }
        | Edit::ResetNode { grade, node } => {
            visit_target(grade, f)?;
            return f(node, Role::Names);
        }
        Edit::AppendNode { grade } => return visit_target(grade, f),
        Edit::CopyGrade { from, to, .. } | Edit::AppendNodeGraph { from, to, .. } => {
            visit_target(from, f)?;
            for target in to {
                visit_target(target, f)?;
            }
            return Ok(());
        }
        Edit::CopyGivenGrade { to, .. } | Edit::AppendGivenGraph { to, .. } => {
            for target in to {
                visit_target(target, f)?;
            }
            return Ok(());
        }
        Edit::CollapseGroupGrades { timeline, clips } => {
            f(timeline, Role::Names)?;
            for clip in clips {
                f(clip, Role::Names)?;
            }
            return Ok(());
        }
        Edit::PasteValue {
            from,
            from_node,
            to,
            to_node,
            ..
        }
        | Edit::PasteNode {
            from,
            from_node,
            to,
            to_node,
        } => {
            visit_target(from, f)?;
            f(from_node, Role::Names)?;
            visit_target(to, f)?;
            return f(to_node, Role::Names);
        }
        Edit::Insert { timeline, .. }
        | Edit::Overwrite { timeline, .. }
        | Edit::Replace { timeline, .. }
        | Edit::PlaceOnTop { timeline, .. }
        | Edit::Append { timeline, .. }
        | Edit::SmartInsert { timeline, .. }
        | Edit::RippleOverwrite { timeline, .. }
        | Edit::Lift { timeline, .. }
        | Edit::RippleDelete { timeline, .. }
        | Edit::RippleDeleteRange { timeline, .. }
        | Edit::CloseGap { timeline, .. }
        | Edit::Blade { timeline, .. }
        | Edit::Move { timeline, .. }
        | Edit::InsertMove { timeline, .. }
        | Edit::PlaceCopies { timeline, .. }
        | Edit::Trim { timeline, .. }
        | Edit::Roll { timeline, .. }
        | Edit::Slip { timeline, .. }
        | Edit::Slide { timeline, .. }
        | Edit::AddTransition { timeline, .. }
        | Edit::RemoveTransition { timeline, .. }
        | Edit::SetTransition { timeline, .. }
        | Edit::SetSpeed { timeline, .. }
        | Edit::AddCue { timeline, .. }
        | Edit::SetCue { timeline, .. }
        | Edit::RemoveCue { timeline, .. }
        | Edit::CuesFrom { timeline, .. }
        | Edit::AddTrack { timeline, .. }
        | Edit::RemoveTrack { timeline, .. }
        | Edit::SetTrackAudio { timeline, .. }
        | Edit::ClipGainKeys { timeline, .. }
        | Edit::ClipGainRange { timeline, .. }
        | Edit::NudgeClipGain { timeline, .. }
        | Edit::PasteAttributes { timeline, .. }
        | Edit::RemoveAttributes { timeline, .. }
        | Edit::FadeToPlayhead { timeline, .. }
        | Edit::SetBus { timeline, .. }
        | Edit::CopyEffects { timeline, .. }
        | Edit::SetInsert { timeline, .. }
        | Edit::MoveInsert { timeline, .. }
        | Edit::LanePoints { timeline, .. }
        | Edit::LaneRange { timeline, .. }
        | Edit::LaneDraw { timeline, .. }
        | Edit::RecordPass { timeline, .. }
        | Edit::ClearLanes { timeline, .. } => timeline,
    };
    let of = timeline.clone();
    f(timeline, Role::Names)?;
    match edit {
        Edit::Insert {
            source,
            video_track,
            audio_track,
            ..
        }
        | Edit::Overwrite {
            source,
            video_track,
            audio_track,
            ..
        }
        | Edit::Replace {
            source,
            video_track,
            audio_track,
            ..
        }
        | Edit::Append {
            source,
            video_track,
            audio_track,
            ..
        }
        | Edit::SmartInsert {
            source,
            video_track,
            audio_track,
            ..
        }
        | Edit::RippleOverwrite {
            source,
            video_track,
            audio_track,
            ..
        } => {
            if let Material::Media { media, .. } = source {
                f(media, Role::Names)?;
            }
            for id in [video_track, audio_track]
                .into_iter()
                .filter_map(|x| x.as_mut().and_then(Option::as_mut))
            {
                f(id, Role::NamesTrack(of.clone()))?;
            }
        }
        Edit::PlaceOnTop { source, .. } => {
            if let Material::Media { media, .. } = source {
                f(media, Role::Names)?;
            }
        }
        Edit::Lift { clips, .. }
        | Edit::RippleDelete { clips, .. }
        | Edit::InsertMove { clips, .. }
        | Edit::NudgeClipGain { clips, .. }
        | Edit::RemoveAttributes { clips, .. }
        | Edit::FadeToPlayhead { clips, .. } => {
            for id in clips {
                f(id, Role::Names)?;
            }
        }
        Edit::Move {
            clips, to_track, ..
        } => {
            for id in clips {
                f(id, Role::Names)?;
            }
            if let Some(id) = to_track {
                f(id, Role::NamesTrack(of))?;
            }
        }
        Edit::PlaceCopies { copies, .. } => {
            for copy in copies {
                f(&mut copy.clip, Role::Names)?;
                f(&mut copy.track, Role::NamesTrack(of.clone()))?;
            }
        }
        Edit::RippleDeleteRange { tracks, .. } | Edit::Blade { tracks, .. } => {
            for id in tracks.iter_mut().flatten() {
                f(id, Role::NamesTrack(of.clone()))?;
            }
        }
        Edit::CloseGap { track, .. }
        | Edit::AddCue { track, .. }
        | Edit::CuesFrom { track, .. }
        | Edit::RemoveTrack { track, .. } => f(track, Role::NamesTrack(of))?,
        Edit::SetTrackAudio { track, patch, .. } => {
            f(track, Role::NamesTrack(of.clone()))?;
            visit_sources(patch, &of, f)?;
        }
        Edit::Trim { clip, .. }
        | Edit::Slip { clip, .. }
        | Edit::Slide { clip, .. }
        | Edit::SetSpeed { clip, .. }
        | Edit::ClipGainKeys { clip, .. }
        | Edit::ClipGainRange { clip, .. } => f(clip, Role::Names)?,
        Edit::Roll { left, right, .. } | Edit::AddTransition { left, right, .. } => {
            f(left, Role::Names)?;
            f(right, Role::Names)?;
        }
        Edit::RemoveTransition { id, .. }
        | Edit::SetTransition { id, .. }
        | Edit::SetCue { id, .. }
        | Edit::RemoveCue { id, .. } => f(id, Role::Names)?,
        Edit::AddTrack { .. } => {}
        Edit::SetBus { patch, .. } => visit_sources(patch, &of, f)?,
        Edit::PasteAttributes { from, to, .. } => {
            f(from, Role::Names)?;
            for id in to {
                f(id, Role::Names)?;
            }
        }
        Edit::CopyEffects { from, to, .. } => {
            visit_inserts_of(from, &of, f)?;
            for chain in to {
                visit_inserts_of(chain, &of, f)?;
            }
        }
        Edit::SetInsert {
            of: chain, insert, ..
        } => {
            visit_inserts_of(chain, &of, f)?;
            if let Some(insert) = insert {
                visit_insert(insert, &of, f)?;
            }
        }
        Edit::MoveInsert {
            of: chain, to_of, ..
        } => {
            visit_inserts_of(chain, &of, f)?;
            if let Some(chain) = to_of {
                visit_inserts_of(chain, &of, f)?;
            }
        }
        Edit::LanePoints { of: owner, .. }
        | Edit::LaneRange { of: owner, .. }
        | Edit::LaneDraw { of: owner, .. }
        | Edit::ClearLanes { of: owner, .. } => visit_lanes_of(owner, &of, f)?,
        Edit::RecordPass { lanes, .. } => {
            for lane in lanes {
                visit_lanes_of(&mut lane.of, &of, f)?;
            }
        }
        _ => unreachable!("the edits of grades are visited above"),
    }
    Ok(())
}

/// Call `f` on the id of the track of sound or the clip whose inserts a chain is, of timeline
/// `of`; a bus has no name of a transaction.
fn visit_inserts_of<E>(
    chain: &mut InsertsOf,
    of: &Id,
    f: &mut impl FnMut(&mut Id, Role) -> Result<(), E>,
) -> Result<(), E> {
    match chain {
        InsertsOf::Track(id) => f(id, Role::NamesTrack(of.clone())),
        InsertsOf::Clip(id) => f(id, Role::Names),
        InsertsOf::Bus(_) => Ok(()),
    }
}

/// Call `f` on the id of the track of sound whose lanes are named, of timeline `of`; a bus and
/// a VCA have no name of a transaction.
fn visit_lanes_of<E>(
    owner: &mut LanesOf,
    of: &Id,
    f: &mut impl FnMut(&mut Id, Role) -> Result<(), E>,
) -> Result<(), E> {
    match owner {
        LanesOf::Track(id) => f(id, Role::NamesTrack(of.clone())),
        LanesOf::Bus(_) | LanesOf::Vca(_) => Ok(()),
    }
}

/// Call `f` on the tracks of timeline `of` that a patch of sound names as the sources of a
/// sidechain or a Ducker: each object of it, at any depth, that is `{"track": id}`, as a source
/// is written.
fn visit_sources<E>(
    patch: &mut Value,
    of: &Id,
    f: &mut impl FnMut(&mut Id, Role) -> Result<(), E>,
) -> Result<(), E> {
    match patch {
        Value::Object(object) => {
            if object.len() == 1 {
                if let Some(Value::String(id)) = object.get_mut("track") {
                    return f(id, Role::NamesTrack(of.clone()));
                }
            }
            for value in object.values_mut() {
                visit_sources(value, of, f)?;
            }
            Ok(())
        }
        Value::Array(values) => {
            for value in values {
                visit_sources(value, of, f)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Call `f` on the track that drives the sidechain of an insert, if a track does.
fn visit_insert<E>(
    insert: &mut Insert,
    of: &Id,
    f: &mut impl FnMut(&mut Id, Role) -> Result<(), E>,
) -> Result<(), E> {
    match &mut insert.effect {
        Effect::Limiter(limiter) => match &mut limiter.sidechain.source {
            Some(SourceRef::Track(id)) => f(id, Role::NamesTrack(of.clone())),
            _ => Ok(()),
        },
        _ => Ok(()),
    }
}

/// Call `f` on the tracks that drive the sidechains and the Ducker of a strip.
fn visit_strip<E>(
    strip: &mut Strip,
    of: &Id,
    f: &mut impl FnMut(&mut Id, Role) -> Result<(), E>,
) -> Result<(), E> {
    let sources = strip
        .track_fx
        .iter_mut()
        .flat_map(|ducker| ducker.sources.iter_mut())
        .chain(strip.dynamics.compressor.sidechain.source.iter_mut());
    for source in sources {
        if let SourceRef::Track(id) = source {
            f(id, Role::NamesTrack(of.clone()))?;
        }
    }
    for insert in &mut strip.inserts {
        visit_insert(insert, of, f)?;
    }
    Ok(())
}

/// Call `f` on the ids of the timeline, the clip or the group that a grade is of.
fn visit_target<E>(
    target: &mut Target,
    f: &mut impl FnMut(&mut Id, Role) -> Result<(), E>,
) -> Result<(), E> {
    match target {
        Target::Clip { timeline, clip } => {
            f(timeline, Role::Names)?;
            f(clip, Role::Names)
        }
        Target::PreClip { group } | Target::PostClip { group } => f(group, Role::Names),
        Target::Timeline { timeline } => f(timeline, Role::Names),
    }
}

/// Call `f` on the ends of a link that are nodes. The ends of a grade are not names.
fn visit_link<E>(
    link: &mut Link,
    f: &mut impl FnMut(&mut Id, Role) -> Result<(), E>,
) -> Result<(), E> {
    f(&mut link.from, Role::Names)?;
    f(&mut link.to, Role::Names)
}

/// Call `f` on the ids of the nodes of a grade that is put in whole, and the ends of its links.
fn visit_grade<E>(
    grade: &mut Grade,
    f: &mut impl FnMut(&mut Id, Role) -> Result<(), E>,
) -> Result<(), E> {
    for node in &mut grade.nodes {
        f(&mut node.id, Role::Names)?;
    }
    for link in &mut grade.links {
        visit_link(link, f)?;
    }
    Ok(())
}

/// The id of the thing at `place` in an operation as it was done.
fn id_at<'a>(op: &'a Op, place: &Place) -> Option<&'a str> {
    let track_at = |at: Option<usize>| match (op, at) {
        (Op::AddTimeline { timeline, .. }, Some(at)) => timeline.tracks.get(at),
        (Op::AddTrack { track, .. }, None) => Some(track),
        _ => None,
    };
    let clip_at = |at, start| match op {
        Op::InsertClip { clip, .. } => Some(clip),
        _ => track_at(at)?.clips.iter().find(|c| c.start == start),
    };
    match place {
        Place::Media => {
            if let Op::AddMedia { media, .. } = op {
                Some(&media.id)
            } else {
                None
            }
        }
        Place::Bin => {
            if let Op::AddBin { bin, .. } = op {
                Some(&bin.id)
            } else {
                None
            }
        }
        Place::Timeline => {
            if let Op::AddTimeline { timeline, .. } = op {
                Some(&timeline.id)
            } else {
                None
            }
        }
        Place::Track(at) => Some(&track_at(*at)?.id),
        Place::Clip(at, start) => Some(&clip_at(*at, *start)?.id),
        Place::Link(at, start) => match op {
            Op::SetClip { link, .. } => link.as_ref()?.as_deref(),
            _ => clip_at(*at, *start)?.link.as_deref(),
        },
        Place::Transition(at, left) => match op {
            Op::AddTransition { transition, .. } => Some(&transition.id),
            _ => Some(
                &track_at(*at)?
                    .transitions
                    .iter()
                    .find(|x| x.left == *left)?
                    .id,
            ),
        },
        Place::Cue(at, start) => match op {
            Op::AddCue { cue, .. } => Some(&cue.id),
            _ => Some(&track_at(*at)?.cues.iter().find(|q| q.start == *start)?.id),
        },
        Place::Node => match op {
            Op::AddNode { node, .. } => Some(&node.id),
            _ => None,
        },
        Place::Group => match op {
            Op::AddGroup { group, .. } => Some(&group.id),
            _ => None,
        },
        Place::Output(_) => None,
    }
}

/// Visit references before declarations so aliases are only usable in later changes.
fn visit_track<E>(
    track: &mut Track,
    at: Option<usize>,
    of: Option<Id>,
    f: &mut impl FnMut(&mut Id, Role) -> Result<(), E>,
) -> Result<(), E> {
    for clip in &mut track.clips {
        if let Source::Media { media, .. } = &mut clip.source {
            f(media, Role::Names)?;
        }
        if let Some(group) = &mut clip.group {
            f(group, Role::Names)?;
        }
    }
    for transition in &mut track.transitions {
        f(&mut transition.left, Role::Names)?;
        f(&mut transition.right, Role::Names)?;
    }
    f(&mut track.id, Role::GivesTrack(Place::Track(at), of))?;
    for clip in &mut track.clips {
        f(&mut clip.id, Role::Gives(Place::Clip(at, clip.start)))?;
        if let Some(link) = &mut clip.link {
            f(link, Role::Link(Place::Link(at, clip.start)))?;
        }
    }
    for transition in &mut track.transitions {
        let place = Place::Transition(at, transition.left.clone());
        f(&mut transition.id, Role::Gives(place))?;
    }
    for cue in &mut track.cues {
        f(&mut cue.id, Role::Gives(Place::Cue(at, cue.start)))?;
    }
    Ok(())
}

/// Call `f` on each id of an operation, with what it is to the operation: first those that name
/// things that are there, then those of the things that it adds.
#[allow(clippy::too_many_lines, reason = "an arm for each operation")]
fn visit<E>(op: &mut Op, f: &mut impl FnMut(&mut Id, Role) -> Result<(), E>) -> Result<(), E> {
    match op {
        Op::AddMedia { media, .. } => {
            if let Some(bin) = &mut media.bin {
                f(bin, Role::Names)?;
            }
            f(&mut media.id, Role::Gives(Place::Media))
        }
        Op::RemoveMedia { id }
        | Op::SetMediaColor { id, .. }
        | Op::RemoveTimeline { id }
        | Op::SetTimeline { id, .. }
        | Op::RemoveBin { id }
        | Op::RemoveGroup { id }
        | Op::SetGroup { id, .. } => f(id, Role::Names),
        Op::SetProxy { media, .. } => f(media, Role::Names),
        Op::SetColorSettings { .. } => Ok(()),
        Op::AddTimeline { timeline, .. } => {
            let of = Some(timeline.id.clone()).filter(|id| !id.is_empty());
            f(&mut timeline.id, Role::Gives(Place::Timeline))?;
            for (at, track) in timeline.tracks.iter_mut().enumerate() {
                visit_track(track, Some(at), of.clone(), f)?;
            }
            Ok(())
        }
        Op::AddTrack {
            timeline, track, ..
        } => {
            let of = timeline.clone();
            f(timeline, Role::Names)?;
            visit_track(track, None, Some(of), f)
        }
        Op::RemoveTrack { timeline, id } | Op::SetTrack { timeline, id, .. } => {
            let of = timeline.clone();
            f(timeline, Role::Names)?;
            f(id, Role::NamesTrack(of))
        }
        Op::RemoveClip { timeline, id }
        | Op::TrimClip { timeline, id, .. }
        | Op::RemoveTransition { timeline, id }
        | Op::SetTransition { timeline, id, .. }
        | Op::RemoveCue { timeline, id }
        | Op::SetCue { timeline, id, .. } => {
            f(timeline, Role::Names)?;
            f(id, Role::Names)
        }
        Op::InsertClip {
            timeline,
            track,
            clip,
        } => {
            let of = timeline.clone();
            f(timeline, Role::Names)?;
            f(track, Role::NamesTrack(of))?;
            if let Source::Media { media, .. } = &mut clip.source {
                f(media, Role::Names)?;
            }
            if let Some(group) = &mut clip.group {
                f(group, Role::Names)?;
            }
            let start = clip.start;
            f(&mut clip.id, Role::Gives(Place::Clip(None, start)))?;
            if let Some(link) = &mut clip.link {
                f(link, Role::Link(Place::Link(None, start)))?;
            }
            Ok(())
        }
        Op::MoveClip {
            timeline,
            id,
            track,
            ..
        } => {
            let of = timeline.clone();
            f(timeline, Role::Names)?;
            f(id, Role::Names)?;
            f(track, Role::NamesTrack(of))
        }
        Op::ShiftClips {
            timeline,
            track,
            clips,
            ..
        }
        | Op::ShiftCues {
            timeline,
            track,
            cues: clips,
            ..
        } => {
            let of = timeline.clone();
            f(timeline, Role::Names)?;
            f(track, Role::NamesTrack(of))?;
            for id in clips {
                f(id, Role::Names)?;
            }
            Ok(())
        }
        Op::AddBin { bin, .. } => {
            if let Some(parent) = &mut bin.parent {
                f(parent, Role::Names)?;
            }
            f(&mut bin.id, Role::Gives(Place::Bin))
        }
        Op::SetClip {
            timeline,
            id,
            link,
            source,
            ..
        } => {
            f(timeline, Role::Names)?;
            f(id, Role::Names)?;
            if let Some(Source::Media { media, .. }) = source {
                f(media, Role::Names)?;
            }
            if let Some(Some(link)) = link {
                f(link, Role::Link(Place::Link(None, 0)))?;
            }
            Ok(())
        }
        Op::AddCue {
            timeline,
            track,
            cue,
        } => {
            let of = timeline.clone();
            f(timeline, Role::Names)?;
            f(track, Role::NamesTrack(of))?;
            f(&mut cue.id, Role::Gives(Place::Cue(None, cue.start)))
        }
        Op::SetBin { id, parent, .. } => {
            f(id, Role::Names)?;
            match parent {
                Some(Some(parent)) => f(parent, Role::Names),
                _ => Ok(()),
            }
        }
        Op::SetMedia { id, bin, .. } => {
            f(id, Role::Names)?;
            match bin {
                Some(Some(bin)) => f(bin, Role::Names),
                _ => Ok(()),
            }
        }
        Op::AddTransition {
            timeline,
            track,
            transition,
        } => {
            let of = timeline.clone();
            f(timeline, Role::Names)?;
            f(track, Role::NamesTrack(of))?;
            f(&mut transition.left, Role::Names)?;
            f(&mut transition.right, Role::Names)?;
            f(
                &mut transition.id,
                Role::Gives(Place::Transition(None, transition.left.clone())),
            )
        }
        Op::AddNode {
            grade, node, links, ..
        } => {
            visit_target(grade, f)?;
            // A link names the node that is added by the id that it is given, which is a name
            // that only the operations after it may name: so it is made empty with the id.
            let own = node.id.clone();
            for link in links {
                for end in [&mut link.from, &mut link.to] {
                    if *end == own && is_name(&own) {
                        end.clear();
                    } else {
                        f(end, Role::Names)?;
                    }
                }
            }
            f(&mut node.id, Role::Gives(Place::Node))
        }
        Op::RemoveNode { grade, id, links } => {
            visit_target(grade, f)?;
            f(id, Role::Names)?;
            for link in links.iter_mut().flatten() {
                visit_link(link, f)?;
            }
            Ok(())
        }
        Op::Connect { grade, link } => {
            visit_target(grade, f)?;
            visit_link(link, f)
        }
        Op::Disconnect { grade, to, .. } => {
            visit_target(grade, f)?;
            f(to, Role::Names)
        }
        Op::SetNode { grade, id, .. } => {
            visit_target(grade, f)?;
            f(id, Role::Names)
        }
        Op::SetGrade { grade, value } => {
            visit_target(grade, f)?;
            visit_grade(value, f)
        }
        Op::AddGroup { group, .. } => {
            visit_grade(&mut group.pre, f)?;
            visit_grade(&mut group.post, f)?;
            f(&mut group.id, Role::Gives(Place::Group))
        }
        Op::SetClipGroup {
            timeline,
            clip,
            group,
        } => {
            f(timeline, Role::Names)?;
            f(clip, Role::Names)?;
            match group {
                Some(group) => f(group, Role::Names),
                None => Ok(()),
            }
        }
        // The buses, VCAs and groups of tracks of a timeline are named by their own ids, not by
        // names of a transaction; the tracks that a group has, and that drive a sidechain or a
        // Ducker, are.
        Op::SetTrackAudio {
            timeline,
            track,
            patch,
        } => {
            let of = timeline.clone();
            f(timeline, Role::Names)?;
            f(track, Role::NamesTrack(of.clone()))?;
            visit_sources(patch, &of, f)
        }
        Op::SetBus {
            timeline, patch, ..
        } => {
            let of = timeline.clone();
            f(timeline, Role::Names)?;
            visit_sources(patch, &of, f)
        }
        Op::AddBus { timeline, bus, .. } => {
            let of = timeline.clone();
            f(timeline, Role::Names)?;
            visit_strip(&mut bus.strip, &of, f)
        }
        Op::AddTrackGroup {
            timeline, group, ..
        } => {
            let of = timeline.clone();
            f(timeline, Role::Names)?;
            for member in &mut group.members {
                f(member, Role::NamesTrack(of.clone()))?;
            }
            Ok(())
        }
        Op::SetTrackGroup {
            timeline, patch, ..
        } => {
            let of = timeline.clone();
            f(timeline, Role::Names)?;
            let members = patch.get_mut("members").and_then(Value::as_array_mut);
            for member in members.into_iter().flatten() {
                if let Value::String(id) = member {
                    f(id, Role::NamesTrack(of.clone()))?;
                }
            }
            Ok(())
        }
        Op::RemoveBus { timeline, .. }
        | Op::AddVca { timeline, .. }
        | Op::RemoveVca { timeline, .. }
        | Op::SetVca { timeline, .. }
        | Op::RemoveTrackGroup { timeline, .. }
        | Op::SetTimelineAudio { timeline, .. } => f(timeline, Role::Names),
        Op::AddInsert {
            timeline,
            of,
            insert,
            ..
        }
        | Op::SetInsert {
            timeline,
            of,
            insert: Some(insert),
            ..
        } => {
            let at = timeline.clone();
            f(timeline, Role::Names)?;
            visit_inserts_of(of, &at, f)?;
            visit_insert(insert, &at, f)
        }
        Op::SetInsert { timeline, of, .. } | Op::MoveInsert { timeline, of, .. } => {
            let at = timeline.clone();
            f(timeline, Role::Names)?;
            visit_inserts_of(of, &at, f)
        }
        Op::SetLane { timeline, of, .. } | Op::SpliceLane { timeline, of, .. } => {
            let at = timeline.clone();
            f(timeline, Role::Names)?;
            visit_lanes_of(of, &at, f)
        }
    }
}

#[cfg(test)]
mod tests {
    use columbina_model::{History, Project, TransactionError};
    use serde_json::json;

    use super::*;

    fn ops(value: serde_json::Value) -> Vec<Input> {
        serde_json::from_value(value).unwrap()
    }

    fn media(id: &str) -> serde_json::Value {
        let mut media = serde_json::to_value(&Project::sample().media[0]).unwrap();
        media["id"] = id.into();
        media
    }

    fn clip(id: &str, media: &str, start: i64) -> serde_json::Value {
        let mut clip =
            serde_json::to_value(&Project::sample().timelines[0].tracks[0].clips[0]).unwrap();
        clip["id"] = id.into();
        clip["source"]["media"] = media.into();
        clip["start"] = start.into();
        clip["length"] = 10.into();
        clip
    }

    /// Apply a transaction with names to the sample, and give back what was done and the ids
    /// that the names were given.
    fn apply(
        history: &mut History,
        mut ops: Vec<Input>,
    ) -> Result<(Vec<Op>, BTreeMap<String, Id>), TransactionError> {
        let mut names = prepare(&mut ops).unwrap();
        let applied = history
            .change_with(
                ops.into_iter().map(|input| input.change).collect(),
                "agent",
                &mut names,
            )?
            .applied;
        let ids = names.finish();
        Ok((applied.done, ids))
    }

    #[test]
    fn a_track_named_in_a_transaction_drives_a_ducker_and_joins_a_group_by_its_name() {
        let mut history = History::new(Project::sample());
        let track = json!({"id": "$voice", "kind": "audio", "name": "", "enabled": true,
            "locked": false, "transitions": [], "clips": []});
        let transaction = ops(json!([
            {"op": "add_track", "timeline": "t1", "track": track},
            {"op": "set_track_audio", "timeline": "t1", "track": "A1",
                "patch": {"strip": {"track_fx": {"sources": [{"track": "$voice"}]},
                    "dynamics": {"compressor": {"sidechain": {"source": {"track": "$voice"}}}}}}},
            {"op": "add_bus", "timeline": "t1", "bus": {"id": "", "name": "Music",
                "color": "#ffffffff", "format": "stereo", "strip": {"outputs": [{"bus": "B1"}],
                "inserts": [{"effect": {"kind": "limiter", "sidechain": {"source": {"track": "$voice"}}}}]}}},
            {"op": "add_track_group", "timeline": "t1",
                "group": {"id": "", "name": "Talk", "members": ["$voice"]}},
            {"op": "set_track_group", "timeline": "t1", "id": "TG1",
                "patch": {"members": ["A1", "$voice"]}},
        ]));
        let (_, ids) = apply(&mut history, transaction).unwrap();
        let voice = ids["$voice"].clone();
        assert_eq!(voice, "A2");
        let timeline = &history.project().timelines[0];
        let a1 = &timeline.tracks[1].audio.strip;
        assert_eq!(
            a1.track_fx.as_ref().unwrap().sources,
            [SourceRef::Track(voice.clone())]
        );
        assert_eq!(
            a1.dynamics.compressor.sidechain.source,
            Some(SourceRef::Track(voice.clone()))
        );
        let Effect::Limiter(limiter) = &timeline.buses[1].strip.inserts[0].effect else {
            panic!()
        };
        assert_eq!(
            limiter.sidechain.source,
            Some(SourceRef::Track(voice.clone()))
        );
        assert_eq!(timeline.track_groups[0].members, ["A1".to_string(), voice]);
    }

    #[test]
    fn a_track_named_in_a_transaction_is_given_lanes_by_its_name() {
        let mut history = History::new(Project::sample());
        let track = json!({"id": "$voice", "kind": "audio", "name": "", "enabled": true,
            "locked": false, "transitions": [], "clips": []});
        let point = |ticks: i64, value: f64| json!({"at": {"ticks": ticks, "scale": 48000}, "value": value});
        let transaction = ops(json!([
            {"op": "add_track", "timeline": "t1", "track": track},
            {"op": "set_lane", "timeline": "t1", "of": {"track": "$voice"}, "path": "fader_db",
                "lane": {"points": [point(0, -10.0)]}},
            {"op": "splice_lane", "timeline": "t1", "of": {"track": "$voice"}, "path": "fader_db",
                "at": 1, "remove": 0, "points": [point(48000, 0.0)]},
            {"edit": "lane_points", "timeline": "t1", "of": {"track": "$voice"}, "path": "mute",
                "action": "add", "points": [{"at": {"ticks": 0, "scale": 48000}, "value": 1}]},
        ]));
        let (_, ids) = apply(&mut history, transaction).unwrap();
        let voice = &history.project().timelines[0].tracks[2];
        assert_eq!(voice.id, ids["$voice"]);
        assert_eq!(voice.audio.lanes["fader_db"].points.len(), 2);
        assert_eq!(voice.audio.lanes["mute"].points[0].value, 1.0);
    }

    #[test]
    fn things_named_in_a_transaction_are_named_by_the_operations_after_them() {
        let mut history = History::new(Project::sample());
        let transaction = ops(json!([
            {"op": "add_media", "media": media("$m")},
            {"op": "add_timeline", "timeline": {
                "id": "$t", "name": "Second", "rate": {"num": 25, "den": 1}, "width": 1920, "height": 1080, "sample_rate": 48000,
                "tracks": [
                    {"id": "$v", "kind": "video", "name": "", "enabled": true, "locked": false, "transitions": [], "clips": [
                        clip("$late", "$m", 50), clip("$early", "m1", 0),
                    ]},
                    {"id": "", "kind": "audio", "name": "", "enabled": true, "locked": false, "transitions": [], "clips": []},
                ],
            }},
            {"op": "add_track", "timeline": "$t", "track": {
                "id": "$a", "kind": "audio", "name": "", "enabled": true, "locked": false, "transitions": [], "clips": [clip("$sound", "$m", 5)],
            }},
            {"op": "insert_clip", "timeline": "$t", "track": "$v", "clip": clip("$c", "$m", 100)},
            {"op": "move_clip", "timeline": "$t", "id": "$late", "track": "$v", "start": 200},
            {"op": "trim_clip", "timeline": "$t", "id": "$c", "source_in": {"ticks": 0, "scale": 15360},
             "start": 100, "length": 5},
            {"op": "remove_clip", "timeline": "$t", "id": "$early"},
            {"op": "shift_clips", "timeline": "$t", "track": "$v", "clips": ["$late", "$c"], "by": 10},
        ]));
        let (done, ids) = apply(&mut history, transaction).unwrap();
        // next_id is 3: m3, then t4 and its clips in the order of the list, c5 and c6; the
        // track of picture is V1 and the new track of sound A2; then c7 and c8.
        let expected: BTreeMap<String, Id> = [
            ("$m", "m3"),
            ("$t", "t4"),
            ("$v", "V1"),
            ("$late", "c5"),
            ("$early", "c6"),
            ("$a", "A2"),
            ("$sound", "c7"),
            ("$c", "c8"),
        ]
        .into_iter()
        .map(|(name, id)| (name.to_owned(), id.to_owned()))
        .collect();
        assert_eq!(ids, expected);
        assert_eq!(
            done[4],
            Op::MoveClip {
                timeline: "t4".into(),
                id: "c5".into(),
                track: "V1".into(),
                start: 200,
            }
        );
        assert_eq!(
            done[6],
            Op::RemoveClip {
                timeline: "t4".into(),
                id: "c6".into(),
            }
        );
        let text = serde_json::to_string(history.project()).unwrap();
        assert_eq!(
            done[7],
            Op::ShiftClips {
                timeline: "t4".into(),
                track: "V1".into(),
                clips: vec!["c5".into(), "c8".into()],
                by: 10,
            }
        );
        assert!(!text.contains('$'), "{text}");
    }

    #[test]
    fn a_name_is_named_only_after_it_is_given() {
        let mut transaction = ops(json!([
            {"op": "remove_media", "id": "$m"},
            {"op": "add_media", "media": media("$m")},
        ]));
        assert_eq!(
            prepare(&mut transaction).unwrap_err(),
            NameError {
                index: 0,
                name: "$m".into(),
                reason: "unknown_name",
            }
        );
        let mut transaction = ops(json!([
            {"op": "add_media", "media": media("$m")},
            {"op": "insert_clip", "timeline": "t1", "track": "V1", "clip": clip("", "$n", 100)},
        ]));
        let error = prepare(&mut transaction).unwrap_err();
        assert_eq!((error.index, error.reason), (1, "unknown_name"));
        assert_eq!(
            error.to_string(),
            "operation 1 names $n, which no operation before it in the transaction gives to a \
             thing"
        );
    }

    fn timeline(id: &str, tracks: &serde_json::Value) -> serde_json::Value {
        json!({ "id": id, "name": "N", "rate": { "num": 25, "den": 1 }, "width": 1920, "height": 1080, "sample_rate": 48000, "tracks": tracks })
    }

    #[test]
    fn a_name_of_a_track_is_named_only_with_its_timeline() {
        // $v is a track of the new timeline $t, and t1 has a V1 of its own.
        let with_v = timeline(
            "$t",
            &json!([{ "id": "$v", "kind": "video", "name": "", "enabled": true, "locked": false, "transitions": [], "clips": [] }]),
        );
        let mut transaction = ops(json!([
            { "op": "add_timeline", "timeline": with_v },
            { "op": "insert_clip", "timeline": "t1", "track": "$v", "clip": clip("", "m1", 200) },
        ]));
        let error = prepare(&mut transaction).unwrap_err();
        assert_eq!(
            (error.index, error.name.as_str(), error.reason),
            (1, "$v", "name_elsewhere")
        );
        assert_eq!(
            error.to_string(),
            "operation 1 names the track $v in another timeline than the one that it was given in"
        );
        // A track of a timeline that is added without an id cannot be named at all.
        let anonymous = timeline(
            "",
            &json!([{ "id": "$x", "kind": "video", "name": "", "enabled": true, "locked": false, "transitions": [], "clips": [] }]),
        );
        let mut transaction = ops(json!([
            { "op": "add_timeline", "timeline": anonymous },
            { "op": "move_clip", "timeline": "t3", "id": "c1", "track": "$x", "start": 0 },
        ]));
        assert_eq!(
            prepare(&mut transaction).unwrap_err().reason,
            "name_elsewhere"
        );

        // With its timeline, by the name or the id that the timeline was named by, it is.
        let mut history = History::new(Project::sample());
        let transaction = ops(json!([
            { "op": "add_timeline", "timeline": with_v },
            { "op": "insert_clip", "timeline": "$t", "track": "$v", "clip": clip("$c", "m1", 0) },
            { "op": "add_track", "timeline": "t1", "track": { "id": "$w", "kind": "video", "name": "", "enabled": true, "locked": false, "transitions": [], "clips": [] } },
            { "op": "move_clip", "timeline": "t1", "id": "c1", "track": "$w", "start": 0 },
            { "op": "remove_track", "timeline": "$t", "id": "$v" },
        ]));
        let (done, ids) = apply(&mut history, transaction).unwrap();
        assert_eq!(ids["$t"], "t3");
        assert_eq!(ids["$v"], "V1");
        assert_eq!(ids["$c"], "c4");
        assert_eq!(ids["$w"], "V2");
        assert_eq!(
            done[3],
            Op::MoveClip {
                timeline: "t1".into(),
                id: "c1".into(),
                track: "V2".into(),
                start: 0,
            }
        );
        let tracks = &history.project().timelines[0].tracks;
        assert_eq!(tracks[2].clips[0].id, "c1");
        assert_eq!(history.project().timelines[1].tracks, []);
    }

    #[test]
    fn clips_that_start_at_one_frame_on_two_tracks_keep_their_names() {
        let mut history = History::new(Project::sample());
        let tracks = json!([
            { "id": "$pic", "kind": "video", "name": "", "enabled": true, "locked": false, "transitions": [], "clips": [clip("$p2", "m1", 30), clip("$p1", "m1", 0)] },
            { "id": "$snd", "kind": "audio", "name": "", "enabled": true, "locked": false, "transitions": [], "clips": [clip("$s1", "m1", 0), clip("$s2", "m1", 30)] },
        ]);
        let transaction = ops(json!([
            { "op": "add_timeline", "timeline": timeline("$t", &tracks) },
            { "op": "trim_clip", "timeline": "$t", "id": "$s1", "source_in": {"ticks": 0, "scale": 15360},
              "start": 0, "length": 5 },
        ]));
        let (done, ids) = apply(&mut history, transaction).unwrap();
        // Given in the order of the lists: t3, V1, c4 ($p2), c5 ($p1), A1, c6 ($s1), c7 ($s2).
        let expected: Vec<(&str, &str)> = vec![
            ("$p1", "c5"),
            ("$p2", "c4"),
            ("$pic", "V1"),
            ("$s1", "c6"),
            ("$s2", "c7"),
            ("$snd", "A1"),
            ("$t", "t3"),
        ];
        let got: Vec<(&str, &str)> = ids.iter().map(|(n, i)| (n.as_str(), i.as_str())).collect();
        assert_eq!(got, expected);
        let Op::TrimClip { id, .. } = &done[1] else {
            panic!("{done:?}")
        };
        assert_eq!(id, "c6");
        let sound = &history.project().timelines[1].tracks[1].clips;
        assert_eq!((sound[0].id.as_str(), sound[0].length), ("c6", 5));
    }

    #[test]
    fn a_name_is_given_once() {
        let mut transaction = ops(json!([
            {"op": "add_media", "media": media("$x")},
            {"op": "insert_clip", "timeline": "t1", "track": "V1", "clip": clip("$x", "m1", 100)},
        ]));
        let error = prepare(&mut transaction).unwrap_err();
        assert_eq!((error.index, error.reason), (1, "duplicate_name"));
    }

    #[test]
    fn an_id_that_starts_with_a_dollar_is_a_name_or_refused() {
        let long = format!("${}", "a".repeat(LONGEST + 1));
        for id in ["$", "$a-b", "$é", long.as_str()] {
            let mut transaction = ops(json!([{"op": "add_media", "media": media(id)}]));
            let error = prepare(&mut transaction).unwrap_err();
            assert_eq!((error.index, error.reason), (0, "invalid_name"), "{id}");
            let mut transaction = ops(json!([{"op": "remove_timeline", "id": id}]));
            let error = prepare(&mut transaction).unwrap_err();
            assert_eq!(error.reason, "invalid_name", "{id}");
        }
        let longest = format!("${}", "Az_9".repeat(LONGEST / 4));
        assert!(is_name(&longest));
        assert!(!is_name("m1"));
    }

    #[test]
    fn a_transaction_without_names_is_applied_as_it_is() {
        let mut history = History::new(Project::sample());
        let transaction = ops(json!([
            {"op": "insert_clip", "timeline": "t1", "track": "V1", "clip": clip("", "m1", 100)},
        ]));
        let (done, ids) = apply(&mut history, transaction).unwrap();
        assert!(ids.is_empty());
        let Op::InsertClip { clip, .. } = &done[0] else {
            panic!("{done:?}")
        };
        assert_eq!(clip.id, "c3");
    }

    #[test]
    fn names_follow_input_changes_when_an_edit_expands_into_several_operations() {
        let mut history = History::new(Project::sample());
        let transaction = ops(json!([
            {"edit": "insert", "timeline": "t1", "at": 0, "source": {"kind": "color", "color": "#000000ff", "length": 10}},
            {"op": "add_media", "media": media("$m")},
            {"edit": "insert", "timeline": "t1", "at": 10, "source": {"kind": "media", "media": "$m", "length": 10}, "audio_track": null},
            {"op": "set_media", "id": "$m", "name": "Named after an edit"},
        ]));
        let (done, names) = apply(&mut history, transaction).unwrap();
        assert!(done.len() > 4);
        let id = &names["$m"];
        let added = history
            .project()
            .media
            .iter()
            .find(|m| m.id == *id)
            .unwrap();
        assert_eq!(added.name, "Named after an edit");
        assert_eq!(
            history.project().timelines[0].tracks[0].clips[1].media(),
            Some(id)
        );
        assert!(!serde_json::to_string(history.project())
            .unwrap()
            .contains('$'));
    }

    #[test]
    fn an_insert_names_its_new_clip_not_the_half_cut_from_the_old_one() {
        let mut history = History::new(Project::sample());
        let before = history.project().clone();
        let (_, names) = apply(&mut history, ops(json!([
            {"edit": "insert", "timeline": "t1", "at": 10,
             "source": {"kind": "color", "color": "#00ff00ff", "length": 5}, "name": "$inserted"},
            {"op": "set_clip", "timeline": "t1", "id": "$inserted", "enabled": false}
        ]))).unwrap();
        let new = history.project().timelines[0].tracks[0]
            .clips
            .iter()
            .find(|c| c.id == names["$inserted"])
            .unwrap();
        assert_eq!((new.start, new.length, new.enabled), (10, 5, false));
        assert!(matches!(new.source, Source::Color { .. }));
        let after = history.project().clone();
        history.undo("desktop").unwrap().unwrap();
        assert_eq!(
            history.project(),
            &Project {
                next_id: after.next_id,
                ..before
            }
        );
        history.redo("desktop").unwrap().unwrap();
        assert_eq!(history.project(), &after);
    }

    #[test]
    fn bins_transitions_subtitles_and_links_have_transaction_names() {
        let mut history = History::new(Project::sample());
        let (_, names) = apply(&mut history, ops(json!([
            {"op": "add_bin", "bin": {"id": "$bin", "name": "$literal", "parent": null}},
            {"op": "set_media", "id": "m1", "bin": "$bin"},
            {"op": "set_clip", "timeline": "t1", "id": "c1", "link": "$group"},
            {"op": "add_track", "timeline": "t1", "track": {"id": "$s", "kind": "subtitle", "name": "Subtitles", "enabled": true, "locked": false, "language": "en", "cues": []}},
            {"edit": "add_cue", "timeline": "t1", "track": "$s", "start": 0, "length": 10, "text": "$literal", "name": "$cue"},
            {"edit": "set_cue", "timeline": "t1", "id": "$cue", "text": "Named cue"},
            {"edit": "append", "timeline": "t1", "source": {"kind": "color", "color": "#000000ff", "length": 30}, "name": "$left"},
            {"edit": "append", "timeline": "t1", "source": {"kind": "color", "color": "#ffffffff", "length": 30}, "name": "$right"},
            {"edit": "add_transition", "timeline": "t1", "left": "$left", "right": "$right", "length": 10, "align": "center", "name": "$transition"},
            {"edit": "set_transition", "timeline": "t1", "id": "$transition", "length": 8}
        ]))).unwrap();
        let project = history.project();
        assert_eq!(project.bins[0].name, "$literal");
        assert_eq!(project.media[0].bin.as_ref(), Some(&names["$bin"]));
        assert_eq!(
            project.timelines[0].tracks[0].clips[0].link.as_ref(),
            Some(&names["$group"])
        );
        let t = &project.timelines[0];
        assert_eq!(
            t.tracks
                .iter()
                .flat_map(|r| &r.cues)
                .find(|q| q.id == names["$cue"])
                .unwrap()
                .text,
            "Named cue"
        );
        assert_eq!(t.tracks[0].transitions[0].id, names["$transition"]);
        assert_eq!(t.tracks[0].transitions[0].length, 8);
    }

    #[test]
    fn nodes_and_groups_of_grades_have_transaction_names() {
        let mut history = History::new(Project::sample());
        let corrector = serde_json::to_value(columbina_model::Node::corrector("$n")).unwrap();
        let mut mixer = serde_json::to_value(columbina_model::Node::new(
            "$mix",
            columbina_model::Kind::ParallelMixer,
        ))
        .unwrap();
        mixer["label"] = "$literal".into();
        let c1 = json!({"level": "clip", "timeline": "t1", "clip": "c1"});
        let (_, names) = apply(&mut history, ops(json!([
            {"op": "add_node", "grade": c1, "node": corrector, "links": [
                {"from": "source", "to": "$n", "kind": "rgb", "input": 0},
                {"from": "$n", "to": "output", "kind": "rgb", "input": 0},
            ]},
            {"op": "add_node", "grade": c1, "node": mixer, "links": [
                {"from": "$n", "to": "$mix", "kind": "rgb", "input": 1},
            ]},
            {"op": "set_node", "grade": c1, "id": "$n", "label": "Skin"},
            {"op": "connect", "grade": c1, "link": {"from": "source", "to": "$mix", "kind": "rgb", "input": 0}},
            {"op": "add_group", "group": {"id": "$k", "name": "$literal",
                "pre": columbina_model::Grade::default(), "post": columbina_model::Grade::default()}},
            {"op": "set_clip_group", "timeline": "t1", "clip": "c1", "group": "$k"},
            {"op": "add_node", "grade": {"level": "post_clip", "group": "$k"}, "node": serde_json::to_value(columbina_model::Node::corrector("")).unwrap()},
        ]))).unwrap();
        let project = history.project();
        let clip = &project.timelines[0].tracks[0].clips[0];
        let (n, mix) = (&names["$n"], &names["$mix"]);
        assert_eq!(clip.grade.nodes[0].id, *n);
        assert_eq!(clip.grade.nodes[0].label, "Skin");
        assert_eq!(clip.grade.nodes[1].id, *mix);
        assert_eq!(clip.grade.nodes[1].label, "$literal");
        assert_eq!(
            clip.grade
                .link_into(mix, columbina_model::Port::Rgb, 1)
                .unwrap()
                .from,
            *n
        );
        assert_eq!(
            clip.grade
                .link_into(mix, columbina_model::Port::Rgb, 0)
                .unwrap()
                .from,
            "source"
        );
        assert_eq!(clip.group.as_ref(), Some(&names["$k"]));
        assert_eq!(project.groups[0].id, names["$k"]);
        assert_eq!(project.groups[0].name, "$literal");
        assert_eq!(project.groups[0].post.nodes.len(), 1);
    }

    #[test]
    fn each_placement_can_name_its_picture_sound_and_link_group() {
        for edit in [
            "insert",
            "overwrite",
            "append",
            "smart_insert",
            "ripple_overwrite",
            "place_on_top",
        ] {
            let mut history = History::new(Project::sample());
            let mut command = json!({"edit": edit, "timeline": "t1", "at": 10,
                "source": {"kind": "media", "media": "m1", "length": 5},
                "name": {"video": "$v", "audio": "$a", "link": "$g"}});
            if edit == "append" {
                command.as_object_mut().unwrap().remove("at");
            }
            let (_, names) = apply(
                &mut history,
                ops(json!([
                    command,
                    {"op": "set_clip", "timeline": "t1", "id": "$a", "gain_db": -6},
                    {"op": "set_clip", "timeline": "t1", "id": "$v", "link": "$g"}
                ])),
            )
            .unwrap();
            let t = &history.project().timelines[0];
            for (name, kind) in [("$v", TrackKind::Video), ("$a", TrackKind::Audio)] {
                let (track, clip) = t
                    .tracks
                    .iter()
                    .find_map(|r| r.clips.iter().find(|c| c.id == names[name]).map(|c| (r, c)))
                    .unwrap();
                assert_eq!(track.kind, kind, "{edit}");
                assert_eq!(clip.length, 5, "{edit}");
                assert_eq!(clip.link.as_ref(), Some(&names["$g"]), "{edit}");
                if kind == TrackKind::Audio {
                    assert_eq!(clip.gain_db, (-6.0).into());
                }
            }
        }
    }

    #[test]
    fn an_absent_named_output_rolls_back_even_the_last_change_and_preserves_redo() {
        let mut history = History::new(Project::sample());
        apply(
            &mut history,
            ops(json!([{"op":"set_track", "timeline":"t1", "id":"V1", "name":"Picture"}])),
        )
        .unwrap();
        history.undo("desktop").unwrap().unwrap();
        let before = history.project().clone();
        let revision = history.revision();
        let error = apply(&mut history, ops(json!([
            {"op":"set_track", "timeline":"t1", "id":"V1", "name":"Changed"},
            {"edit":"insert", "timeline":"t1", "at":10, "source":{"kind":"color", "color":"#000000ff", "length":5}, "name":{"audio":"$a"}}
        ]))).unwrap_err();
        assert!(matches!(
            error,
            TransactionError::Refused {
                index: 1,
                error: OpError::Missing {
                    what: "named output",
                    ..
                }
            }
        ));
        assert_eq!(history.project(), &before);
        assert_eq!(history.revision(), revision);
        assert!(!history.can_undo());
        assert!(history.can_redo());
        history.redo("desktop").unwrap().unwrap();
        assert_eq!(history.project().timelines[0].tracks[0].name, "Picture");
    }

    #[test]
    fn output_names_are_strict_and_only_id_fields_are_rewritten() {
        for name in [
            json!(null),
            json!(3),
            json!({}),
            json!({"videos":"$v"}),
            json!({"clip":3}),
        ] {
            assert!(serde_json::from_value::<Input>(
                json!({"edit":"insert", "timeline":"t1", "at":0,
                "source":{"kind":"color", "color":"#000000ff", "length":5}, "name":name})
            )
            .is_err());
        }
        for (name, reason) in [
            (json!("plain"), "invalid_name"),
            (json!("$"), "invalid_name"),
            (json!({"clip":"$c", "video":"$c"}), "duplicate_name"),
        ] {
            let error = prepare(&mut ops(json!([{"edit":"insert", "timeline":"t1", "at":0,
                "source":{"kind":"color", "color":"#000000ff", "length":5}, "name":name}])))
            .unwrap_err();
            assert_eq!(error.reason, reason);
        }
        let mut history = History::new(Project::sample());
        apply(&mut history, ops(json!([
            {"edit":"add_track", "timeline":"t1", "kind":"subtitle", "name":"$display"},
            {"edit":"add_cue", "timeline":"t1", "track":"S1", "start":0, "length":5, "text":"$literal", "name":"$q"}
        ]))).unwrap();
        let track = &history.project().timelines[0].tracks[2];
        assert_eq!(track.name, "$display");
        assert_eq!(track.cues[0].text, "$literal");
    }

    #[test]
    fn basic_and_nested_schema_two_outputs_keep_names_after_sorting() {
        let mut history = History::new(Project::new());
        let c = |id, at| {
            serde_json::to_value(columbina_model::Clip::new(
                id,
                Source::Color {
                    color: columbina_model::Rgba([0, 0, 0, 255]),
                },
                at,
                20,
            ))
            .unwrap()
        };
        let (_, names) = apply(&mut history, ops(json!([
            {"op":"add_timeline", "timeline":{"id":"$t", "name":"", "rate":{"num":25,"den":1},
                "width":1920,"height":1080,"sample_rate":48000,"tracks":[
                    {"id":"$v","kind":"video","name":"","enabled":true,"locked":false,
                     "clips":[c("right",20), c("left",0)], "transitions":[
                        {"id":"$x","left":"left","right":"right","kind":"dissolve","align":"center","length":4}]},
                    {"id":"$s","kind":"subtitle","name":"","enabled":true,"locked":false,"language":"en","cues":[
                        {"id":"$late","start":20,"length":5,"text":"late"},
                        {"id":"$early","start":0,"length":5,"text":"early"}]}
                ]}},
            {"op":"set_cue","timeline":"$t","id":"$early","text":"first"},
            {"op":"set_transition","timeline":"$t","id":"$x","length":6},
            {"op":"add_cue","timeline":"$t","track":"$s","cue":{"id":"$new","start":10,"length":5,"text":"new"}},
            {"op":"set_cue","timeline":"$t","id":"$new","text":"middle"},
            {"op":"remove_transition","timeline":"$t","id":"$x"},
            {"op":"add_transition","timeline":"$t","track":"$v","transition":{
                "id":"$again","left":"left","right":"right","kind":"dissolve","align":"center","length":2}}
        ]))).unwrap();
        let t = &history.project().timelines[0];
        assert_eq!(
            t.tracks[1]
                .cues
                .iter()
                .map(|q| (q.id.as_str(), q.text.as_str()))
                .collect::<Vec<_>>(),
            [
                (names["$early"].as_str(), "first"),
                (names["$new"].as_str(), "middle"),
                (names["$late"].as_str(), "late")
            ]
        );
        assert_eq!(t.tracks[0].transitions[0].id, names["$again"]);
    }
}
