//! Check integer inputs before an edit does arithmetic with them.

use super::{Edit, Material};
use crate::ops::{within, OpError};

pub(super) fn check(edit: &Edit) -> Result<(), OpError> {
    match edit {
        Edit::Insert { source, at, .. }
        | Edit::Overwrite { source, at, .. }
        | Edit::Replace { source, at, .. }
        | Edit::PlaceOnTop { source, at, .. }
        | Edit::SmartInsert { source, at, .. }
        | Edit::RippleOverwrite { source, at, .. } => {
            within("at", i128::from(*at))?;
            material(source)
        }
        Edit::Append { source, .. } => material(source),
        Edit::CloseGap { at, .. } | Edit::Blade { at, .. } | Edit::InsertMove { at, .. } => {
            within("at", i128::from(*at))
        }
        Edit::RippleDeleteRange { from, to, .. } => {
            within("from", i128::from(*from))?;
            within("to", i128::from(*to))
        }
        Edit::Move { delta, .. }
        | Edit::Trim { delta, .. }
        | Edit::Roll { delta, .. }
        | Edit::Slip { delta, .. }
        | Edit::Slide { delta, .. } => within("delta", i128::from(*delta)),
        Edit::AddTransition { length, .. } => within("length", i128::from(*length)),
        Edit::SetTransition { length, .. } => optional("length", *length),
        Edit::AddCue { start, length, .. } => {
            within("start", i128::from(*start))?;
            within("length", i128::from(*length))
        }
        Edit::SetCue { start, length, .. } => {
            optional("start", *start)?;
            optional("length", *length)
        }
        Edit::CuesFrom { items, .. } => {
            for cue in items {
                within("start", i128::from(cue.start))?;
                within("length", i128::from(cue.length))?;
            }
            Ok(())
        }
        Edit::FadeToPlayhead { at, .. } => within("at", i128::from(*at)),
        Edit::ClipGainKeys { at, to, .. } => {
            within("at", i128::from(at.ticks))?;
            optional("to", to.map(|to| to.ticks))
        }
        Edit::ClipGainRange { from, to, .. } => {
            within("from", i128::from(from.ticks))?;
            within("to", i128::from(to.ticks))
        }
        Edit::SetTrackAudio { .. }
        | Edit::NudgeClipGain { .. }
        | Edit::PasteAttributes { .. }
        | Edit::RemoveAttributes { .. }
        | Edit::CopyEffects { .. }
        | Edit::Lift { .. }
        | Edit::RippleDelete { .. }
        | Edit::RemoveTransition { .. }
        | Edit::SetSpeed { .. }
        | Edit::RemoveCue { .. }
        | Edit::AddTrack { .. }
        | Edit::RemoveTrack { .. }
        | Edit::AddSerial { .. }
        | Edit::AddSerialBefore { .. }
        | Edit::AppendNode { .. }
        | Edit::AddParallel { .. }
        | Edit::AddLayer { .. }
        | Edit::AddOutside { .. }
        | Edit::ExtractNode { .. }
        | Edit::ResetNode { .. }
        | Edit::CopyGrade { .. }
        | Edit::AppendNodeGraph { .. }
        | Edit::CopyGivenGrade { .. }
        | Edit::AppendGivenGraph { .. }
        | Edit::CollapseGroupGrades { .. }
        | Edit::PasteValue { .. }
        | Edit::PasteNode { .. } => Ok(()),
    }
}

fn material(source: &Material) -> Result<(), OpError> {
    match source {
        Material::Title { length, .. } | Material::Color { length, .. } => {
            within("length", i128::from(*length))
        }
        Material::Media {
            from, to, length, ..
        } => {
            for (what, time) in [("from", from), ("to", to)] {
                if let Some(time) = time {
                    within(what, i128::from(time.ticks))?;
                }
            }
            optional("length", *length)
        }
    }
}

