//! The operations on the stabilizer of a clip (see [`crate::stabilize`]): it is set whole, its
//! analysis with it, or its controls are set alone, its analysis kept; and the inverse of
//! either puts back what the clip had.

use super::{clip_at, timeline_at, Op, OpError};
use crate::project::{Clip, Project, TrackKind};
use crate::stabilize::{Controls, Stabilize};

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

/// Set the controls that `given` gives of the stabilizer of a clip, its analysis kept; or give
/// a clip that has none the one that a clip has at first with them. The inverse sets them back
/// to what they were, or takes the stabilizer away again. The analysis, which may be long, is
/// neither copied nor checked anew: it is what the project has, which was checked as it came.
pub(super) fn set_stabilize_settings(
    project: &mut Project,
    timeline: &str,
    clip: &str,
    given: &Controls,
) -> Result<Op, OpError> {
    let t = timeline_at(project, timeline)?;
    let (r, at) = clip_at(&project.timelines[t], clip)?;
    let kind = project.timelines[t].tracks[r].kind;
    let place = &mut project.timelines[t].tracks[r].clips[at].stabilize;
    // The controls as they would be, without the analysis.
    let mut controlled = match place.as_ref() {
        Some(had) => Stabilize {
            method: had.method,
            camera_lock: had.camera_lock,
            zoom: had.zoom,
            cropping_ratio: had.cropping_ratio,
            smooth: had.smooth,
            strength: had.strength,
            analysis: None,
        },
        None => Stabilize::default(),
    };
    controlled.set_controls(given);
    check_on(kind, &controlled)?;
    if let Some(stabilize) = place.as_mut() {
        let settings = stabilize.controls(given);
        stabilize.set_controls(given);
        return Ok(Op::SetStabilizeSettings {
            timeline: timeline.into(),
            clip: clip.into(),
            settings,
        });
    }
    *place = Some(controlled);
    Ok(Op::SetStabilize {
        timeline: timeline.into(),
        clip: clip.into(),
        value: None,
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

    fn settings(clip: &str, settings: Controls) -> Op {
        Op::SetStabilizeSettings {
            timeline: "t1".into(),
            clip: clip.into(),
            settings,
        }
    }

    #[test]
    fn controls_are_set_alone_the_analysis_kept_and_the_inverse_sets_back_those_given() {
        let mut project = Project::sample();
        apply(&mut project, set("c1", Some(measured()))).unwrap();
        let before = project.clone();
        let given = Controls {
            method: Some(Method::Translation),
            smooth: Some(0.9),
            strength: Some(-0.5),
            ..Controls::default()
        };
        let (done, inverse) = apply_done(&mut project, settings("c1", given)).unwrap();
        assert_eq!(done, settings("c1", given));
        // The old values of exactly the controls given.
        let old = Controls {
            method: Some(Method::Similarity),
            smooth: Some(0.5),
            strength: Some(1.0),
            ..Controls::default()
        };
        assert_eq!(inverse, settings("c1", old));
        // The others are kept, and so is the analysis, which is now of another Method.
        let changed = Stabilize {
            method: Method::Translation,
            smooth: 0.9,
            strength: -0.5,
            ..measured()
        };
        let clip = &project.timelines[0].tracks[0].clips[0];
        assert_eq!(clip.stabilize, Some(changed));
        assert!(clip.stabilize.as_ref().unwrap().needs_analysis());
        project.validate().unwrap();
        let again = apply(&mut project, inverse).unwrap();
        assert_eq!(project, before);
        assert_eq!(again, settings("c1", given));
    }

    #[test]
    fn a_clip_without_a_stabilizer_is_given_one_and_the_inverse_takes_it_away() {
        let mut project = Project::sample();
        let before = project.clone();
        let given = Controls {
            zoom: Some(false),
            cropping_ratio: Some(0.8),
            ..Controls::default()
        };
        let (done, inverse) = apply_done(&mut project, settings("c1", given)).unwrap();
        assert_eq!(done, settings("c1", given));
        assert_eq!(inverse, set("c1", None));
        let given_one = Stabilize {
            zoom: false,
            cropping_ratio: 0.8,
            ..Stabilize::default()
        };
        assert_eq!(
            project.timelines[0].tracks[0].clips[0].stabilize,
            Some(given_one)
        );
        project.validate().unwrap();
        apply(&mut project, inverse).unwrap();
        assert_eq!(project, before);
        // Given no control, it has the one that a clip has at first.
        let nothing = settings("c1", Controls::default());
        assert_eq!(apply(&mut project, nothing), Ok(set("c1", None)));
        assert_eq!(
            project.timelines[0].tracks[0].clips[0].stabilize,
            Some(Stabilize::default())
        );
    }

    #[test]
    fn controls_of_sound_or_out_of_their_ranges_or_of_no_clip_are_refused() {
        type Case = (&'static str, Controls);
        let mut project = Project::sample();
        apply(&mut project, set("c1", Some(measured()))).unwrap();
        let before = project.clone();
        assert_eq!(
            apply(&mut project, settings("c2", Controls::default())),
            Err(OpError::WrongTrack {
                what: "stabilize",
                kind: TrackKind::Audio
            })
        );
        let cases: [Case; 4] = [
            (
                "stabilize.cropping_ratio",
                Controls {
                    cropping_ratio: Some(1.5),
                    ..Controls::default()
                },
            ),
            (
                "stabilize.smooth",
                Controls {
                    smooth: Some(-0.1),
                    ..Controls::default()
                },
            ),
            (
                "stabilize.strength",
                Controls {
                    strength: Some(2.0),
                    ..Controls::default()
                },
            ),
            // One out of its range among others that are in theirs.
            (
                "stabilize.smooth",
                Controls {
                    zoom: Some(false),
                    smooth: Some(1.01),
                    ..Controls::default()
                },
            ),
        ];
        for (what, given) in cases {
            let refused: Result<Op, OpError> = Err(OpError::OutOfRange { what });
            assert_eq!(
                apply(&mut project, settings("c1", given)),
                refused,
                "{what}"
            );
            // And of a clip that has no stabilizer.
            let mut plain = Project::sample();
            assert_eq!(apply(&mut plain, settings("c1", given)), refused, "{what}");
            assert_eq!(plain, Project::sample());
        }
        assert!(matches!(
            apply(&mut project, settings("c404", Controls::default())),
            Err(OpError::Missing { what: "clip", .. })
        ));
        assert_eq!(project, before);
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
