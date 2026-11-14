//! The Highlight modes of the timeline monitor (`viewer.highlight`, pp. 3170–3171): what it shows
//! of the key of the node chosen in place of the picture, in the preview alone (an export, a
//! frame drawn off screen and the reference of a wipe are drawn as they were).
//!
//! - **Highlight** (Shift-H): the picture where the node's key is and a flat grey where it is not,
//!   "the selected portion of the image with the original colors, and the unselected portion of
//!   the image with a flat gray", mixed by the key; the picture is the frame as it is shown, every
//!   grade and transform done. The grey is put in place in the node's grade, so the grades after
//!   it (a group's post-clip grade, the timeline's) and the output transform do to it what they
//!   do to the picture: with none, as in a project of `DaVinci YRGB` with its timeline ungraded, it
//!   is flat. A node without a key of its own is shown alone instead ("only the currently
//!   selected node is output", p. 3171): the grade gives the node's picture in place of its own.
//! - **Highlight BW** (Alt-Shift-H): the key of the node, "the selected portion of the image is
//!   white, and the unselected portion of the image is black". For a node of a clip's grade or of
//!   its group's, only the layers of those clips are drawn, without the grades after it, the
//!   timeline's grade, the output transform or subtitles, so that the key is seen as it is; for a
//!   node of the timeline's grade, every layer, and the output transform left out.
//!
//! A node that the grade does not run through (one that is not there, or a grade that is not
//! complete) shows the picture as it is.

use columbina_compose::grade::{Look, Show};
use columbina_compose::{Scene, Stage};
use columbina_model::grade::Target as GradeTarget;
use columbina_model::Project;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use serde_json::{json, Value};

use crate::grading::{Grading, Level};
use crate::rpc::{self, Fault};
use crate::view::Monitor;

/// What a Highlight mode shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// The picture, as it is.
    Off,
    /// Highlight: the picture where the key is, grey where it is not.
    Highlight,
    /// Highlight BW: the key, white and black.
    Bw,
}

/// A Highlight mode that is on: what it shows, of the key of which node of which grade.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Highlight {
    pub mode: Mode,
    pub grade: GradeTarget,
    pub node: String,
}

/// The params of `viewer.highlight`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Asked {
    #[serde(default)]
    monitor: Option<Monitor>,
    mode: Mode,
    #[serde(default)]
    grade: Option<GradeTarget>,
    #[serde(default)]
    node: Option<String>,
}

impl Highlight {
    /// The mode that the params of `viewer.highlight` ask for: none to turn it off.
    ///
    /// # Errors
    /// The params are not those of the method (`-32602`): a monitor other than the timeline
    /// monitor (`reason: monitor`), or a mode on without the grade and the node it shows
    /// (`reason: node`).
    pub fn read(params: Option<&RawValue>) -> Result<Option<Highlight>, Fault> {
        let asked: Asked = rpc::params(
            "viewer.highlight",
            r#"{"mode": "off" | "highlight" | "bw", "grade"?: {"level": "clip", ...}, "node"?: "n1", "monitor"?: "timeline"}"#,
            params,
        )?;
        if asked.monitor == Some(Monitor::Source) {
            return Err(Fault::params(
                "the source monitor shows no grade: a Highlight mode is of the timeline monitor",
            )
            .with_data(json!({ "reason": "monitor" })));
        }
        match (asked.mode, asked.grade, asked.node) {
            (Mode::Off, _, _) => Ok(None),
            (mode, Some(grade), Some(node)) => Ok(Some(Highlight { mode, grade, node })),
            _ => Err(Fault::params(
                "a Highlight mode that is on shows the key of a node: give its grade and node",
            )
            .with_data(json!({ "reason": "node" }))),
        }
    }

    /// What `viewer.highlight` answers, of the mode `highlight` (none: off).
    #[must_use]
    pub fn written(highlight: Option<&Highlight>) -> Value {
        match highlight {
            None => json!({"monitor": "timeline", "mode": "off"}),
            Some(highlight) => json!({
                "monitor": "timeline",
                "mode": highlight.mode,
                "grade": highlight.grade,
                "node": highlight.node,
            }),
        }
    }

    /// The look of the node that the mode shows.
    fn look(&self) -> Look {
        Look {
            node: self.node.clone(),
            show: if self.mode == Mode::Bw {
                Show::Matte
            } else {
                Show::Highlight
            },
        }
    }

