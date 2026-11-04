//! The operations on the sound of a timeline (schema 4, see [`crate::audio`]): what a track of
//! sound has of sound, its buses, VCAs and groups of tracks, the inserts of a strip or the effects
//! of a clip, and the settings of the sound of a timeline.
//!
//! The settings of a track, a bus, a VCA, a group or a timeline are set by a patch, as the
//! settings of colour management are (see [`Op::SetColorSettings`]): each object within it is put
//! into the object of that name, and any other value (a list, a number, `null`) takes the place
//! of the one there, whole. Its inverse is the patch of what was there at the same places, and
//! the patch as it was done is that of what is there now at the places of its inverse.
//!
//! A bus that is added with an empty id is given `B` and a number one above the highest of the
//! ids of that shape among the buses of its timeline, a VCA `VCA` and one so, and a group of
//! tracks `TG` and one so; as a track is, and unlike the things that `next_id` numbers, so that
//! taking one away and adding one again may give the id that it had. An insert of a strip that is
//! added with an empty id is given the next `fx<n>` of its strip ([`audio::name_inserts`]).

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    clip_at, place, routed, routing_of, routing_of_track, timeline_at, track_at, Op, OpError,
};
use crate::audio::fx::SourceRef;
use crate::audio::{
    self, ChannelOrder, Insert, Strip, TimelineAudio, TrackAudio, TrackFormat, TrackGroup, Vca,
};
use crate::grade::{changed, merge};
use crate::project::{Id, Project, Timeline, TrackKind};

/// The chain of inserts that an operation is of: the strip of a track of sound, the strip of a
/// bus, or the effects of a clip. It is written `{"track": "A1"}`, `{"bus": "B1"}` or
/// `{"clip": "c1"}`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InsertsOf {
    /// The strip of a track of sound.
    Track(Id),
    /// The strip of a bus.
    Bus(Id),
    /// The effects of a clip.
    Clip(Id),
}

/// What a track of sound has of sound that a patch may set: all of [`TrackAudio`] but its lanes,
/// which the operations on automation set.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(clippy::struct_excessive_bools, reason = "the buttons of a track")]
struct TrackSound {
    format: TrackFormat,
    channels: u8,
    order: ChannelOrder,
    mute: bool,
    solo: bool,
    solo_safe: bool,
    adc: bool,
    automation_arm: bool,
    strip: Strip,
}

impl TrackSound {
    fn of(audio: &TrackAudio) -> Self {
        Self {
            format: audio.format,
            channels: audio.channels,
            order: audio.order,
            mute: audio.mute,
            solo: audio.solo,
            solo_safe: audio.solo_safe,
            adc: audio.adc,
            automation_arm: audio.automation_arm,
            strip: audio.strip.clone(),
        }
    }

    fn put(self, audio: &mut TrackAudio) {
        audio.format = self.format;
        audio.channels = self.channels;
        audio.order = self.order;
        audio.mute = self.mute;
        audio.solo = self.solo;
        audio.solo_safe = self.solo_safe;
        audio.adc = self.adc;
        audio.automation_arm = self.automation_arm;
        audio.strip = self.strip;
    }
}

/// Settings as they are written.
fn written(settings: &impl Serialize) -> Value {
    serde_json::to_value(settings).expect("settings of sound are written as JSON")
}

fn wrong(why: impl Into<String>) -> OpError {
    OpError::AudioSettings { why: why.into() }
}

/// Put `patch` into `before`, and give back what it makes and the patch that sets it back.
///
/// # Errors
/// The patch is not an object, sets one of `fixed` (an id, or lanes), or does not make settings
/// of the kind of `before` (`audio_settings`, with serde's message).
fn patched<T: Serialize + DeserializeOwned>(
    before: &T,
    patch: &Value,
    fixed: &[&str],
) -> Result<(T, Value), OpError> {
    let Some(fields) = patch.as_object() else {
        return Err(wrong("the settings are not an object"));
    };
    if let Some(name) = fixed.iter().find(|name| fields.contains_key(**name)) {
        return Err(wrong(format!("`{name}` is not set by a patch")));
    }
    let before = written(before);
    let mut value = before.clone();
    merge(&mut value, patch);
    let after = serde_json::from_value(value).map_err(|error| wrong(error.to_string()))?;
    Ok((after, changed(&before, patch)))
}

