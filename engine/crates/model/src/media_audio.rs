//! Media-pool audio layouts, copied into independent clips when media is placed.

use serde::{Deserialize, Serialize};

use crate::audio::{check_channel_maps, ChannelMap, ChannelOrder, TrackFormat, MOST_CHANNELS};
use crate::ops::{Op, OpError};
use crate::project::{Id, Media, Project};

/// One source audio track in Clip Attributes (pp. 449–450, 952–955).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MediaAudioTrack {
    /// Its display name, used for a new destination track.
    pub name: String,
    /// The configured output layout; an adaptive layout has as many channels as its maps.
    pub format: TrackFormat,
    /// The ordering of the channels of a 5.1 layout.
    #[serde(default)]
    pub order: ChannelOrder,
    /// Source choices, activity, mute and level, with the same meanings as on a clip.
    pub channels: Vec<ChannelMap>,
}

impl MediaAudioTrack {
    /// The channels that participate in an edit. A muted channel remains; an inactive one
    /// is left out. A reduced layout becomes mono, stereo or adaptive as appropriate.
    #[must_use]
    pub fn active(&self) -> Option<Self> {
        let channels: Vec<_> = self
            .channels
            .iter()
            .filter(|map| map.enabled)
            .cloned()
            .collect();
        if channels.is_empty() {
            return None;
        }
        let reduced = channels.len() != self.channels.len();
        let format = if reduced {
            match channels.len() {
                1 => TrackFormat::Mono,
                2 => TrackFormat::Stereo,
                _ => TrackFormat::Adaptive,
            }
        } else {
            self.format
        };
        Some(Self {
            name: self.name.clone(),
            format,
            order: if reduced {
                ChannelOrder::Smpte
            } else {
                self.order
            },
            channels,
        })
    }
}

/// Validate an optional media-pool layout without altering any timeline instances.
///
/// # Errors
/// A layout on silent media, an invalid format/channel count, name or channel level.
pub fn check(media: &Media) -> Result<(), OpError> {
    let Some(tracks) = &media.audio_config else {
        return Ok(());
    };
    let bad = || OpError::OutOfRange {
        what: "audio_config",
    };
    if media.facts.audio.is_none() {
        return Err(bad());
    }
    for track in tracks {
        let count = track.channels.len();
        if count == 0
            || count > usize::from(MOST_CHANNELS)
            || track
                .format
                .channels()
                .is_some_and(|n| usize::from(n) != count)
            || (track.format != TrackFormat::FiveOne && track.order != ChannelOrder::Smpte)
            || track.name.chars().count() > crate::grade::LONGEST_LABEL
        {
            return Err(bad());
        }
        check_channel_maps(&track.channels)?;
    }
    Ok(())
}

pub(crate) fn set(
    project: &mut Project,
    id: &Id,
    config: Option<Vec<MediaAudioTrack>>,
) -> Result<Op, OpError> {
    let at = project
        .media
        .iter()
        .position(|media| media.id == *id)
        .ok_or_else(|| OpError::Missing {
            what: "media",
            id: id.clone(),
        })?;
    let mut media = project.media[at].clone();
    media.audio_config = config;
    check(&media)?;
    let old = std::mem::replace(&mut project.media[at], media);
    Ok(Op::SetMediaAudio {
        id: id.clone(),
        config: old.audio_config,
    })
}
