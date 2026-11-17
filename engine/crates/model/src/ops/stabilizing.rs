//! The operation on the stabilizer of a clip (see [`crate::stabilize`]): it is set whole, its
//! analysis with it, and its inverse puts back what the clip had.

use super::{clip_at, timeline_at, Op, OpError};
use crate::project::{Clip, Project, TrackKind};
use crate::stabilize::Stabilize;

/// Check a stabilizer as it would be on a track of `kind`: a stabilizer is of a picture, so a
/// clip of sound has none; and its controls and its analysis are within their ranges.
fn check_on(kind: TrackKind, stabilize: &Stabilize) -> Result<(), OpError> {
    if kind != TrackKind::Video {
        return Err(OpError::WrongTrack {
            what: "stabilize",
            kind,
        });
    }
    stabilize.check()
}

/// Check the stabilizer of a clip as it would be on a track of `kind`, where it has one.
pub(super) fn check_of(kind: TrackKind, clip: &Clip) -> Result<(), OpError> {
    match &clip.stabilize {
        Some(stabilize) => check_on(kind, stabilize),
        None => Ok(()),
    }
}

pub(super) fn set_stabilize(
    project: &mut Project,
    timeline: &str,
    clip: &str,
    value: Option<Stabilize>,
) -> Result<Op, OpError> {
    let t = timeline_at(project, timeline)?;
    let (r, at) = clip_at(&project.timelines[t], clip)?;
    if let Some(stabilize) = &value {
        check_on(project.timelines[t].tracks[r].kind, stabilize)?;
    }
    let old = std::mem::replace(
        &mut project.timelines[t].tracks[r].clips[at].stabilize,
        value,
    );
    Ok(Op::SetStabilize {
        timeline: timeline.into(),
        clip: clip.into(),
        value: old,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::ops::{apply, apply_done};
    use crate::stabilize::{numbers_of, Analysis, Method};
    use columbina_track::Motion;

    fn set(clip: &str, value: Option<Stabilize>) -> Op {
        Op::SetStabilize {
            timeline: "t1".into(),
            clip: clip.into(),
            value,
        }
    }

    /// A stabilizer of Similarity with an analysis of three frames of a shift.
    fn measured() -> Stabilize {
        let path: BTreeMap<i64, [f64; 8]> = (0..3)
            .map(|frame| {
                let shift = Motion {
                    pan: f64::from(i32::try_from(frame).unwrap()) * 1.5,
                    ..Motion::IDENTITY
                };
                (frame, numbers_of(&shift))
            })
            .collect();
        Stabilize {
            method: Method::Similarity,
            smooth: 0.5,
            analysis: Some(Analysis::new(Method::Similarity, [1280, 720], path)),
            ..Stabilize::default()
        }
    }

    #[test]
    fn a_stabilizer_is_set_whole_and_its_inverse_puts_back_what_the_clip_had() {
        let mut project = Project::sample();
        let before = project.clone();
        let (done, inverse) = apply_done(&mut project, set("c1", Some(measured()))).unwrap();
        assert_eq!(done, set("c1", Some(measured())));
        assert_eq!(inverse, set("c1", None));
        assert_eq!(
            project.timelines[0].tracks[0].clips[0].stabilize,
            Some(measured())
        );
        // Another in its place, and back.
        let plain = Stabilize::default();
        let back = apply(&mut project, set("c1", Some(plain.clone()))).unwrap();
        assert_eq!(back, set("c1", Some(measured())));
        assert_eq!(
            project.timelines[0].tracks[0].clips[0].stabilize,
            Some(plain)
        );
        apply(&mut project, back).unwrap();
        apply(&mut project, inverse).unwrap();
        assert_eq!(project, before);
        project.validate().unwrap();
    }

    #[test]
    fn a_stabilizer_of_sound_or_out_of_its_ranges_or_of_no_clip_is_refused() {
        let mut project = Project::sample();
        let before = project.clone();
        assert_eq!(
            apply(&mut project, set("c2", Some(Stabilize::default()))),
            Err(OpError::WrongTrack {
                what: "stabilize",
                kind: TrackKind::Audio
            })
        );
        let smooth = Stabilize {
            smooth: 2.0,
            ..Stabilize::default()
        };
        assert_eq!(
            apply(&mut project, set("c1", Some(smooth))),
            Err(OpError::OutOfRange {
                what: "stabilize.smooth"
            })
        );
        assert!(matches!(
            apply(&mut project, set("c404", None)),
            Err(OpError::Missing { what: "clip", .. })
        ));
        assert_eq!(project, before);
        // Taking away what is not there is no change.
        assert_eq!(apply(&mut project, set("c1", None)), Ok(set("c1", None)));
        // A clip of sound with one is no clip that a project may have.
        project.timelines[0].tracks[1].clips[0].stabilize = Some(Stabilize::default());
        assert!(project.validate().is_err());
    }
}
