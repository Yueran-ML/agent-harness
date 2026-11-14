//! A frame drawn apart from the monitors, to be measured: `view.sample`, a few pixels of the
//! picture where a picker of colour is put on it, and `scopes.statistics` of a frame of a
//! timeline drawn off screen (for an agent whose engine shows nothing) or of the picture at a node
//! of a grade (as the Curves palette's histogram and Auto Color want it), as `PROTOCOL.md` tells
//! them (The view, Scopes).
//!
//! # A frame off screen
//!
//! A frame of a timeline is drawn as an export draws it ([`crate::render::scene`] and
//! [`Grading`], as the comparisons of the viewer draw theirs): its pictures read from the
//! original media by decoders of its own, through their input transforms, graded, and through
//! the output transform of the project's colour management, as the monitor shows it; without
//! subtitles. Its statistics measure luma by the output space, as the monitor's scopes do.
//!
//! # The picture at a node
//!
//! The picture that goes into a node, or comes out of it, is drawn by the grade cut at the node: a
//! grade of the node and of all that leads to it, linked to the output from the node (its output),
//! or from what is linked into its first input (its input; from the source of the grade, the grade
//! is none). The grades before it are done as they are, and none after it: for a node of a clip's
//! grade, its group's pre-clip grade and the clip's grade up to the node; for one of the group's
//! post-clip grade, both of those whole and that grade up to the node. Of the frame, only the
//! layer of the clip is drawn (the clip shown on top that is in the group, for a grade of a group,
//! unless the clip is named), where it lies on the canvas, as the node sees it: not laid over the
//! other layers, nor graded by the timeline. For a node of the timeline's grade, every layer is
//! drawn and graded, laid together, and the timeline's grade done up to the node. Each is in the
//! working space of the timeline, as the node sees it, through the input transforms and before
//! the output transform (and its statistics measure luma by the timeline's primaries).
//!
//! A node that leads to the node only by its key, and to nothing else, has nowhere for its picture
//! to go in the grade cut there; that grade is not complete, and it is not drawn (`node`).

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, PoisonError};

use columbina_compose::sample::{self, Region};
use columbina_compose::{Compositor, Gpu, Scene, Target};
use columbina_media::Hardware;
use columbina_model::grade::{Grade, Link, Port, Target as GradeTarget, OUTPUT, SOURCE};
use columbina_model::{resolve, Project};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::gpu::Graphics;
use crate::grading::{Grading, LutPlaces};
use crate::play::sources::Sources;
use crate::rpc::{self, Fault};

/// The most pixels on a side of the square that `view.sample` averages.
pub const MOST_SIZE: u32 = 31;

/// Where at a node a picture is taken.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Place {
    /// What goes into it.
    Input,
    /// What comes out of it.
    #[default]
    Output,
    /// Its key, as Highlight BW shows it (`crate::highlight`): white where it is full and
    /// black where it is none, in R, G and B alike.
    Key,
}

/// A node of a grade, and where at it a picture is taken.
#[derive(Clone, Debug, PartialEq)]
pub struct NodeAt {
    pub grade: GradeTarget,
    pub node: String,
    pub place: Place,
    /// For a grade of a group: the clip whose layer is drawn, rather than the one on top.
    pub clip: Option<String>,
}

impl NodeAt {
    /// As the params of a method write it.
    pub fn written(&self) -> Value {
        json!({
            "grade": self.grade,
            "node": self.node,
            "at": self.place,
        })
    }
}

/// What draws frames apart from the monitors: its own compositor, grades and decoders, made on
/// the engine's card as they are first wanted, and again on a card made in place of one lost.
pub struct Probe {
    graphics: Arc<Graphics>,
    luts: LutPlaces,
    drawing: Mutex<Option<Drawing>>,
}

struct Drawing {
    gpu: Arc<Gpu>,
    compositor: Compositor,
    grading: Grading,
    sources: Sources,
}

/// A frame drawn: the target it is in, on its card.
pub struct Drawn {
    pub gpu: Arc<Gpu>,
    pub target: Arc<Target>,
    /// What it is measured by: the signal of the output space, or of the timeline's working
    /// space for a picture at a node.
    pub signal: columbina_compose::scopes::Signal,
}