/// Check the routing of a timeline, and the settings of its buses, VCAs and sound, as they would
/// be once `change` is made to them.
fn checked(timeline: &Timeline, change: impl FnOnce(&mut Timeline)) -> Result<(), OpError> {
    let mut routing = routing_of(timeline);
    change(&mut routing);
    audio::check_timeline(&routing).map_err(|(_, _, error)| error)
}

/// The id after the highest of `prefix` and a number among `ids`: `prefix` and 1 where there is
/// none.
fn next_of<'a>(prefix: &str, ids: impl Iterator<Item = &'a Id>) -> Result<Id, OpError> {
    let highest = ids
        .filter_map(|id| id.strip_prefix(prefix)?.parse::<u64>().ok())
        .max()
        .unwrap_or(0);
    match highest.checked_add(1) {
        Some(number) if number <= super::LARGEST_COUNT => Ok(format!("{prefix}{number}")),
        _ => Err(OpError::IdsExhausted),
    }
}

fn bus_at(timeline: &Timeline, id: &str) -> Result<usize, OpError> {
    timeline
        .buses
        .iter()
        .position(|b| b.id == id)
        .ok_or_else(|| OpError::UnknownBus { id: id.into() })
}

fn vca_at(timeline: &Timeline, id: &str) -> Result<usize, OpError> {
    timeline
        .vcas
        .iter()
        .position(|v| v.id == id)
        .ok_or_else(|| OpError::UnknownVca { id: id.into() })
}

fn group_at(timeline: &Timeline, id: &str) -> Result<usize, OpError> {
    timeline
        .track_groups
        .iter()
        .position(|g| g.id == id)
        .ok_or_else(|| OpError::UnknownGroup { id: id.into() })
}

/// The place of a track of sound in a timeline.
fn sound_track_at(timeline: &Timeline, id: &str) -> Result<usize, OpError> {
    let r = track_at(timeline, id)?;
    let kind = timeline.tracks[r].kind;
    if kind == TrackKind::Audio {
        Ok(r)
    } else {
        Err(OpError::WrongTrack {
            what: "strip",
            kind,
        })
    }
}

/// Where a patch of a track sets its format and says not how many channels, those of the
/// format, or the `channels` that the track has for an adaptive one: so a stereo track made
/// mono has one, and the patch names its channels wherever it names its format, as its inverse
/// does.
fn with_channels(patch: &Value, channels: u8) -> Value {
    let mut patch = patch.clone();
    if let Some(fields) = patch.as_object_mut() {
        let format = fields
            .get("format")
            .and_then(|f| serde_json::from_value::<TrackFormat>(f.clone()).ok());
        if let (Some(format), false) = (format, fields.contains_key("channels")) {
            let count = format.channels().unwrap_or(channels);
            fields.insert("channels".into(), count.into());
        }
    }
    patch
}

pub(super) fn set_track_audio(
    project: &mut Project,
    timeline_id: &str,
    track_id: &str,
    patch: &Value,
) -> Result<Op, OpError> {
    let t = timeline_at(project, timeline_id)?;
    let timeline = &project.timelines[t];
    let r = sound_track_at(timeline, track_id)?;
    let patch = with_channels(patch, timeline.tracks[r].audio.channels);
    let (settings, inverse) = patched(&TrackSound::of(&timeline.tracks[r].audio), &patch, &[])?;
    let mut track = timeline.tracks[r].clone();
    settings.put(&mut track.audio);
    audio::name_inserts(&mut track.audio.strip);
    audio::check_track(&track)?;
    routed(timeline, |tracks| tracks[r] = routing_of_track(&track))?;
    project.timelines[t].tracks[r] = track;
    Ok(Op::SetTrackAudio {
        timeline: timeline_id.into(),
        track: track_id.into(),
        patch: inverse,
    })
}

