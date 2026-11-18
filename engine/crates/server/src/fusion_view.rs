//! The Fusion view of the timeline monitor (`viewer.fusion`): the picture of one node of the
//! composition of a clip, alone, in place of the frame of the timeline, as the viewers of the
//! Fusion page of `DaVinci Resolve` show the node that is viewed (Reference Manual 20,
//! pp. 1211–1274); and where that picture lies in the frame, so that a page can draw the
//! controls of a node over it (`view.overlay`) and read the pointer on it (`view.pointer`).
//!
//! # What is drawn
//!
//! Of the frame at the playhead, the layer of the clip alone, showing what its composition
//! gives for the node (the model's `Composition::at_node`): the node and all that leads to
//! it, and nothing after it. With no node, or with the `media_out`, it is what comes out of
//! the composition. A clip that has no composition is taken to have the one that a clip has
//! at first, a `media_in` linked to a `media_out`, so that it shows its source.
//!
//! The layer is not what the timeline shows of the clip: it is not stabilized (the stabilizer
//! of the clip is done after the composition, and the Fusion page sees the picture as it was
//! shot), it is not moved, cut, blended or faded (the transform, the crop, the composite mode
//! and the opacity of the Edit page are left out, with the fades and the transition that the
//! clip is in), and it is not graded (the grade of the clip, those of its group and that of
//! the timeline are left out). No other layer is drawn, and no subtitle. The picture is of the
//! size of the composition (the model's `fusion::size_of`), fitted whole into the frame of the
//! timeline, in its middle
//! (`columbina_compose::geometry::fitted`), over black: where the two are not of one shape
//! there is black at its sides, or above and below it, and what is transparent in it shows
//! black. The source of the clip goes through its input transform before the composition, as
//! it always does, and the frame through the output transform of the project after it: the
//! colour of the view is that of the timeline's picture without its grades.
//!
//! Where the clip is not seen at the frame (the playhead is not on it, it or its track is
//! off, its media is not in the project), or the node is no longer in its composition, the
//! frame is black, and the view stays on: it shows the picture again when there is one.
//!
//! # With the other modes of the viewer
//!
//! While the Fusion view is on, it is all that the monitor shows: a Highlight mode, Bypass
//! Color, a Live Preview of a LUT, and the wipe or the split screen of a comparison are not
//! drawn, and are as they were when it is turned off. The Window controls of the Color page
//! are not drawn over it. A still that is grabbed is of the frame of the timeline as it is
//! graded, not of the view.

use columbina_compose::geometry;
use columbina_model::grade::Composite;
use columbina_model::project::{Cropped, Placed, TrackKind};
use columbina_model::{fusion, resolve, Clip, Project, Timeline};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::value::RawValue;
use serde_json::{json, Value};

use crate::rpc::{self, Fault};
use crate::view::Monitor;

/// What a Fusion view shows: the picture of a node of the composition of a clip.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct View {
    /// The clip, of a track of picture of the timeline shown.
    pub clip: String,
    /// The node of its composition whose picture is shown; none for what comes out of the
    /// composition, which its `media_out` names too.
    #[serde(default)]
    pub node: Option<String>,
}

/// The size of the pictures of a composition, in pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

/// A rectangle of the frame drawn, in its pixels from its top left corner, as `view.pointer`
/// tells them and `view.overlay` takes them.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Its numbers are made of the sizes of pictures, and each is a number: equal is equal.
impl Eq for Rect {}

/// A Fusion view as the state of the monitor tells it (`fusion`), and as `viewer.fusion` and
/// a frame drawn off screen for it answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Told {
    /// The clip.
    pub clip: String,
    /// The node, where one was named.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
    /// Whether the picture is drawn at the frame: the clip is seen there, and the node is in
    /// its composition. The frame is black where it is not.
    pub shown: bool,
    /// The width and the height of the frame drawn, which `picture` is in the pixels of: those
    /// of the timeline.
    pub canvas: [u32; 2],
    /// The size of the pictures of the composition; left out where the clip is not in the
    /// timeline any more.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<Size>,
    /// Where the picture of the composition lies in the frame drawn; left out where the clip
    /// is not in the timeline any more.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub picture: Option<Rect>,
}