/// The refusal of params that do not name what there is.
fn missing(reason: &str, message: String) -> Fault {
    Fault::params(message).with_data(json!({ "reason": reason }))
}

/// The refusal of a frame that cannot be drawn.
pub fn drawing(message: &str) -> Fault {
    Fault::new(rpc::TRANSPORT_FAILED, message).with_data(json!({ "reason": "draw" }))
}

impl Probe {
    pub fn new(graphics: Arc<Graphics>, luts: LutPlaces) -> Probe {
        Probe {
            graphics,
            luts,
            drawing: Mutex::new(None),
        }
    }

    /// Draw frame `frame` of the timeline `timeline` of `project`, as an export draws it, or the
    /// picture at a node of a grade of it. This blocks, and frames are drawn one at a time: call
    /// it where blocking is allowed.
    ///
    /// # Errors
    /// The timeline, the grade, the node or the clip is not there (`-32602`), or the card could
    /// not draw the frame (`-32004`, `draw`).
    pub fn draw(
        &self,
        project: &Project,
        timeline: &str,
        frame: i64,
        node: Option<&NodeAt>,
    ) -> Result<Drawn, Fault> {
        let index = project
            .timelines
            .iter()
            .position(|found| found.id == timeline)
            .ok_or_else(|| {
                missing(
                    "timeline",
                    format!("there is no timeline {timeline} in the project"),
                )
            })?;
        let at_node = node.is_some();
        let (project, only) = match node {
            Some(node) => cut_at(project, index, frame, node)?,
            None => (project.clone(), None),
        };
        let project = Arc::new(project);
        self.with_drawing(|gpu, drawn| {
            let timeline = &project.timelines[index];
            let (mut scene, mut clips) =
                crate::render::scene(&project, timeline, frame, &drawn.sources)
                    .map_err(|message| drawing(&message))?;
            if let Some(only) = &only {
                let mut kept = clips.iter().map(|clip| clip == only);
                scene.layers.retain(|_| kept.next().unwrap_or(false));
                clips.retain(|clip| clip == only);
            }
            drawn.grading.update(gpu, &project, index);
            drawn.grading.grade(&mut scene, &clips, frame);
            if let Some(node) = node.filter(|node| node.place == Place::Key) {
                let matte = crate::highlight::Highlight {
                    mode: crate::highlight::Mode::Bw,
                    grade: node.grade.clone(),
                    node: node.node.clone(),
                };
                if !matte.show(
                    &project,
                    index,
                    frame,
                    &drawn.grading,
                    &mut scene,
                    &mut clips,
                ) {
                    return Err(missing(
                        "node",
                        format!("node {} gives no key in its grade", node.node),
                    ));
                }
            }
            let output = drawn.grading.output();
            let signal = if at_node {
                // The node sees the working space: no output transform.
                scene.output = columbina_compose::Stage::default();
                columbina_compose::scopes::Signal {
                    luma: drawn.grading.luma(),
                    ..output.signal
                }
            } else {
                output.signal
            };
            Ok((scene, signal))
        })
    }

