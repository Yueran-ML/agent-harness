//! The operations on the lanes of automation of a timeline (Ch. 178): a lane of a track of
//! sound, a bus or a VCA put in place or taken away whole ([`Op::SetLane`]), and a run of its
//! points put in place of another ([`Op::SpliceLane`]), whose inverse holds only the points that
//! it took away, so that a change to a long lane is a short step of undo.

use serde::{Deserialize, Serialize};

use super::{timeline_at, track_at, Op, OpError};
use crate::audio::automation::{check_lanes, Owner};
use crate::audio::{Lane, LanePoint, Lanes};
use crate::project::{Id, Project, Timeline, TrackKind};

/// What the lanes of an operation belong to: a track of sound, a bus or a VCA. It is written
/// `{"track": "A1"}`, `{"bus": "B1"}` or `{"vca": "VCA1"}`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LanesOf {
    /// A track of sound.
    Track(Id),
    /// A bus.
    Bus(Id),
    /// A VCA.
    Vca(Id),
}

/// The owner of `of` in a timeline, as it is, and its lanes.
///
/// # Errors
/// It is not there (`missing` track, `unknown_bus`, `unknown_vca`), or the track is not of sound
/// (`wrong_track`, `what: "lanes"`).
pub(crate) fn owner_of<'t>(
    timeline: &'t Timeline,
    of: &LanesOf,
) -> Result<(Owner<'t>, &'t Lanes), OpError> {
    match of {
        LanesOf::Track(id) => {
            let track = &timeline.tracks[track_at(timeline, id)?];
            if track.kind != TrackKind::Audio {
                return Err(OpError::WrongTrack {
                    what: "lanes",
                    kind: track.kind,
                });
            }
            let owner = Owner::Strip {
                strip: &track.audio.strip,
                mute: track.audio.mute,
                bus: false,
            };
            Ok((owner, &track.audio.lanes))
        }
        LanesOf::Bus(id) => {
            let bus = timeline
                .buses
                .iter()
                .find(|b| b.id == *id)
                .ok_or_else(|| OpError::UnknownBus { id: id.clone() })?;
            let owner = Owner::Strip {
                strip: &bus.strip,
                mute: bus.mute,
                bus: true,
            };
            Ok((owner, &bus.lanes))
        }
        LanesOf::Vca(id) => {
            let vca = timeline
                .vcas
                .iter()
                .find(|v| v.id == *id)
                .ok_or_else(|| OpError::UnknownVca { id: id.clone() })?;
            Ok((Owner::Vca(vca), &vca.lanes))
        }
    }
}

/// The lanes of `of` in a timeline, to change.
fn lanes_mut<'t>(timeline: &'t mut Timeline, of: &LanesOf) -> &'t mut Lanes {
    match of {
        LanesOf::Track(id) => {
            let r = track_at(timeline, id).expect("the owner was found");
            &mut timeline.tracks[r].audio.lanes
        }
        LanesOf::Bus(id) => {
            let bus = timeline.buses.iter_mut().find(|b| b.id == *id);
            &mut bus.expect("the owner was found").lanes
        }
        LanesOf::Vca(id) => {
            let vca = timeline.vcas.iter_mut().find(|v| v.id == *id);
            &mut vca.expect("the owner was found").lanes
        }
    }
}

/// Change the lanes of `of` by `change`, check them, and keep them.
fn with_lanes<R>(
    project: &mut Project,
    timeline_id: &str,
    of: &LanesOf,
    change: impl FnOnce(&mut Lanes) -> Result<R, OpError>,
) -> Result<R, OpError> {
    let t = timeline_at(project, timeline_id)?;
    let timeline = &project.timelines[t];
    let (owner, lanes) = owner_of(timeline, of)?;
    let mut lanes = lanes.clone();
    let given = change(&mut lanes)?;
    check_lanes(&lanes, owner)?;
    *lanes_mut(&mut project.timelines[t], of) = lanes;
    Ok(given)
}

fn missing_lane(path: &str) -> OpError {
    OpError::Missing {
        what: "lane",
        id: path.into(),
    }
}

pub(super) fn set_lane(
    project: &mut Project,
    timeline: &str,
    of: LanesOf,
    path: String,
    lane: Option<Lane>,
) -> Result<Op, OpError> {
    let old = with_lanes(project, timeline, &of, |lanes| match lane {
        Some(lane) => Ok(lanes.insert(path.clone(), lane)),
        None => lanes
            .remove(&path)
            .map(Some)
            .ok_or_else(|| missing_lane(&path)),
    })?;
    Ok(Op::SetLane {
        timeline: timeline.into(),
        of,
        path,
        lane: old,
    })
}

pub(super) fn splice_lane(
    project: &mut Project,
    timeline: &str,
    of: LanesOf,
    path: String,
    (at, remove): (usize, usize),
    points: Vec<LanePoint>,
) -> Result<Op, OpError> {
    let put = points.len();
    let removed = with_lanes(project, timeline, &of, |lanes| {
        let lane = lanes.get_mut(&path).ok_or_else(|| missing_lane(&path))?;
        let len = lane.points.len();
        let end = at.checked_add(remove).filter(|&end| end <= len);
        let Some(end) = end else {
            return Err(OpError::OutOfList {
                at: at.saturating_add(remove),
                len,
            });
        };
        Ok(lane.points.splice(at..end, points).collect::<Vec<_>>())
    })?;
    Ok(Op::SpliceLane {
        timeline: timeline.into(),
        of,
        path,
        at,
        remove: put,
        points: removed,
    })
}