/// The params of `viewer.fusion`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Asked {
    #[serde(default)]
    monitor: Option<Monitor>,
    /// Written always: `null` turns the view off.
    #[serde(deserialize_with = "given")]
    clip: Option<String>,
    #[serde(default)]
    node: Option<String>,
}

/// A field that must be written, and may be `null`.
fn given<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    Option::<String>::deserialize(deserializer)
}

/// The refusal of a view of what the timeline has not: `reason`, and the id that it names.
fn missing(reason: &str, named: &str, id: &str, message: String) -> Fault {
    Fault::params(message).with_data(json!({ "reason": reason, named: id }))
}

impl View {
    /// The view that the params of `viewer.fusion` ask for: none to turn it off.
    ///
    /// # Errors
    /// The params are not those of the method (`-32602`), or name the source monitor
    /// (`reason: monitor`), which shows media and no timeline.
    pub fn read(params: Option<&RawValue>) -> Result<Option<View>, Fault> {
        let asked: Asked = rpc::params(
            "viewer.fusion",
            r#"{"clip": "c1" | null, "node"?: "n4", "monitor"?: "timeline"}"#,
            params,
        )?;
        if asked.monitor == Some(Monitor::Source) {
            return Err(Fault::params(
                "the source monitor shows no composition: the Fusion view is of the timeline \
                 monitor",
            )
            .with_data(json!({ "reason": "monitor" })));
        }
        Ok(asked.clip.map(|clip| View {
            clip,
            node: asked.node,
        }))
    }

    /// What `viewer.fusion` answers, of the view `told` (none: off).
    #[must_use]
    pub fn written(told: Option<&Told>) -> Value {
        json!({"monitor": "timeline", "fusion": told})
    }

    /// The clip of the view, where it is a clip of a track of picture of `timeline`.
    fn clip_of<'t>(&self, timeline: &'t Timeline) -> Option<&'t Clip> {
        timeline
            .tracks
            .iter()
            .filter(|track| track.kind == TrackKind::Video)
            .flat_map(|track| &track.clips)
            .find(|clip| clip.id == self.clip)
    }

    /// Check that `timeline` has what the view names: its clip, on a track of picture, and its
    /// node in the composition of that clip (in the one that a clip has at first, where the
    /// clip has none).
    ///
    /// # Errors
    /// `-32602`: there is no such clip in the timeline (`reason: clip`), it is a clip of sound
    /// (`wrong_track`), or its composition has no such node (`node`).
    pub fn check(&self, timeline: &Timeline) -> Result<(), Fault> {
        let id = &self.clip;
        let Some(clip) = self.clip_of(timeline) else {
            // A clip of the timeline that is on no track of picture is one of sound.
            let sound = timeline
                .tracks
                .iter()
                .flat_map(|track| &track.clips)
                .any(|clip| clip.id == *id);
            let (reason, message) = if sound {
                let message = format!("clip {id} is a clip of sound: it has no composition");
                ("wrong_track", message)
            } else {
                let message = format!("there is no clip {id} in timeline {}", timeline.id);
                ("clip", message)
            };
            return Err(missing(reason, "clip", id, message));
        };
        let first = fusion::Composition::default();
        let composition = clip.fusion.as_ref().unwrap_or(&first);
        match &self.node {
            Some(node) if composition.node(node).is_none() => {
                let message = format!("there is no node {node} in the composition of clip {id}");
                Err(missing("node", "node", node, message))
            }
            _ => Ok(()),
        }
    }

    /// The one layer that the view draws at frame `frame` of `timeline`, of `layers`, those
    /// that the timeline shows there (`resolve::video_at`): that of its clip, showing what the
    /// composition of the clip gives for the node, not stabilized, not moved, whole, laid as it
    /// is and opaque.
    /// None where the clip is not seen at the frame, or its composition has no such node.
    pub fn layer(
        &self,
        project: &Project,
        timeline: &Timeline,
        frame: i64,
        layers: Vec<resolve::Layer>,
    ) -> Option<resolve::Layer> {
        let clip = self.clip_of(timeline)?;
        let mut layer = layers.into_iter().find(|layer| layer.clip == self.clip)?;
        let fused = resolve::fused_for(timeline, project, clip, frame, self.node.as_deref())?;
        layer.fusion = Some(fused);
        layer.stabilize = None;
        layer.transform = Placed::NONE;
        layer.crop = Cropped::NONE;
        layer.composite = Composite::Normal;
        layer.opacity = 1.0;
        Some(layer)
    }

    /// The view as it is told, of `timeline`, the picture drawn at the frame or not (`shown`).
    pub fn told(&self, project: &Project, timeline: &Timeline, shown: bool) -> Told {
        let size = self
            .clip_of(timeline)
            .map(|clip| fusion::size_of(clip, project, timeline));
        let picture = size.and_then(|size| Rect::fitted(size, (timeline.width, timeline.height)));
        Told {
            clip: self.clip.clone(),
            node: self.node.clone(),
            shown: shown && picture.is_some(),
            canvas: [timeline.width, timeline.height],
            size: size.map(|(width, height)| Size { width, height }),
            picture,
        }
    }

    /// The view as it is told at frame `frame` of `timeline`, for a monitor that does not draw
    /// the frame now.
    pub fn state(&self, project: &Project, timeline: &Timeline, frame: i64) -> Told {
        let layers = resolve::video_at(timeline, project, frame);
        let shown = self.layer(project, timeline, frame, layers).is_some();
        self.told(project, timeline, shown)
    }
}