    /// Draw frame `frame` of the timeline `timeline` of `project` small, `height` pixels high at
    /// most (as high as the timeline at most), for a thumbnail: as an export draws it; or, with
    /// `clip`, that clip's layer alone, fitted to the frame and opaque, graded by its own grades
    /// (its group's and its own) and not the timeline's, through the output transform, "the
    /// media as it's currently graded" (p. 3026), on a track that is off too, and nothing of the
    /// other tracks read. This blocks, as [`Probe::draw`] does.
    ///
    /// # Errors
    /// The timeline is not there, or the clip is not seen at the frame (`-32602`), or the card
    /// could not draw it (`-32004`, `draw`).
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        reason = "a size of pixels"
    )]
    pub fn draw_small(
        &self,
        project: &Project,
        timeline: &str,
        frame: i64,
        clip: Option<&str>,
        height: u32,
    ) -> Result<Drawn, Fault> {
        let index = project
            .timelines
            .iter()
            .position(|found| found.id == timeline)
            .ok_or_else(|| {
                missing(
                    "timeline",
                    format!("there is no timeline {timeline} in the project"),
                )
            })?;
        let project = Arc::new(project.clone());
        self.with_drawing(|gpu, drawn| {
            let timeline = &project.timelines[index];
            // A clip alone: its own layers, read by themselves, on a track that is off too.
            let layers = match clip {
                Some(clip) => {
                    let layers = crate::render::clip_layers(&project, timeline, clip, frame);
                    if layers.is_empty() {
                        return Err(missing(
                            "clip",
                            format!("clip {clip} is not seen at frame {frame} of the timeline"),
                        ));
                    }
                    layers
                }
                None => resolve::video_at(timeline, &project, frame),
            };
            let (mut scene, clips) =
                crate::render::scene_of(&project, timeline, layers, &drawn.sources)
                    .map_err(|message| drawing(&message))?;
            drawn.grading.update(gpu, &project, index);
            drawn.grading.grade(&mut scene, &clips, frame);
            if clip.is_some() {
                for layer in &mut scene.layers {
                    layer.transform = columbina_compose::Transform::NONE;
                    layer.opacity = 1.0;
                }
                scene.grade = columbina_compose::Stage::default();
            }
            // Small: the grades' neighbourhoods and the titles scale with the frame.
            let high = height.clamp(1, scene.height.max(1));
            let wide = ((f64::from(scene.width) * f64::from(high) / f64::from(scene.height.max(1)))
                .round() as u32)
                .max(1);
            scene.units = high as f32 / scene.height.max(1) as f32;
            (scene.width, scene.height) = (wide, high);
            Ok((scene, drawn.grading.output().signal))
        })
    }

    /// Draw the scene that `make` makes with what draws frames on the card, made where it is not
    /// there or is of another card.
    fn with_drawing(
        &self,
        make: impl FnOnce(
            &Arc<Gpu>,
            &mut Drawing,
        ) -> Result<(Scene, columbina_compose::scopes::Signal), Fault>,
    ) -> Result<Drawn, Fault> {
        let gpu = self
            .graphics
            .gpu()
            .map_err(|error| drawing(&error.to_string()))?;
        let mut locked = self.drawing.lock().unwrap_or_else(PoisonError::into_inner);
        if locked
            .as_ref()
            .is_none_or(|drawing| !Arc::ptr_eq(&drawing.gpu, &gpu))
        {
            *locked = Some(Drawing {
                gpu: Arc::clone(&gpu),
                compositor: Compositor::new(Arc::clone(&gpu)),
                grading: Grading::new(self.luts.clone()),
                sources: Sources::new(Hardware::Auto, Arc::new(|_, _| {})),
            });
        }
        let drawn = locked.as_mut().expect("made above");
        let (scene, signal) = make(&gpu, drawn)?;
        let target = Target::preview(&gpu, scene.width, scene.height)
            .map_err(|error| drawing(&error.to_string()))?;
        let _turn = self.graphics.draw_turn(false);
        drawn
            .compositor
            .render(&scene, &target)
            .map_err(|error| drawing(&error.to_string()))?;
        Ok(Drawn {
            gpu,
            target: Arc::new(target),
            signal,
        })
    }

    /// A picture shown by itself, as the source monitor shows it, drawn on the card.
    ///
    /// # Errors
    /// The card could not draw it.
    pub fn draw_picture(
        &self,
        picture: Arc<columbina_media::NativePicture>,
        rotation: i16,
    ) -> Result<Drawn, Fault> {
        self.with_drawing(|_, _| {
            let (width, height) =
                columbina_compose::geometry::shown_size((picture.width, picture.height), rotation);
            Ok((
                Scene {
                    width,
                    height,
                    layers: vec![crate::render::picture_layer(picture, rotation)],
                    ..Scene::default()
                },
                columbina_compose::scopes::Signal::default(),
            ))
        })
    }
}

