//! What the timeline monitor shows for a while of the Color page, and the project does not
//! hold:
//!
//! - **Live Preview** (`viewer.preview`): "how the current clip will appear with the LUT you
//!   select applied to the currently selected node of the current grade" as the pointer is over
//!   a LUT of the LUT browser (Reference Manual 20, p. 3411), and, with Hover Scrub Preview,
//!   over "the duration of the current clip" (p. 3259): the frames are drawn from a copy of the
//!   project whose node has that LUT in place of its own, at the frame asked for where one is,
//!   in the preview alone. The project, its history and every other client see nothing of it;
//!   an export, a frame drawn off screen, the references of a wipe and a still grabbed are drawn
//!   as the project is. A node that cannot take the LUT (one that is not there, not a corrector,
//!   or locked) leaves the picture as it is, as a Highlight mode does. It ends with the
//!   connection that turned it on last (`Engine::leave`), which no client is left to end.
//! - **The Color Viewer LUT where the viewer is the Color page's** (`viewer.lut`): "The Color
//!   Viewer Lookup Table options … only affect the GUI Viewer in the Color page. They do not
//!   affect the viewers in the Edit page" (p. 672). The engine does not know the page that its
//!   view is on: the client turns the LUT off as it leaves the Color page and on as it comes
//!   back. It is on until it is turned off.

use std::sync::Arc;

use columbina_model::grade::Target as GradeTarget;
use columbina_model::Project;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use serde_json::{json, Value};

use crate::rpc::{self, Fault};
use crate::view::Monitor;

/// What the monitor shows in place of the project's grade of a node: a LUT of the library on it,
/// and the frame of the timeline shown where one is asked for (Hover Scrub Preview).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Preview {
    pub grade: GradeTarget,
    pub node: String,
    pub lut: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frame: Option<i64>,
}

/// The params of `viewer.preview`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Asked {
    #[serde(default)]
    monitor: Option<Monitor>,
    #[serde(default = "on")]
    on: bool,
    #[serde(default)]
    grade: Option<GradeTarget>,
    #[serde(default)]
    node: Option<String>,
    #[serde(default)]
    lut: Option<String>,
    #[serde(default)]
    frame: Option<i64>,
}

fn on() -> bool {
    true
}

/// The params of `viewer.lut`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Lut {
    #[serde(default)]
    monitor: Option<Monitor>,
    on: bool,
}

/// The refusal of the source monitor, which shows no grade.
fn of_the_source() -> Fault {
    Fault::params("the source monitor shows no grade: this is of the timeline monitor")
        .with_data(json!({ "reason": "monitor" }))
}

impl Preview {
    /// The preview that the params of `viewer.preview` ask for: none to end it.
    ///
    /// # Errors
    /// They are not those of the method (`-32602`): the source monitor (`reason: monitor`), a
    /// preview on without its grade, node and LUT (`reason: lut`), or a LUT that is not named
    /// relative to the folders of LUTs (`reason: name`).
    pub fn read(params: Option<&RawValue>) -> Result<Option<Preview>, Fault> {
        let asked: Asked = rpc::params(
            "viewer.preview",
            r#"{"grade": {"level": "clip", ...}, "node": "n1", "lut": "Film/Look.cube", "frame"?: 0, "on"?: true, "monitor"?: "timeline"} or {"on": false}"#,
            params,
        )?;
        if asked.monitor == Some(Monitor::Source) {
            return Err(of_the_source());
        }
        if !asked.on {
            return Ok(None);
        }
        let (Some(grade), Some(node), Some(lut)) = (asked.grade, asked.node, asked.lut) else {
            return Err(Fault::params(
                "a preview shows a LUT on a node: give its grade, its node and the LUT",
            )
            .with_data(json!({ "reason": "lut" })));
        };
        if crate::lut_preview::Request::read(Some(
            &RawValue::from_string(json!({ "luts": [lut] }).to_string())
                .map_err(|error| Fault::params(error.to_string()))?,
        ))
        .is_err()
        {
            return Err(Fault::params(format!(
                "{lut:?} is not a LUT named relative to the folders of LUTs"
            ))
            .with_data(json!({ "reason": "name" })));
        }
        Ok(Some(Preview {
            grade,
            node,
            lut,
            frame: asked.frame,
        }))
    }

    /// What `viewer.preview` answers, of the preview `preview` (none: off).
    #[must_use]
    pub fn written(preview: Option<&Preview>) -> Value {
        json!({"monitor": "timeline", "preview": preview})
    }

    /// `project` as the preview shows it: the node's LUT made the preview's; none where the node
    /// cannot take it.
    #[must_use]
    pub fn of(&self, project: &Project) -> Option<Project> {
        crate::lut_preview::with_lut(project, &self.grade, &self.node, &self.lut).ok()
    }
}

/// The project that a preview draws, made once for each project and preview.
#[derive(Default)]
pub struct Previewed {
    made: Option<(Arc<Project>, Preview, Arc<Project>)>,
}

impl Previewed {
    /// `project` as `preview` shows it (itself where there is none, or the node cannot take its
    /// LUT), made again only where the project or the preview is another.
    pub fn of(&mut self, project: &Arc<Project>, preview: Option<&Preview>) -> Arc<Project> {
        let Some(preview) = preview else {
            self.made = None;
            return Arc::clone(project);
        };
        if let Some((base, made_of, made)) = &self.made {
            if Arc::ptr_eq(base, project) && made_of == preview {
                return Arc::clone(made);
            }
        }
        let made = preview
            .of(project)
            .map_or_else(|| Arc::clone(project), Arc::new);
        self.made = Some((Arc::clone(project), preview.clone(), Arc::clone(&made)));
        made
    }
}