pub(super) fn add_bus(
    project: &mut Project,
    timeline_id: &str,
    mut bus: audio::Bus,
    index: Option<usize>,
) -> Result<Op, OpError> {
    let t = timeline_at(project, timeline_id)?;
    let timeline = &project.timelines[t];
    let at = place(index, timeline.buses.len())?;
    if bus.id.is_empty() {
        bus.id = next_of("B", timeline.buses.iter().map(|b| &b.id))?;
    }
    audio::name_inserts(&mut bus.strip);
    checked(timeline, |t| t.buses.insert(at, bus.clone()))?;
    let id = bus.id.clone();
    project.timelines[t].buses.insert(at, bus);
    Ok(Op::RemoveBus {
        timeline: timeline_id.into(),
        id,
    })
}

/// What names bus `id` of a timeline, if anything does: a strip that outputs or sends to it, or
/// whose sidechain or Ducker it drives (the track or the bus of the strip), or the monitor (the
/// timeline).
fn user_of_bus(timeline: &Timeline, id: &str) -> Option<Id> {
    let names = |strip: &Strip| {
        strip.outputs.iter().any(|o| o.bus == id)
            || strip.sends.iter().any(|s| s.bus == id)
            || strip
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
                .any(|source| matches!(source, SourceRef::Bus(bus) if bus == id))
    };
    let strips = timeline
        .tracks
        .iter()
        .filter(|r| r.kind == TrackKind::Audio)
        .map(|r| (&r.id, &r.audio.strip))
        .chain(timeline.buses.iter().map(|b| (&b.id, &b.strip)));
    for (by, strip) in strips {
        if names(strip) {
            return Some(by.clone());
        }
    }
    (timeline.audio.monitor.as_deref() == Some(id)).then(|| timeline.id.clone())
}

pub(super) fn remove_bus(
    project: &mut Project,
    timeline_id: &str,
    id: &str,
) -> Result<Op, OpError> {
    let t = timeline_at(project, timeline_id)?;
    let timeline = &project.timelines[t];
    let at = bus_at(timeline, id)?;
    if let Some(by) = user_of_bus(timeline, id) {
        return Err(OpError::InUse {
            what: "bus",
            id: id.into(),
            by,
        });
    }
    let bus = project.timelines[t].buses.remove(at);
    Ok(Op::AddBus {
        timeline: timeline_id.into(),
        bus,
        index: Some(at),
    })
}

pub(super) fn set_bus(
    project: &mut Project,
    timeline_id: &str,
    id: &str,
    patch: &Value,
) -> Result<Op, OpError> {
    let t = timeline_at(project, timeline_id)?;
    let timeline = &project.timelines[t];
    let b = bus_at(timeline, id)?;
    let (mut bus, inverse) = patched(&timeline.buses[b], patch, &["id", "lanes"])?;
    audio::name_inserts(&mut bus.strip);
    checked(timeline, |t| t.buses[b] = bus.clone())?;
    project.timelines[t].buses[b] = bus;
    Ok(Op::SetBus {
        timeline: timeline_id.into(),
        id: id.into(),
        patch: inverse,
    })
}

pub(super) fn add_vca(
    project: &mut Project,
    timeline_id: &str,
    mut vca: Vca,
    index: Option<usize>,
) -> Result<Op, OpError> {
    let t = timeline_at(project, timeline_id)?;
    let timeline = &project.timelines[t];
    let at = place(index, timeline.vcas.len())?;
    if vca.id.is_empty() {
        vca.id = next_of("VCA", timeline.vcas.iter().map(|v| &v.id))?;
    }
    checked(timeline, |t| t.vcas.insert(at, vca.clone()))?;
    let id = vca.id.clone();
    project.timelines[t].vcas.insert(at, vca);
    Ok(Op::RemoveVca {
        timeline: timeline_id.into(),
        id,
    })
}

pub(super) fn remove_vca(
    project: &mut Project,
    timeline_id: &str,
    id: &str,
) -> Result<Op, OpError> {
    let t = timeline_at(project, timeline_id)?;
    let timeline = &project.timelines[t];
    let at = vca_at(timeline, id)?;
    let member = |vcas: &[Id]| vcas.iter().any(|v| v == id);
    let by = timeline
        .tracks
        .iter()
        .filter(|r| r.kind == TrackKind::Audio)
        .map(|r| (&r.id, &r.audio.strip.vcas))
        .chain(timeline.buses.iter().map(|b| (&b.id, &b.strip.vcas)))
        .chain(timeline.vcas.iter().map(|v| (&v.id, &v.vcas)))
        .find(|(_, vcas)| member(vcas))
        .map(|(by, _)| by.clone());
    if let Some(by) = by {
        return Err(OpError::InUse {
            what: "vca",
            id: id.into(),
            by,
        });
    }
    let vca = project.timelines[t].vcas.remove(at);
    Ok(Op::AddVca {
        timeline: timeline_id.into(),
        vca,
        index: Some(at),
    })
}