impl Told {
    /// The view that is told.
    #[must_use]
    pub fn view(&self) -> View {
        View {
            clip: self.clip.clone(),
            node: self.node.clone(),
        }
    }
}

impl Rect {
    /// Where a picture of `size` pixels lies in a frame of `canvas` pixels, fitted whole and
    /// in its middle, as a layer that is not moved shows it; none where either has no pixels.
    fn fitted(size: (u32, u32), canvas: (u32, u32)) -> Option<Rect> {
        let [x, y, width, height] = geometry::fitted(
            (f64::from(canvas.0), f64::from(canvas.1)),
            (f64::from(size.0), f64::from(size.1)),
        )?;
        Some(Rect {
            x,
            y,
            width,
            height,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use columbina_model::fusion::{Composition, Input, Kind, Link, Node};

    fn read(params: &Value) -> Result<Option<View>, Fault> {
        let raw = RawValue::from_string(params.to_string()).unwrap();
        View::read(Some(&raw))
    }

    fn reason(fault: Fault) -> Option<Value> {
        assert_eq!(fault.code, rpc::INVALID_PARAMS);
        fault.data.map(|data| data["reason"].clone())
    }

    fn view(clip: &str, node: Option<&str>) -> View {
        View {
            clip: clip.into(),
            node: node.map(str::to_owned),
        }
    }

    /// The sample (c1 of m1, 1920 × 1080, on V1 over frames 0 to 99, and c2 on A1), its clip
    /// of picture with a Background under its picture and a mask that is not linked.
    fn composed() -> Project {
        let mut project = Project::sample();
        let mut composition = Composition {
            nodes: vec![
                Node::new("in", Kind::MediaIn),
                Node::new("bg", Kind::Background),
                Node::new("over", Kind::Merge),
                Node::new("r", Kind::Rectangle),
                Node::new("out", Kind::MediaOut),
            ],
            links: vec![
                Link::new("bg", "over", Input::Background),
                Link::new("in", "over", Input::Foreground),
                Link::new("over", "out", Input::Input),
            ],
        };
        composition.sort_links();
        project.timelines[0].tracks[0].clips[0].fusion = Some(composition);
        project.validate().unwrap();
        project
    }

    #[test]
    fn a_view_is_read_with_its_clip_and_its_node_and_turned_off_by_null() {
        let on = read(&json!({"clip": "c1"})).unwrap();
        assert_eq!(on, Some(view("c1", None)));
        let of_a_node = json!({"clip": "c1", "node": "n4", "monitor": "timeline"});
        assert_eq!(read(&of_a_node).unwrap(), Some(view("c1", Some("n4"))));
        assert_eq!(read(&json!({"clip": null})).unwrap(), None);
        let off = json!({"clip": null, "node": "n4"});
        assert_eq!(read(&off).unwrap(), None);
        for (params, why) in [
            (json!({"clip": "c1", "monitor": "source"}), Some("monitor")),
            (json!({"clip": null, "monitor": "source"}), Some("monitor")),
            // The clip is written, though it be null.
            (json!({}), None),
            (json!({"node": "n4"}), None),
            (json!({"clip": "c1", "at": "input"}), None),
            (json!({"clip": 1}), None),
        ] {
            let fault = read(&params).unwrap_err();
            assert_eq!(reason(fault), why.map(|why| json!(why)), "{params}");
        }
        assert_eq!(
            View::written(None),
            json!({"monitor": "timeline", "fusion": null})
        );
    }

    #[test]
    fn a_view_names_a_clip_of_picture_of_the_timeline_and_a_node_of_its_composition() {
        let project = composed();
        let timeline = &project.timelines[0];
        for node in [None, Some("in"), Some("over"), Some("r"), Some("out")] {
            assert!(view("c1", node).check(timeline).is_ok(), "{node:?}");
        }
        let refused = |clip: &str, node: Option<&str>| {
            let fault = view(clip, node).check(timeline).unwrap_err();
            let data = fault.data.clone().unwrap();
            (reason(fault), data)
        };
        let (why, data) = refused("c1", Some("n404"));
        assert_eq!(why, Some(json!("node")));
        assert_eq!(data["node"], "n404");
        let (why, data) = refused("c404", None);
        assert_eq!(why, Some(json!("clip")));
        assert_eq!(data["clip"], "c404");
        // A clip of sound has no composition.
        assert_eq!(refused("c2", None).0, Some(json!("wrong_track")));
        // A clip without a composition has the nodes of the one that a clip has at first.
        let plain = Project::sample();
        let timeline = &plain.timelines[0];
        for node in [None, Some(fusion::MEDIA_IN), Some(fusion::MEDIA_OUT)] {
            assert!(view("c1", node).check(timeline).is_ok(), "{node:?}");
        }
        let fault = view("c1", Some("over")).check(timeline).unwrap_err();
        assert_eq!(reason(fault), Some(json!("node")));
    }

    #[test]
    fn the_layer_of_a_view_is_that_of_its_clip_alone_made_for_the_node_and_laid_as_it_is() {
        let mut project = composed();
        {
            let clip = &mut project.timelines[0].tracks[0].clips[0];
            clip.transform.x = 0.25.into();
            clip.transform.opacity = 0.5.into();
            clip.crop.left = 0.1.into();
            clip.composite = Composite::Screen;
        }
        project.validate().unwrap();
        let timeline = &project.timelines[0];
        let clip = &timeline.tracks[0].clips[0];
        let layers = || resolve::video_at(timeline, &project, 10);
        let shown = layers().into_iter().next().unwrap();
        assert_ne!(shown.transform, Placed::NONE);
        let layer = view("c1", Some("bg"))
            .layer(&project, timeline, 10, layers())
            .unwrap();
        assert_eq!(layer.clip, "c1");
        assert_eq!(layer.content, shown.content);
        assert_eq!(layer.transform, Placed::NONE);
        assert_eq!(layer.crop, Cropped::NONE);
        assert_eq!(layer.composite, Composite::Normal);
        assert!((layer.opacity - 1.0).abs() < f32::EPSILON);
        assert_eq!(
            layer.fusion,
            resolve::fused_for(timeline, &project, clip, 10, Some("bg"))
        );
        let ids = |layer: &resolve::Layer| -> Vec<String> {
            let fused = layer.fusion.as_ref().unwrap();
            fused.nodes.iter().map(|node| node.id.clone()).collect()
        };
        assert_eq!(ids(&layer), ["bg"]);
        // With no node, and with the MediaOut, what comes out of the composition.
        for node in [None, Some("out")] {
            let layer = view("c1", node)
                .layer(&project, timeline, 10, layers())
                .unwrap();
            assert_eq!(layer.fusion, shown.fusion, "{node:?}");
        }
        // A node that the MediaOut does not read has its picture too.
        let mask = view("c1", Some("r"))
            .layer(&project, timeline, 10, layers())
            .unwrap();
        assert_eq!(ids(&mask), ["r"]);
        // No layer where the playhead is not on the clip, where the node is gone, and of a
        // clip that the timeline has not.
        let beyond = resolve::video_at(timeline, &project, 100);
        assert_eq!(
            view("c1", None).layer(&project, timeline, 100, beyond),
            None
        );
        assert_eq!(
            view("c1", Some("n404")).layer(&project, timeline, 10, layers()),
            None
        );
        assert_eq!(
            view("c404", None).layer(&project, timeline, 10, layers()),
            None
        );
    }

    #[test]
    fn a_view_is_told_with_the_size_of_its_composition_and_where_its_picture_lies() {
        let mut project = composed();
        // A timeline of 4:3 shows the picture of 16:9 with black above and below it.
        (project.timelines[0].width, project.timelines[0].height) = (1440, 1080);
        let timeline = &project.timelines[0];
        let told = view("c1", Some("over")).state(&project, timeline, 10);
        assert_eq!(
            serde_json::to_value(&told).unwrap(),
            json!({
                "clip": "c1", "node": "over", "shown": true, "canvas": [1440, 1080],
                "size": {"width": 1920, "height": 1080},
                "picture": {"x": 0.0, "y": 135.0, "width": 1440.0, "height": 810.0},
            })
        );
        assert_eq!(told.view(), view("c1", Some("over")));
        assert_eq!(
            View::written(Some(&told))["fusion"]["picture"]["y"],
            json!(135.0)
        );
        // The playhead off the clip: the picture is not shown, and lies where it lay.
        let off = view("c1", None).state(&project, timeline, 100);
        assert_eq!(
            serde_json::to_value(&off).unwrap(),
            json!({
                "clip": "c1", "shown": false, "canvas": [1440, 1080],
                "size": {"width": 1920, "height": 1080},
                "picture": {"x": 0.0, "y": 135.0, "width": 1440.0, "height": 810.0},
            })
        );
        // A node that is gone, and a clip that is gone: not shown; of the clip, nothing more.
        assert!(!view("c1", Some("n404")).state(&project, timeline, 10).shown);
        assert_eq!(
            serde_json::to_value(view("c404", None).state(&project, timeline, 10)).unwrap(),
            json!({"clip": "c404", "shown": false, "canvas": [1440, 1080]})
        );
        // In a timeline of 1920 × 1080, a composition of 4:3 has black at its sides.
        let mut narrow = composed();
        let video = narrow.media[0].facts.video.as_mut().unwrap();
        (video.width, video.height) = (1440, 1080);
        let timeline = &narrow.timelines[0];
        let told = view("c1", None).state(&narrow, timeline, 0);
        assert_eq!(
            told.picture,
            Some(Rect {
                x: 240.0,
                y: 0.0,
                width: 1440.0,
                height: 1080.0
            })
        );
        assert_eq!(
            told.size,
            Some(Size {
                width: 1440,
                height: 1080
            })
        );
    }
}
