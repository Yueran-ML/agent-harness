//! Route configured media-pool audio tracks without reformatting existing timeline tracks.

use super::misc::new_track;
use super::work::Work;
use super::EditError;
use crate::audio::TrackFormat;
use crate::media_audio::MediaAudioTrack;
use crate::ops::Op;
use crate::project::{Id, TrackKind};

pub(super) fn configured(
    work: &mut Work,
    config: &[MediaAudioTrack],
    mut from: usize,
) -> Result<Vec<Id>, EditError> {
    let mut placed = Vec::new();
    for source in config {
        let tracks: Vec<_> = work
            .timeline()
            .tracks
            .iter()
            .filter(|track| track.kind == TrackKind::Audio)
            .collect();
        let found = tracks
            .iter()
            .enumerate()
            .skip(from)
            .find(|(_, track)| {
                !track.locked
                    && track.audio.format == source.format
                    && usize::from(track.audio.channels) == source.channels.len()
                    && (source.format != TrackFormat::FiveOne || track.audio.order == source.order)
            })
            .map(|(index, track)| (index, track.id.clone()));
        let (index, id) = if let Some(found) = found {
            found
        } else {
            let id = new_track(
                work,
                TrackKind::Audio,
                Some(from),
                (!source.name.is_empty()).then(|| source.name.clone()),
                None,
            )?;
            work.op(Op::SetTrackAudio {
                timeline: work.timeline_id(),
                track: id.clone(),
                patch: serde_json::json!({
                    "format": source.format,
                    "channels": source.channels.len(),
                    "order": source.order,
                }),
            })?;
            (from, id)
        };
        from = index + 1;
        placed.push(id);
    }
    Ok(placed)
}