    /// Put the mode into `scene`, frame `frame` of timeline `index` of `project`, as `grading`
    /// graded it, its layers those of `clips`: the grade of the node shows its look, and for
    /// Highlight BW what is not of the key is taken away. Gives whether the node was found.
    pub fn show(
        &self,
        project: &Project,
        index: usize,
        frame: i64,
        grading: &Grading,
        scene: &mut Scene,
        clips: &mut Vec<String>,
    ) -> bool {
        let look = self.look();
        let alone = self.mode == Mode::Bw;
        let timeline = &project.timelines[index];
        let clip_group = |clip: &str| {
            timeline
                .tracks
                .iter()
                .flat_map(|track| &track.clips)
                .find(|found| found.id == clip)
                .and_then(|found| found.group.clone())
        };
        let level = match &self.grade {
            GradeTarget::Timeline { timeline: id } => {
                if *id != timeline.id {
                    return false;
                }
                let Some(stage) = grading.timeline_showing(frame, &look) else {
                    return false;
                };
                scene.grade = stage;
                if alone {
                    scene.output = Stage::default();
                    scene.subtitles.clear();
                }
                return true;
            }
            GradeTarget::Clip { timeline: id, .. } if *id != timeline.id => return false,
            GradeTarget::Clip { .. } => Level::Own,
            GradeTarget::PreClip { .. } => Level::Pre,
            GradeTarget::PostClip { .. } => Level::Post,
        };
        let of = |clip: &str| match &self.grade {
            GradeTarget::Clip { clip: named, .. } => named == clip,
            GradeTarget::PreClip { group } | GradeTarget::PostClip { group } => {
                clip_group(clip).as_deref() == Some(group.as_str())
            }
            GradeTarget::Timeline { .. } => false,
        };
        let mut shown = vec![false; clips.len()];
        for ((layer, clip), shown) in scene.layers.iter_mut().zip(clips.iter()).zip(&mut shown) {
            if !of(clip) {
                continue;
            }
            if let Some(stage) = grading.layer_showing(clip, frame, level, &look, alone) {
                layer.color.grade = stage;
                *shown = true;
            }
        }
        if !shown.contains(&true) {
            return false;
        }
        if alone {
            let mut kept = shown.iter();
            scene.layers.retain(|_| *kept.next().unwrap_or(&false));
            let mut kept = shown.iter();
            clips.retain(|_| *kept.next().unwrap_or(&false));
            for layer in &mut scene.layers {
                layer.opacity = 1.0;
            }
            scene.grade = Stage::default();
            scene.output = Stage::default();
            scene.subtitles.clear();
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(params: &str) -> Result<Option<Highlight>, Fault> {
        let raw = serde_json::value::RawValue::from_string(params.to_owned()).unwrap();
        Highlight::read(Some(&raw))
    }

    #[test]
    fn a_mode_is_read_with_the_node_it_shows_and_written_back() {
        assert_eq!(read(r#"{"mode": "off"}"#).unwrap(), None);
        assert_eq!(
            read(r#"{"mode": "off", "monitor": "timeline", "node": "n1"}"#).unwrap(),
            None
        );
        let grade = json!({"level": "clip", "timeline": "t1", "clip": "c1"});
        let on = read(&json!({"mode": "bw", "grade": grade, "node": "n2"}).to_string())
            .unwrap()
            .unwrap();
        assert_eq!(on.mode, Mode::Bw);
        assert_eq!(on.node, "n2");
        assert_eq!(on.look().show, Show::Matte);
        assert_eq!(
            Highlight::written(Some(&on)),
            json!({"monitor": "timeline", "mode": "bw", "grade": grade, "node": "n2"})
        );
        assert_eq!(
            Highlight::written(None),
            json!({"monitor": "timeline", "mode": "off"})
        );
        let highlight = read(
            &json!({"mode": "highlight", "grade": {"level": "timeline", "timeline": "t1"},
                "node": "n1"})
            .to_string(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(highlight.look().show, Show::Highlight);
        // As the state of the monitor tells it.
        assert_eq!(
            serde_json::to_value(&highlight).unwrap(),
            json!({"mode": "highlight", "grade": {"level": "timeline", "timeline": "t1"},
                "node": "n1"})
        );
    }

    #[test]
    fn a_mode_of_the_source_monitor_or_without_its_node_is_refused() {
        let reason = |params: &str| {
            let fault = read(params).unwrap_err();
            assert_eq!(fault.code, rpc::INVALID_PARAMS, "{params}");
            fault.data.map(|data| data["reason"].clone())
        };
        let grade = json!({"level": "timeline", "timeline": "t1"});
        assert_eq!(
            reason(
                &json!({"mode": "bw", "monitor": "source", "grade": grade, "node": "n1"})
                    .to_string()
            ),
            Some(json!("monitor"))
        );
        assert_eq!(
            reason(&json!({"mode": "bw", "grade": grade}).to_string()),
            Some(json!("node"))
        );
        assert_eq!(
            reason(r#"{"mode": "highlight", "node": "n1"}"#),
            Some(json!("node"))
        );
        for wrong in [
            r#"{"mode": "difference"}"#,
            r#"{"mode": "bw", "colour": 1}"#,
            "{}",
        ] {
            assert_eq!(
                read(wrong).unwrap_err().code,
                rpc::INVALID_PARAMS,
                "{wrong}"
            );
        }
    }
}