/// The square of `size` pixels about the pixel at `(x, y)` of a frame of `width` × `height`,
/// within the frame.
///
/// # Errors
/// The point is not on the frame, or the size is not an odd number from 1 to [`MOST_SIZE`].
pub fn square(x: f64, y: f64, size: u32, width: u32, height: u32) -> Result<Region, Fault> {
    if size == 0 || size.is_multiple_of(2) || size > MOST_SIZE {
        return Err(missing(
            "size",
            format!("the size of a sample is an odd number from 1 to {MOST_SIZE}, not {size}"),
        ));
    }
    let on = |at: f64, most: u32| at.is_finite() && at >= 0.0 && at < f64::from(most);
    if !on(x, width) || !on(y, height) {
        return Err(missing(
            "outside",
            format!("({x}, {y}) is not on the frame of {width}×{height}"),
        ));
    }
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "within the frame, checked above"
    )]
    let (px, py) = (x.floor() as u32, y.floor() as u32);
    let half = size / 2;
    let left = px.saturating_sub(half);
    let top = py.saturating_sub(half);
    let right = (px + half + 1).min(width);
    let bottom = (py + half + 1).min(height);
    Ok(Region {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    })
}

/// The mean R'G'B' of `region` of a frame drawn.
///
/// # Errors
/// The card could not give the pixels back.
pub fn mean_of(drawn: &Drawn, region: Region) -> Result<[f32; 3], Fault> {
    let pixels = sample::read_region(&drawn.gpu, drawn.target.texture(), region)
        .map_err(|error| drawing(&error.to_string()))?;
    Ok(sample::mean(&pixels).unwrap_or([0.0; 3]))
}

/// `project` made to draw the picture at a node: the grade of the node cut there, and the grades
/// after it none; and the clip whose layer alone is drawn, where it is a grade of a clip or a
/// group.
fn cut_at(
    project: &Project,
    index: usize,
    frame: i64,
    node: &NodeAt,
) -> Result<(Project, Option<String>), Fault> {
    let mut project = project.clone();
    let timeline_id = project.timelines[index].id.clone();
    let shown: Vec<String> = resolve::video_at(&project.timelines[index], &project, frame)
        .into_iter()
        .map(|layer| layer.clip)
        .collect();
    let group_of = |project: &Project, clip: &str| {
        project.timelines[index]
            .tracks
            .iter()
            .flat_map(|track| &track.clips)
            .find(|found| found.id == clip)
            .and_then(|found| found.group.clone())
    };
    let other_timeline = |named: &str| {
        missing(
            "timeline",
            format!("the grade is of timeline {named}, and the frame is of {timeline_id}"),
        )
    };
    let not_shown = |clip: &str| {
        missing(
            "clip",
            format!("clip {clip} is not shown at frame {frame} of timeline {timeline_id}"),
        )
    };
    let clip_of_group = |project: &Project, group: &str| -> Result<String, Fault> {
        if let Some(clip) = &node.clip {
            return if shown.contains(clip) && group_of(project, clip).as_deref() == Some(group) {
                Ok(clip.clone())
            } else {
                Err(not_shown(clip))
            };
        }
        shown
            .iter()
            .rev()
            .find(|clip| group_of(project, clip).as_deref() == Some(group))
            .cloned()
            .ok_or_else(|| {
                missing(
                    "clip",
                    format!("no clip of group {group} is shown at frame {frame} of timeline {timeline_id}"),
                )
            })
    };
    let only = match &node.grade {
        GradeTarget::Clip { timeline, clip } => {
            if *timeline != timeline_id {
                return Err(other_timeline(timeline));
            }
            if !shown.contains(clip) {
                return Err(not_shown(clip));
            }
            let group = group_of(&project, clip);
            let found = clip_mut(&mut project, index, clip).expect("a clip that is shown is there");
            found.grade = cut(&found.grade, &node.node, node.place)?;
            if let Some(group) = group.and_then(|id| group_mut(&mut project, &id)) {
                group.post = Grade::default();
            }
            Some(clip.clone())
        }
        GradeTarget::PreClip { group } | GradeTarget::PostClip { group } => {
            let pre = matches!(node.grade, GradeTarget::PreClip { .. });
            let found = group_mut(&mut project, group)
                .ok_or_else(|| missing("grade", format!("there is no group {group}")))?;
            if pre {
                found.pre = cut(&found.pre, &node.node, node.place)?;
                found.post = Grade::default();
            } else {
                found.post = cut(&found.post, &node.node, node.place)?;
            }
            let clip = clip_of_group(&project, group)?;
            if pre {
                clip_mut(&mut project, index, &clip)
                    .expect("a clip that is shown is there")
                    .grade = Grade::default();
            }
            Some(clip)
        }
        GradeTarget::Timeline { timeline } => {
            if *timeline != timeline_id {
                return Err(other_timeline(timeline));
            }
            let grade = &mut project.timelines[index].grade;
            *grade = cut(grade, &node.node, node.place)?;
            None
        }
    };
    if only.is_some() {
        project.timelines[index].grade = Grade::default();
    }
    Ok((project, only))
}