pub(super) fn set_vca(
    project: &mut Project,
    timeline_id: &str,
    id: &str,
    patch: &Value,
) -> Result<Op, OpError> {
    let t = timeline_at(project, timeline_id)?;
    let timeline = &project.timelines[t];
    let v = vca_at(timeline, id)?;
    let (vca, inverse) = patched(&timeline.vcas[v], patch, &["id", "lanes"])?;
    checked(timeline, |t| t.vcas[v] = vca.clone())?;
    project.timelines[t].vcas[v] = vca;
    Ok(Op::SetVca {
        timeline: timeline_id.into(),
        id: id.into(),
        patch: inverse,
    })
}

pub(super) fn add_track_group(
    project: &mut Project,
    timeline_id: &str,
    mut group: TrackGroup,
    index: Option<usize>,
) -> Result<Op, OpError> {
    let t = timeline_at(project, timeline_id)?;
    let timeline = &project.timelines[t];
    let at = place(index, timeline.track_groups.len())?;
    if group.id.is_empty() {
        group.id = next_of("TG", timeline.track_groups.iter().map(|g| &g.id))?;
    }
    checked(timeline, |t| t.track_groups.insert(at, group.clone()))?;
    let id = group.id.clone();
    project.timelines[t].track_groups.insert(at, group);
    Ok(Op::RemoveTrackGroup {
        timeline: timeline_id.into(),
        id,
    })
}

pub(super) fn remove_track_group(
    project: &mut Project,
    timeline_id: &str,
    id: &str,
) -> Result<Op, OpError> {
    let t = timeline_at(project, timeline_id)?;
    let at = group_at(&project.timelines[t], id)?;
    let group = project.timelines[t].track_groups.remove(at);
    Ok(Op::AddTrackGroup {
        timeline: timeline_id.into(),
        group,
        index: Some(at),
    })
}

pub(super) fn set_track_group(
    project: &mut Project,
    timeline_id: &str,
    id: &str,
    patch: &Value,
) -> Result<Op, OpError> {
    let t = timeline_at(project, timeline_id)?;
    let timeline = &project.timelines[t];
    let g = group_at(timeline, id)?;
    let (group, inverse) = patched(&timeline.track_groups[g], patch, &["id"])?;
    checked(timeline, |t| t.track_groups[g] = group.clone())?;
    project.timelines[t].track_groups[g] = group;
    Ok(Op::SetTrackGroup {
        timeline: timeline_id.into(),
        id: id.into(),
        patch: inverse,
    })
}

pub(super) fn set_timeline_audio(
    project: &mut Project,
    timeline_id: &str,
    patch: &Value,
) -> Result<Op, OpError> {
    let t = timeline_at(project, timeline_id)?;
    let timeline = &project.timelines[t];
    let (settings, inverse) = patched::<TimelineAudio>(&timeline.audio, patch, &[])?;
    checked(timeline, |t| t.audio = settings.clone())?;
    project.timelines[t].audio = settings;
    Ok(Op::SetTimelineAudio {
        timeline: timeline_id.into(),
        patch: inverse,
    })
}