/// Whether a setting that is on unless turned off is on: where it is, it is not written.
#[allow(clippy::trivially_copy_pass_by_ref, reason = "as serde asks it")]
pub fn is_on(on: &bool) -> bool {
    *on
}

/// Whether the params of `viewer.lut` turn the Color Viewer LUT on.
///
/// # Errors
/// They are not those of the method (`-32602`), or name the source monitor (`reason:
/// monitor`), which has no Viewer LUT.
pub fn viewer_lut(params: Option<&RawValue>) -> Result<bool, Fault> {
    let asked: Lut = rpc::params(
        "viewer.lut",
        r#"{"on": true | false, "monitor"?: "timeline"}"#,
        params,
    )?;
    if asked.monitor == Some(Monitor::Source) {
        return Err(of_the_source());
    }
    Ok(asked.on)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(params: &Value) -> Box<RawValue> {
        RawValue::from_string(params.to_string()).unwrap()
    }

    fn reason(fault: Fault) -> Option<Value> {
        assert_eq!(fault.code, rpc::INVALID_PARAMS);
        fault.data.map(|data| data["reason"].clone())
    }

    #[test]
    fn a_preview_is_read_with_its_node_and_lut_and_written_back() {
        let grade = json!({"level": "clip", "timeline": "t1", "clip": "c1"});
        let preview = Preview::read(Some(&raw(&json!({
            "grade": grade, "node": "n2", "lut": "Film/Look.cube", "frame": 30,
        }))))
        .unwrap()
        .unwrap();
        assert_eq!((preview.node.as_str(), preview.frame), ("n2", Some(30)));
        assert_eq!(
            Preview::written(Some(&preview)),
            json!({"monitor": "timeline", "preview": {"grade": grade, "node": "n2",
                "lut": "Film/Look.cube", "frame": 30}})
        );
        assert_eq!(
            Preview::read(Some(&raw(&json!({"on": false})))).unwrap(),
            None
        );
        assert_eq!(
            Preview::written(None),
            json!({"monitor": "timeline", "preview": null})
        );
        for (params, why) in [
            (json!({"on": false, "monitor": "source"}), Some("monitor")),
            (json!({"grade": grade, "node": "n2"}), Some("lut")),
            (json!({"node": "n2", "lut": "a.cube"}), Some("lut")),
            (
                json!({"grade": grade, "node": "n2", "lut": "../a.cube"}),
                Some("name"),
            ),
            (
                json!({"grade": grade, "node": "n2", "lut": "C:/a.cube"}),
                Some("name"),
            ),
            (json!({"on": false, "colour": 1}), None),
        ] {
            let fault = Preview::read(Some(&raw(&params))).unwrap_err();
            assert_eq!(reason(fault), why.map(|why| json!(why)), "{params}");
        }
    }

    #[test]
    fn the_viewer_lut_is_turned_on_and_off_of_the_timeline_monitor() {
        assert!(viewer_lut(Some(&raw(&json!({"on": true})))).unwrap());
        assert!(!viewer_lut(Some(&raw(&json!({"on": false, "monitor": "timeline"})))).unwrap());
        assert_eq!(
            reason(viewer_lut(Some(&raw(&json!({"on": false, "monitor": "source"})))).unwrap_err()),
            Some(json!("monitor"))
        );
        assert!(viewer_lut(Some(&raw(&json!({})))).is_err());
    }

    #[test]
    fn a_previewed_project_is_made_once_for_each_project_and_preview() {
        let mut project = Project::sample();
        let grade = GradeTarget::Timeline {
            timeline: project.timelines[0].id.clone(),
        };
        let node = serde_json::to_value(columbina_model::Node::corrector("n1")).unwrap();
        let add: columbina_model::Op = serde_json::from_value(json!({
            "op": "add_node", "grade": grade, "node": node, "links": [
                {"from": "source", "to": "n1", "kind": "rgb", "input": 0},
                {"from": "n1", "to": "output", "kind": "rgb", "input": 0},
            ],
        }))
        .unwrap();
        columbina_model::apply(&mut project, add).unwrap();
        let project = Arc::new(project);
        let preview = Preview {
            grade: grade.clone(),
            node: "n1".into(),
            lut: "a.cube".into(),
            frame: None,
        };
        let mut previewed = Previewed::default();
        assert!(Arc::ptr_eq(&previewed.of(&project, None), &project));
        let made = previewed.of(&project, Some(&preview));
        assert!(!Arc::ptr_eq(&made, &project));
        assert_eq!(
            serde_json::to_value(&made.timelines[0].grade.nodes[0]).unwrap()["params"]["lut"],
            "a.cube"
        );
        // The same again: the same copy.
        assert!(Arc::ptr_eq(&previewed.of(&project, Some(&preview)), &made));
        // Another LUT, or another project: another.
        let other = Preview {
            lut: "b.cube".into(),
            ..preview.clone()
        };
        assert!(!Arc::ptr_eq(&previewed.of(&project, Some(&other)), &made));
        // A node that is not there: the project as it is.
        let gone = Preview {
            node: "n9".into(),
            ..preview
        };
        assert!(Arc::ptr_eq(&previewed.of(&project, Some(&gone)), &project));
    }
}