fn clip_mut<'a>(
    project: &'a mut Project,
    index: usize,
    clip: &str,
) -> Option<&'a mut columbina_model::Clip> {
    project.timelines[index]
        .tracks
        .iter_mut()
        .flat_map(|track| &mut track.clips)
        .find(|found| found.id == clip)
}

fn group_mut<'a>(project: &'a mut Project, group: &str) -> Option<&'a mut columbina_model::Group> {
    project.groups.iter_mut().find(|found| found.id == group)
}

/// `grade` cut at the input or output of `node`: that node, or what is linked into its first
/// input, and all that leads to it, linked to the output.
///
/// # Errors
/// There is no such node, it gives no picture (a key mixer), its first input has no link, or the
/// grade cut there is not complete.
pub fn cut(grade: &Grade, node: &str, place: Place) -> Result<Grade, Fault> {
    let refused = |message: String| missing("node", message);
    let found = grade
        .node(node)
        .ok_or_else(|| refused(format!("there is no node {node} in the grade")))?;
    let end = match place {
        Place::Output => {
            if !found.kind.has_output(Port::Rgb) {
                return Err(refused(format!("node {node} gives no picture, only a key")));
            }
            node.to_owned()
        }
        // The key is shown of the grade as it is, the node in it.
        Place::Key => return Ok(grade.clone()),
        Place::Input => {
            if found.kind.inputs(Port::Rgb) == 0 {
                return Err(refused(format!("node {node} takes no picture, only keys")));
            }
            let link = grade
                .link_into(node, Port::Rgb, 0)
                .ok_or_else(|| refused(format!("nothing is linked into node {node}")))?;
            if link.from == SOURCE {
                return Ok(Grade::default());
            }
            link.from.clone()
        }
    };
    let mut kept = BTreeSet::from([end.clone()]);
    let mut looked = vec![end.clone()];
    while let Some(at) = looked.pop() {
        for link in grade.links.iter().filter(|link| link.to == at) {
            if link.from != SOURCE && kept.insert(link.from.clone()) {
                looked.push(link.from.clone());
            }
        }
    }
    let mut cut = Grade {
        nodes: grade
            .nodes
            .iter()
            .filter(|found| kept.contains(&found.id))
            .cloned()
            .collect(),
        links: grade
            .links
            .iter()
            .filter(|link| {
                kept.contains(&link.to) && (link.from == SOURCE || kept.contains(&link.from))
            })
            .cloned()
            .collect(),
    };
    cut.links.push(Link::rgb(end, OUTPUT));
    cut.sort_links();
    if !cut.is_complete() {
        return Err(refused(format!(
            "the picture at node {node} cannot be drawn apart: a node before it leads to it only \
             by its key"
        )));
    }
    Ok(cut)
}

#[cfg(test)]
mod tests {
    use super::*;
    use columbina_model::grade::Node;