/// Change the chain of inserts `of` of a timeline by `change`, check what it makes and keep it,
/// and give back what `change` gives.
fn with_inserts<R>(
    project: &mut Project,
    timeline_id: &str,
    of: &InsertsOf,
    change: impl FnOnce(&mut Vec<Insert>) -> Result<R, OpError>,
) -> Result<R, OpError> {
    let t = timeline_at(project, timeline_id)?;
    let timeline = &project.timelines[t];
    match of {
        InsertsOf::Track(id) => {
            let r = sound_track_at(timeline, id)?;
            let mut track = timeline.tracks[r].clone();
            let given = change(&mut track.audio.strip.inserts)?;
            audio::name_inserts(&mut track.audio.strip);
            audio::check_track(&track)?;
            routed(timeline, |tracks| tracks[r] = routing_of_track(&track))?;
            project.timelines[t].tracks[r] = track;
            Ok(given)
        }
        InsertsOf::Bus(id) => {
            let b = bus_at(timeline, id)?;
            let mut bus = timeline.buses[b].clone();
            let given = change(&mut bus.strip.inserts)?;
            audio::name_inserts(&mut bus.strip);
            checked(timeline, |t| t.buses[b] = bus.clone())?;
            project.timelines[t].buses[b] = bus;
            Ok(given)
        }
        InsertsOf::Clip(id) => {
            let (r, at) = clip_at(timeline, id)?;
            let mut clip = timeline.tracks[r].clips[at].clone();
            let given = change(&mut clip.effects)?;
            audio::check_clip(&clip, timeline.rate)?;
            project.timelines[t].tracks[r].clips[at] = clip;
            Ok(given)
        }
    }
}

/// The inserts of the chain `of` of a timeline of a project, as they are.
pub(crate) fn inserts_of<'p>(
    project: &'p Project,
    timeline_id: &str,
    of: &InsertsOf,
) -> Result<&'p [Insert], OpError> {
    let timeline = &project.timelines[timeline_at(project, timeline_id)?];
    Ok(match of {
        InsertsOf::Track(id) => {
            &timeline.tracks[sound_track_at(timeline, id)?]
                .audio
                .strip
                .inserts
        }
        InsertsOf::Bus(id) => &timeline.buses[bus_at(timeline, id)?].strip.inserts,
        InsertsOf::Clip(id) => {
            let (r, at) = clip_at(timeline, id)?;
            &timeline.tracks[r].clips[at].effects
        }
    })
}

/// The place `at` of a list of `len`, which must be in it.
fn within_list(at: usize, len: usize) -> Result<usize, OpError> {
    if at < len {
        Ok(at)
    } else {
        Err(OpError::OutOfList { at, len })
    }
}

pub(super) fn add_insert(
    project: &mut Project,
    timeline_id: &str,
    of: InsertsOf,
    insert: Insert,
    index: Option<usize>,
) -> Result<Op, OpError> {
    let slot = with_inserts(project, timeline_id, &of, |inserts| {
        let at = place(index, inserts.len())?;
        inserts.insert(at, insert);
        Ok(at)
    })?;
    Ok(Op::SetInsert {
        timeline: timeline_id.into(),
        of,
        slot,
        insert: None,
    })
}

pub(super) fn set_insert(
    project: &mut Project,
    timeline_id: &str,
    of: InsertsOf,
    slot: usize,
    insert: Option<Insert>,
) -> Result<Op, OpError> {
    let replacing = insert.is_some();
    let old = with_inserts(project, timeline_id, &of, |inserts| {
        let at = within_list(slot, inserts.len())?;
        Ok(match insert {
            Some(insert) => std::mem::replace(&mut inserts[at], insert),
            None => inserts.remove(at),
        })
    })?;
    let timeline = timeline_id.into();
    // What was replaced is put back in its place; what was taken away is added back there.
    Ok(if replacing {
        Op::SetInsert {
            timeline,
            of,
            slot,
            insert: Some(old),
        }
    } else {
        Op::AddInsert {
            timeline,
            of,
            insert: old,
            index: Some(slot),
        }
    })
}

pub(super) fn move_insert(
    project: &mut Project,
    timeline_id: &str,
    of: InsertsOf,
    from: usize,
    to: usize,
) -> Result<Op, OpError> {
    with_inserts(project, timeline_id, &of, |inserts| {
        let len = inserts.len();
        let from = within_list(from, len)?;
        let to = within_list(to, len)?;
        let insert = inserts.remove(from);
        inserts.insert(to, insert);
        Ok(())
    })?;
    Ok(Op::MoveInsert {
        timeline: timeline_id.into(),
        of,
        from: to,
        to: from,
    })
}

