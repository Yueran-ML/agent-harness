//! A temporary mix of file channels, without any timeline-instance processing.
//!
//! Callers supply channel maps, not an existing clip or track. Thus a File audition cannot
//! accidentally inherit clip gain, pitch, fades, EQ, automation, track strips or other clips.
//! Decoding, channel conversion and output routing remain the ordinary mixer's responsibility.

use columbina_model::{
    Clip, Media, MediaAudioTrack, Project, Rate, Time, Timeline, Track, TrackKind,
};

/// Build a source-only mix. `channel` addresses the original flattened layout, before inactive
/// channels are removed. Solo preserves channel routing, activity, mute and channel level.
pub(crate) fn project(
    media: &Media,
    rate: Rate,
    length: i64,
    rows: &[MediaAudioTrack],
    channel: Option<usize>,
) -> Result<Project, String> {
    if channel.is_some_and(|index| index >= rows.iter().map(|row| row.channels.len()).sum()) {
        return Err("the audition channel is outside the file layout".into());
    }
    let mut timeline = Timeline::new("source-audio", "", rate);
    let mut offset = 0;
    for (index, row) in rows.iter().enumerate() {
        let mut row = row.clone();
        for map in &mut row.channels {
            if channel.is_some_and(|selected| selected != offset) {
                map.mute = true;
            }
            offset += 1;
        }
        let Some(row) = row.active() else { continue };
        let mut track = Track::new(format!("A{index}"), TrackKind::Audio);
        track.audio.format = row.format;
        track.audio.channels = u8::try_from(row.channels.len()).expect("validated channel count");
        track.audio.order = row.order;
        let mut clip = Clip::of_media(
            format!("source-{index}"),
            media.id.clone(),
            Time {
                ticks: 0,
                scale: media.facts.duration.scale,
            },
            0,
            length,
        );
        clip.channels = Some(row.channels);
        track.clips.push(clip);
        timeline.tracks.push(track);
    }
    let mut media = media.clone();
    media.bin = None;
    Ok(Project {
        schema: columbina_model::project::SCHEMA,
        next_id: 1,
        bins: vec![],
        media: vec![media],
        timelines: vec![timeline],
        groups: vec![],
        color: columbina_model::ColorSettings::default(),
    })
}

#[cfg(test)]
mod tests;