    /// source → n1 → n2 → n3 → output, and n1's key into n3.
    fn chain() -> Grade {
        let mut grade = Grade {
            nodes: vec![
                Node::corrector("n1"),
                Node::corrector("n2"),
                Node::corrector("n3"),
            ],
            links: vec![
                Link::rgb(SOURCE, "n1"),
                Link::rgb("n1", "n2"),
                Link::rgb("n2", "n3"),
                Link::rgb("n3", OUTPUT),
                Link {
                    from: "n1".into(),
                    to: "n3".into(),
                    kind: Port::Key,
                    input: 0,
                },
            ],
        };
        grade.sort_links();
        assert!(grade.is_complete());
        grade
    }

    fn ids(grade: &Grade) -> Vec<&str> {
        grade.nodes.iter().map(|node| node.id.as_str()).collect()
    }

    #[test]
    fn a_grade_is_cut_at_the_input_or_the_output_of_a_node() {
        let grade = chain();
        let at_n2 = cut(&grade, "n2", Place::Output).unwrap();
        assert_eq!(ids(&at_n2), ["n1", "n2"]);
        assert_eq!(at_n2.link_into(OUTPUT, Port::Rgb, 0).unwrap().from, "n2");
        assert!(at_n2.is_complete());
        // The input of n2 is what n1 gives.
        assert_eq!(cut(&grade, "n2", Place::Input).unwrap(), {
            let mut only = cut(&grade, "n1", Place::Output).unwrap();
            only.sort_links();
            only
        });
        // The input of n1 is the source: no grade at all.
        assert_eq!(cut(&grade, "n1", Place::Input).unwrap(), Grade::default());
        // n3 keeps n1's key.
        let at_n3 = cut(&grade, "n3", Place::Output).unwrap();
        assert_eq!(at_n3.links.len(), grade.links.len());
        assert_eq!(
            cut(&grade, "n9", Place::Output).unwrap_err().code,
            rpc::INVALID_PARAMS
        );
    }

    #[test]
    fn a_node_that_leads_there_only_by_its_key_cannot_be_cut_for() {
        // source → n1 → n2 → mixer → output, source → n4 → mixer, and n4's key into n2: cut at
        // n2, n4 is kept for its key, and its picture has nowhere to go.
        let mut grade = Grade {
            nodes: vec![
                Node::corrector("n1"),
                Node::corrector("n2"),
                Node::corrector("n4"),
                Node::new("m", columbina_model::grade::Kind::ParallelMixer),
            ],
            links: vec![
                Link::rgb(SOURCE, "n1"),
                Link::rgb("n1", "n2"),
                Link::rgb(SOURCE, "n4"),
                Link::rgb("n2", "m"),
                Link {
                    input: 1,
                    ..Link::rgb("n4", "m")
                },
                Link {
                    from: "n4".into(),
                    to: "n2".into(),
                    kind: Port::Key,
                    input: 0,
                },
                Link::rgb("m", OUTPUT),
            ],
        };
        grade.sort_links();
        assert!(grade.is_complete());
        let refused = cut(&grade, "n2", Place::Output).unwrap_err();
        assert_eq!(refused.code, rpc::INVALID_PARAMS);
        // At n1 there is no such node, and the mixer's picture is all of it.
        assert_eq!(ids(&cut(&grade, "n2", Place::Input).unwrap()), ["n1"]);
        assert_eq!(cut(&grade, "m", Place::Output).unwrap().nodes.len(), 4);
    }

    #[test]
    fn a_sample_is_a_square_about_its_pixel_within_the_frame() {
        assert_eq!(
            square(10.5, 20.9, 1, 64, 36).unwrap(),
            Region {
                x: 10,
                y: 20,
                width: 1,
                height: 1
            }
        );
        assert_eq!(
            square(0.2, 35.9, 5, 64, 36).unwrap(),
            Region {
                x: 0,
                y: 33,
                width: 3,
                height: 3
            }
        );
        for (x, y, size) in [
            (64.0, 1.0, 1),
            (-0.5, 1.0, 1),
            (1.0, f64::NAN, 1),
            (1.0, 1.0, 2),
            (1.0, 1.0, 33),
        ] {
            assert_eq!(
                square(x, y, size, 64, 36).unwrap_err().code,
                rpc::INVALID_PARAMS
            );
        }
    }
}