/// An operation on the sound of a timeline as it was done, read back from the project that it
/// was applied to and the inverse that it gave: a patch with the values that the project has now
/// at the places of its inverse, as the engine writes them; and an insert put in place of
/// another with the id that it was given.
///
/// # Panics
/// If what was changed is not in the project, which would be a bug.
pub(super) fn done(project: &Project, op: Op, inverse: &Op) -> Op {
    const THERE: &str = "what was changed is in the project";
    let timeline_of = |id: &str| &project.timelines[timeline_at(project, id).expect(THERE)];
    match (op, inverse) {
        (
            Op::SetTrackAudio {
                timeline, track, ..
            },
            Op::SetTrackAudio { patch, .. },
        ) => {
            let t = timeline_of(&timeline);
            let r = track_at(t, &track).expect(THERE);
            let now = written(&TrackSound::of(&t.tracks[r].audio));
            Op::SetTrackAudio {
                timeline,
                track,
                patch: changed(&now, patch),
            }
        }
        (Op::SetBus { timeline, id, .. }, Op::SetBus { patch, .. }) => {
            let t = timeline_of(&timeline);
            let now = written(&t.buses[bus_at(t, &id).expect(THERE)]);
            Op::SetBus {
                timeline,
                id,
                patch: changed(&now, patch),
            }
        }
        (Op::SetVca { timeline, id, .. }, Op::SetVca { patch, .. }) => {
            let t = timeline_of(&timeline);
            let now = written(&t.vcas[vca_at(t, &id).expect(THERE)]);
            Op::SetVca {
                timeline,
                id,
                patch: changed(&now, patch),
            }
        }
        (Op::SetTrackGroup { timeline, id, .. }, Op::SetTrackGroup { patch, .. }) => {
            let t = timeline_of(&timeline);
            let now = written(&t.track_groups[group_at(t, &id).expect(THERE)]);
            Op::SetTrackGroup {
                timeline,
                id,
                patch: changed(&now, patch),
            }
        }
        (Op::SetTimelineAudio { timeline, .. }, Op::SetTimelineAudio { patch, .. }) => {
            let now = written(&timeline_of(&timeline).audio);
            Op::SetTimelineAudio {
                timeline,
                patch: changed(&now, patch),
            }
        }
        (
            Op::SetInsert {
                timeline,
                of,
                slot,
                insert: Some(_),
            },
            _,
        ) => {
            let inserts = inserts_of(project, &timeline, &of).expect(THERE);
            Op::SetInsert {
                insert: Some(inserts[slot].clone()),
                timeline,
                of,
                slot,
            }
        }
        (op, _) => op,
    }
}

/// [`Op::AddBus`], [`Op::AddVca`], [`Op::AddTrackGroup`] or [`Op::AddInsert`] as it was done,
/// with the thing as it is in the project and its place, read back from the inverse that takes
/// it away.
pub(super) fn added(project: &Project, inverse: &Op) -> Option<Op> {
    let timeline_of = |id: &str| timeline_at(project, id).ok().map(|t| &project.timelines[t]);
    match inverse {
        Op::RemoveBus { timeline, id } => {
            let t = timeline_of(timeline)?;
            let at = bus_at(t, id).ok()?;
            Some(Op::AddBus {
                timeline: timeline.clone(),
                bus: t.buses[at].clone(),
                index: Some(at),
            })
        }
        Op::RemoveVca { timeline, id } => {
            let t = timeline_of(timeline)?;
            let at = vca_at(t, id).ok()?;
            Some(Op::AddVca {
                timeline: timeline.clone(),
                vca: t.vcas[at].clone(),
                index: Some(at),
            })
        }
        Op::RemoveTrackGroup { timeline, id } => {
            let t = timeline_of(timeline)?;
            let at = group_at(t, id).ok()?;
            Some(Op::AddTrackGroup {
                timeline: timeline.clone(),
                group: t.track_groups[at].clone(),
                index: Some(at),
            })
        }
        Op::SetInsert {
            timeline,
            of,
            slot,
            insert: None,
        } => Some(Op::AddInsert {
            timeline: timeline.clone(),
            of: of.clone(),
            insert: inserts_of(project, timeline, of).ok()?.get(*slot)?.clone(),
            index: Some(*slot),
        }),
        _ => None,
    }
}