fn optional(what: &'static str, value: Option<i64>) -> Result<(), OpError> {
    value.map_or(Ok(()), |value| within(what, i128::from(value)))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::super::{expand, fixture, Edit, EditError};
    use crate::ops::{OpError, LARGEST};

    #[test]
    fn edit_inputs_outside_the_integer_range_are_refused_before_arithmetic() {
        for far in [i64::MIN, -LARGEST - 1, LARGEST + 1, i64::MAX] {
            let colour = json!({"kind": "color", "color": "#000000ff", "length": 10});
            let mut edits = vec![
                json!({"edit": "move", "clips": ["c1"], "delta": far}),
                json!({"edit": "trim", "clip": "c1", "edge": "out", "delta": far}),
                json!({"edit": "roll", "left": "c1", "right": "c3", "delta": far}),
                json!({"edit": "slip", "clip": "c1", "delta": far}),
                json!({"edit": "slide", "clip": "c1", "delta": far}),
                json!({"edit": "blade", "at": far}),
                json!({"edit": "close_gap", "track": "V2", "at": far}),
                json!({"edit": "insert_move", "clips": ["c1"], "at": far}),
                json!({"edit": "ripple_delete_range", "from": far, "to": 40}),
                json!({"edit": "ripple_delete_range", "from": 20, "to": far}),
                json!({"edit": "add_cue", "track": "S1", "start": far, "length": 10, "text": "hi"}),
                json!({"edit": "add_cue", "track": "S1", "start": 50, "length": far, "text": "hi"}),
                json!({"edit": "set_cue", "id": "q6", "start": far}),
                json!({"edit": "set_cue", "id": "q6", "length": far}),
                json!({"edit": "cues_from", "track": "S1", "items": [{"start": far, "length": 10, "text": "hi"}]}),
                json!({"edit": "cues_from", "track": "S1", "items": [{"start": 50, "length": far, "text": "hi"}]}),
                json!({"edit": "add_transition", "left": "c1", "right": "c3", "length": far}),
                json!({"edit": "set_transition", "id": "x99", "length": far}),
            ];
            for name in [
                "insert",
                "overwrite",
                "replace",
                "place_on_top",
                "smart_insert",
                "ripple_overwrite",
            ] {
                edits.push(json!({"edit": name, "source": colour, "at": far}));
            }
            for source in [
                json!({"kind": "color", "color": "#000000ff", "length": far}),
                json!({"kind": "title", "text": "hi", "style": {"font": "", "size": 72, "color": "#ffffffff", "x": 0.5, "y": 0.5, "align": "center"}, "length": far}),
                json!({"kind": "media", "media": "m1", "length": far}),
                json!({"kind": "media", "media": "m1", "from": {"ticks": far, "scale": 25}}),
                json!({"kind": "media", "media": "m1", "to": {"ticks": far, "scale": 25}}),
            ] {
                edits.push(json!({"edit": "append", "source": source}));
                edits.push(json!({"edit": "overwrite", "source": source, "at": 20}));
            }
            for mut value in edits {
                value["timeline"] = json!("t1");
                let edit: Edit = serde_json::from_value(value.clone()).unwrap();
                let before = fixture::project();
                let mut project = before.clone();
                let error = expand(&mut project, edit).unwrap_err();
                assert!(
                    matches!(error, EditError::Op(OpError::TooLarge { .. })),
                    "{value}: {error}"
                );
                assert_eq!(project, before, "{value}");
            }
        }
    }

    #[test]
    fn a_frame_at_the_integer_limit_still_reaches_the_edit_rules() {
        let error = fixture::refused(
            fixture::project(),
            Edit::Blade {
                timeline: "t1".into(),
                at: LARGEST,
                tracks: None,
                linked: true,
            },
        );
        assert_eq!(
            error,
            EditError::NothingAt {
                track: None,
                frame: LARGEST
            }
        );
    }

    #[test]
    fn an_out_of_range_edit_takes_back_earlier_edits_in_its_transaction() {
        let before = fixture::project();
        let mut history = crate::History::new(before.clone());
        let error = history
            .edit(
                vec![
                    Edit::Lift {
                        timeline: "t1".into(),
                        clips: vec!["c5".into()],
                        linked: true,
                    },
                    Edit::Move {
                        timeline: "t1".into(),
                        clips: vec!["c3".into()],
                        delta: i64::MAX,
                        to_track: None,
                        overwrite: false,
                        linked: true,
                    },
                ],
                "agent",
            )
            .unwrap_err();
        assert!(matches!(
            error,
            crate::TransactionError::Edit {
                index: 1,
                error: EditError::Op(OpError::TooLarge { what: "delta" })
            }
        ));
        assert_eq!(history.project(), &before);
        assert!(history.undo("desktop").is_none());
    }
}
