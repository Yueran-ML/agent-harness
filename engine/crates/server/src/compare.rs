//! What the timeline monitor compares its frame with: a wipe against a reference, or a split
//! screen of several pictures (`viewer.wipe`, `viewer.split`; RM20 p2999–3006, p3260–3261). The
//! pictures are put together on the card by [`columbina_compose::wipe`]; this module says what
//! the references are and draws them. Only the monitor does: an export never compares.
//!
//! # References
//!
//! - **A still** (`{"kind": "still", "path"}`, or `{"kind": "still", "still": "s3"}` for a still
//!   of the Gallery, [`crate::stills`]): a picture file (PNG, TIFF, `OpenEXR`, as media reads
//!   stills) as the Gallery holds it (p2999). It is read when the wipe is set, and shown as it is
//!   stored, fitted to the canvas: a still is a grabbed picture, its grade in it, and is neither
//!   graded nor transformed again.
//! - **A frame of the timeline** (`{"kind": "frame", "frame"}`): the timeline at that frame,
//!   graded as it is there. It stays where it is as the playhead moves, as a timeline wipe does
//!   with "Gang timeline wipe with current clip" off (p3002, p3261).
//! - **A clip at a frame** (`{"kind": "clip", "clip", "frame"}`): the timeline at that many
//!   frames from the first of the clip (held within it), as Wipe Timeline Clip shows another
//!   clip (p3002).
//! - **The clip before or after** (`{"kind": "neighbour", "side": "previous" | "next"}`): the
//!   clip before or after the current one, at the same offset from its first frame as the
//!   playhead is from the current clip's, held within it: a timeline wipe ganged with the
//!   current clip (p3002: "the offset between the timeline wiped clip and the current clip is
//!   maintained"). The current clip is the one that the client names (`"current"`, as the
//!   desktop has one clicked), else the one on top at the playhead; the clips are in the order
//!   of the thumbnail timeline, by their start, then by their track from V1 up.
//! - **The frame shown** (`{"kind": "current"}`), in a split screen: the picture of the playhead.
//!
//! A reference of the timeline is drawn as the frame is: its pictures read (from the media, not
//! its proxies), its clips and the timeline graded by their grades at that frame, its subtitles
//! left out. A reference that cannot be found where the playhead now is (the clip before the
//! first) is black. What was drawn is kept until the project, the card or the frame that it is
//! changes, so a still or a frame is drawn once, and a wipe while playing costs the wipe alone.
//!
//! # Scopes
//!
//! The manual does not say whether the scopes measure the comparison or the frame alone. They
//! measure what the viewer shows (as they measure the video output, which mirrors the wipe,
//! p2999 and p3003), the comparison with it; [`SCOPES_SEE_COMPARISON`] says so in one place, to
//! be checked against `DaVinci Resolve` itself.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use columbina_compose::wipe::{Split, Style, Wipe, Wiper, MOST_TILES};
use columbina_compose::{Compositor, Gpu, Scene, Target};
use columbina_media::{Converter, Hardware, NativePicture, VideoDecoder};
use columbina_model::project::TrackKind;
use columbina_model::{resolve, Project};
use serde::{Deserialize, Serialize};
use serde_json::{json, value::RawValue, Value};

use crate::grading::Grading;
use crate::play::sources::Sources;
use crate::rpc::{self, Fault};

/// Whether the scopes of the timeline monitor (and its statistics) measure the comparison that
/// the viewer shows, rather than the frame alone.
pub const SCOPES_SEE_COMPARISON: bool = true;

/// A style of wipe, as the protocol names it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StyleName {
    #[default]
    Horizontal,
    Vertical,
    Diagonal,
    Mix,
    Difference,
    Box,
    VenetianBlind,
    Checkerboard,
}

impl From<StyleName> for Style {
    fn from(name: StyleName) -> Style {
        match name {
            StyleName::Horizontal => Style::Horizontal,
            StyleName::Vertical => Style::Vertical,
            StyleName::Diagonal => Style::Diagonal,
            StyleName::Mix => Style::Mix,
            StyleName::Difference => Style::Difference,
            StyleName::Box => Style::Box,
            StyleName::VenetianBlind => Style::VenetianBlind,
            StyleName::Checkerboard => Style::Checkerboard,
        }
    }
}

/// Which clip next to the current one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Previous,
    Next,
}

/// What a picture is compared with. See the module.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Reference {
    Still {
        /// The file of the still. The engine puts in the file of a still of the Gallery named
        /// by `still` before the reference comes here.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        /// A still of the Gallery, by its id.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        still: Option<String>,
    },
    Frame {
        frame: i64,
    },
    Clip {
        clip: String,
        #[serde(default)]
        frame: i64,
    },
    Neighbour {
        side: Side,
        /// The current clip, where the client chose it (the one clicked in its thumbnails);
        /// the one on top at the playhead where left out.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        current: Option<String>,
    },
    Current,
}

fn yes() -> bool {
    true
}
fn half() -> f32 {
    0.5
}
fn diagonal() -> f32 {
    45.0
}
fn white() -> [f32; 3] {
    [1.0, 1.0, 1.0]
}
fn one() -> f32 {
    1.0
}

/// The params of `viewer.wipe`: the whole wipe, each value its default where left out.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WipeSettings {
    #[serde(default = "yes")]
    on: bool,
    #[serde(default)]
    style: StyleName,
    #[serde(default = "half")]
    position: f32,
    #[serde(default = "diagonal")]
    angle: f32,
    #[serde(default)]
    softness: f32,
    #[serde(default)]
    border: f32,
    #[serde(default = "white")]
    border_color: [f32; 3],
    #[serde(default)]
    invert: bool,
    #[serde(default = "one")]
    gain: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reference: Option<Reference>,
}

impl WipeSettings {
    fn wipe(&self) -> Wipe {
        Wipe {
            style: self.style.into(),
            position: self.position,
            angle: self.angle,
            softness: self.softness,
            border: self.border,
            border_colour: self.border_color,
            invert: self.invert,
            gain: self.gain,
        }
    }
}

/// The params of `viewer.split`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SplitSettings {
    #[serde(default = "yes")]
    on: bool,
    #[serde(default)]
    items: Vec<Reference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    selected: Option<usize>,
    #[serde(default = "yes")]
    outline: bool,
}

/// A change of what the monitor compares with, as a method asks it.
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    Wipe(WipeSettings),
    Split(SplitSettings),
}

impl Change {
    /// Read the params of `viewer.wipe` or `viewer.split`, and check what can be checked without
    /// the timeline.
    ///
    /// # Errors
    /// The params are not what the method takes, or a value is out of its range.
    pub fn read(method: &str, params: Option<&RawValue>) -> Result<Change, Fault> {
        let range = |message: String| Fault::params(message).with_data(json!({"reason": "range"}));
        if method == "viewer.wipe" {
            let settings: WipeSettings = rpc::params(
                method,
                r#"{"on": true, "style": "horizontal", "position": 0.5, "reference": {"kind": "still", "path": "…"}}"#,
                params,
            )?;
            if settings.on {
                settings.wipe().check().map_err(range)?;
                match &settings.reference {
                    None => {
                        return Err(Fault::params("a wipe that is on needs a reference")
                            .with_data(json!({"reason": "reference"})))
                    }
                    Some(Reference::Current) => {
                        return Err(Fault::params(
                            "a wipe compares the frame shown with another picture, not itself",
                        )
                        .with_data(json!({"reason": "reference"})))
                    }
                    Some(_) => {}
                }
            }
            Ok(Change::Wipe(settings))
        } else {
            let settings: SplitSettings = rpc::params(
                method,
                r#"{"on": true, "items": [{"kind": "current"}, {"kind": "clip", "clip": "c1"}], "selected": 0}"#,
                params,
            )?;
            if settings.on {
                if settings.items.is_empty() || settings.items.len() > MOST_TILES {
                    return Err(range(format!(
                        "a split screen is of 1 to {MOST_TILES} items, not {}",
                        settings.items.len()
                    )));
                }
                if settings
                    .selected
                    .is_some_and(|selected| selected >= settings.items.len())
                {
                    return Err(range("selected is the index of one of the items".into()));
                }
            }
            Ok(Change::Split(settings))
        }
    }
}

/// What is compared now.
#[derive(Clone, Debug, Default, PartialEq)]
enum Mode {
    #[default]
    Off,
    Wipe(WipeSettings),
    Split(SplitSettings),
}

/// A reference found in the timeline for a frame: what is drawn.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Found {
    Still(PathBuf),
    /// A frame of the timeline.
    Frame(i64),
    /// The frame shown itself.
    Current,
    /// Nothing there: black.
    Nothing,
}

/// What a picture drawn of a reference was drawn of: what, of which project (as the address of
/// its `Arc`, for a frame of the timeline), at which size.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Key {
    found: Found,
    project: usize,
    size: (u32, u32),
}

/// What the timeline monitor compares with, and the pictures drawn for it.
pub struct Comparison {
    mode: Mode,
    /// The stills read, by path, each when it was set.
    stills: HashMap<PathBuf, Arc<NativePicture>>,
    /// What reads the pictures of references of the timeline: decoders of its own, so that
    /// the readers ahead of the playhead keep theirs.
    sources: Sources,
    wiper: Option<Wiper>,
    /// The pictures drawn of references, kept while they are what is compared with.
    drawn: HashMap<Key, Arc<Target>>,
    /// What the frame being drawn compares with, in order, and the scenes of those not drawn
    /// yet (`prepare`), drawn by `show`.
    wanted: Vec<Key>,
    scenes: Vec<(Key, Scene)>,
    /// The pictures shown, drawn into in turn: one is free when no one else holds it.
    shown: Vec<Arc<Target>>,
    /// The frame last compared, and the picture of the comparison shown for it: what a still
    /// is grabbed of while the frame is compared ([`Comparison::original`]).
    compared: Option<(Arc<Target>, Arc<Target>)>,
}

impl Default for Comparison {
    fn default() -> Self {
        Self::new()
    }
}

impl Comparison {
    pub fn new() -> Comparison {
        Comparison {
            mode: Mode::Off,
            stills: HashMap::new(),
            sources: Sources::new(Hardware::Auto, Arc::new(|_, _| {})),
            wiper: None,
            drawn: HashMap::new(),
            wanted: Vec::new(),
            scenes: Vec::new(),
            shown: Vec::new(),
            compared: None,
        }
    }

    /// Let go of what was drawn and read (the timeline was unloaded); what is compared stays.
    pub fn forget(&mut self) {
        self.drawn.clear();
        self.wanted.clear();
        self.scenes.clear();
        self.shown.clear();
        self.compared = None;
    }

    /// Take a change, the timeline `index` of `project` being loaded with its playhead at
    /// `frame`: check its references there, read its stills, and give what is compared now.
    ///
    /// # Errors
    /// A reference cannot be found in the timeline, or a still cannot be read.
    pub fn change(
        &mut self,
        change: Change,
        loaded: Option<(&Project, usize, i64)>,
    ) -> Result<Value, Fault> {
        let references: Vec<Reference> = match &change {
            Change::Wipe(settings) if settings.on => settings.reference.iter().cloned().collect(),
            Change::Split(settings) if settings.on => settings.items.clone(),
            _ => Vec::new(),
        };
        let mut stills = HashMap::new();
        for reference in &references {
            if let Reference::Still { path, .. } = reference {
                let path = PathBuf::from(path.as_deref().ok_or_else(|| {
                    Fault::params("a still is named by its path or by its id")
                        .with_data(json!({"reason": "still"}))
                })?);
                let picture = read_still(&path).map_err(|message| {
                    Fault::params(format!(
                        "the still {} cannot be read: {message}",
                        path.display()
                    ))
                    .with_data(json!({"reason": "still", "path": path, "message": message}))
                })?;
                stills.insert(path, Arc::new(picture));
            } else if !matches!(reference, Reference::Current) {
                let (project, index, frame) = loaded.ok_or_else(|| {
                    Fault::new(
                        rpc::TRANSPORT_FAILED,
                        "no timeline is loaded in the transport",
                    )
                    .with_data(json!({"reason": "not_loaded"}))
                })?;
                find(reference, project, index, frame).map_err(|message| {
                    Fault::params(message)
                        .with_data(json!({"reason": "reference", "reference": reference}))
                })?;
            }
        }
        self.mode = match change {
            Change::Wipe(settings) if settings.on => Mode::Wipe(settings),
            Change::Split(settings) if settings.on => Mode::Split(settings),
            // Turning one off leaves the other as it is.
            Change::Wipe(_) => match &self.mode {
                Mode::Wipe(_) => Mode::Off,
                other => other.clone(),
            },
            Change::Split(_) => match &self.mode {
                Mode::Split(_) => Mode::Off,
                other => other.clone(),
            },
        };
        self.stills = stills;
        if self.mode == Mode::Off {
            self.forget();
        }
        Ok(self.state(loaded))
    }

    /// What is compared, as the methods answer: the mode, and the wipe or split with each
    /// reference of the timeline found where the playhead is (`at`: the frame of the timeline
    /// drawn, null where there is nothing).
    pub fn state(&self, loaded: Option<(&Project, usize, i64)>) -> Value {
        let at = |reference: &Reference| {
            let mut value = json!(reference);
            if let Some((project, index, frame)) = loaded {
                if !matches!(reference, Reference::Still { .. } | Reference::Current) {
                    value["at"] = match find(reference, project, index, frame) {
                        Ok(Found::Frame(frame)) => json!(frame),
                        _ => Value::Null,
                    };
                }
            }
            value
        };
        match &self.mode {
            Mode::Off => json!({"mode": "off"}),
            Mode::Wipe(settings) => {
                let mut wipe = json!(settings);
                if let Some(reference) = &settings.reference {
                    wipe["reference"] = at(reference);
                }
                json!({"mode": "wipe", "wipe": wipe})
            }
            Mode::Split(settings) => {
                let mut split = json!(settings);
                split["items"] = settings.items.iter().map(at).collect();
                json!({"mode": "split", "split": split})
            }
        }
    }

    /// Before the frame `frame` of the timeline `index` of `project` is drawn: find what it is
    /// compared with, and make the scenes of what is not drawn yet, graded by `grading` (made
    /// ready for this project and card). Nothing where nothing is compared.
    pub fn prepare(&mut self, project: &Arc<Project>, index: usize, frame: i64, grading: &Grading) {
        self.wanted.clear();
        self.scenes.clear();
        let references: Vec<&Reference> = match &self.mode {
            Mode::Off => return,
            Mode::Wipe(settings) => settings.reference.iter().collect(),
            Mode::Split(settings) => settings.items.iter().collect(),
        };
        let timeline = &project.timelines[index];
        let size = (timeline.width, timeline.height);
        for reference in references {
            let found = find(reference, project, index, frame).unwrap_or(Found::Nothing);
            let key = Key {
                project: match found {
                    Found::Frame(_) => Arc::as_ptr(project) as usize,
                    _ => 0,
                },
                found,
                size,
            };
            if !self.drawn.contains_key(&key)
                && !self.scenes.iter().any(|(made, _)| *made == key)
                && key.found != Found::Current
            {
                let scene = match &key.found {
                    Found::Still(path) => self.stills.get(path).map(|picture| Scene {
                        width: size.0,
                        height: size.1,
                        layers: vec![crate::render::picture_layer(Arc::clone(picture), 0)],
                        ..Scene::default()
                    }),
                    Found::Frame(at) => {
                        match crate::render::scene(project, timeline, *at, &self.sources) {
                            Ok((mut scene, clips)) => {
                                grading.grade(&mut scene, &clips, *at);
                                Some(scene)
                            }
                            Err(message) => {
                                crate::log::log(
                                    "compare_error",
                                    &json!({"frame": at, "message": message}),
                                );
                                None
                            }
                        }
                    }
                    Found::Current | Found::Nothing => None,
                };
                // What cannot be drawn is black: a scene with nothing in it.
                let scene = scene.unwrap_or_else(|| Scene {
                    width: size.0,
                    height: size.1,
                    ..Scene::default()
                });
                self.scenes.push((key.clone(), scene));
            }
            self.wanted.push(key);
        }
    }

    /// After the frame was drawn into `current`: draw what it is compared with, with
    /// `compositor`, and give the picture that the viewer shows; none where nothing is
    /// compared, when the frame itself is shown, as it is.
    ///
    /// # Errors
    /// The card failed to draw.
    pub fn show(
        &mut self,
        gpu: &Arc<Gpu>,
        compositor: &mut Compositor,
        current: &Arc<Target>,
    ) -> Result<Option<Arc<Target>>, String> {
        if self.mode == Mode::Off || self.wanted.is_empty() {
            return Ok(None);
        }
        if self
            .wiper
            .as_ref()
            .is_none_or(|wiper| !Arc::ptr_eq(wiper.gpu(), gpu))
        {
            self.wiper = Some(Wiper::new(Arc::clone(gpu)));
            self.drawn.clear();
            self.shown.clear();
        }
        // Draw what is not drawn yet; keep only what this frame compares with.
        let wanted = std::mem::take(&mut self.wanted);
        for (key, scene) in std::mem::take(&mut self.scenes) {
            let target = Arc::new(
                Target::preview(gpu, scene.width, scene.height)
                    .map_err(|error| error.to_string())?,
            );
            // A reference is kept and drawn once: drawn with its grades as they are.
            compositor
                .render_waiting(&scene, &target)
                .map_err(|error| error.to_string())?;
            self.drawn.insert(key, target);
        }
        self.drawn.retain(|key, _| wanted.contains(key));
        let size = current.size();
        self.shown.retain(|target| target.size() == size);
        let out = if let Some(free) = self.shown.iter().find(|target| {
            Arc::strong_count(target) == 1 && Arc::strong_count(target.texture()) == 1
        }) {
            Arc::clone(free)
        } else {
            let made =
                Arc::new(Target::preview(gpu, size.0, size.1).map_err(|error| error.to_string())?);
            self.shown.push(Arc::clone(&made));
            made
        };
        let textures: Vec<_> = wanted
            .iter()
            .map(|key| {
                if key.found == Found::Current {
                    current.texture()
                } else {
                    self.drawn[key].texture()
                }
            })
            .collect();
        let wiper = self.wiper.as_ref().expect("made above");
        let drawn = match &self.mode {
            Mode::Wipe(settings) => wiper.wipe(
                current.texture(),
                textures[0],
                &settings.wipe(),
                out.texture(),
            ),
            Mode::Split(settings) => {
                let pictures: Vec<_> = textures.iter().map(|texture| &***texture).collect();
                wiper.split(
                    &pictures,
                    Split {
                        selected: settings.selected,
                        outline: settings.outline,
                    },
                    out.texture(),
                )
            }
            Mode::Off => unreachable!("returned above"),
        };
        drawn.map_err(|error| error.to_string())?;
        self.compared = Some((Arc::clone(current), Arc::clone(&out)));
        Ok(Some(out))
    }

    /// The frame itself of `shown`, what the viewer shows: where it is the comparison last
    /// drawn, the frame that was compared (image A, never the wipe or the split screen); else
    /// `shown` itself.
    pub fn original(&self, shown: &Arc<Target>) -> Arc<Target> {
        match &self.compared {
            Some((frame, out)) if Arc::ptr_eq(out, shown) => Arc::clone(frame),
            _ => Arc::clone(shown),
        }
    }

    /// Freeze the comparison that was actually applied to this target, including its resolved
    /// reference positions. Kept server-side; pointer summaries expose only the mode.
    pub fn snapshot(&self, shown: &Arc<Target>, loaded: Option<(&Project, usize, i64)>) -> Value {
        if self
            .compared
            .as_ref()
            .is_some_and(|(_, compared)| Arc::ptr_eq(compared, shown))
        {
            self.state(loaded)
        } else {
            json!({"mode":"off"})
        }
    }
}

/// Read a still: the first picture of the file, as it is stored.
fn read_still(path: &Path) -> Result<NativePicture, String> {
    let mut reader = VideoDecoder::open(path, Hardware::Off).map_err(|error| error.to_string())?;
    let first = reader.decoded_at(0).map_err(|error| error.to_string())?;
    Converter::new()
        .native(&first)
        .map_err(|error| error.to_string())
}

/// The clips of picture of a timeline that are seen, in the order of the thumbnail timeline:
/// by their start, then by their track from V1 up; each with the number of its track.
pub(crate) fn clips_in_order(
    timeline: &columbina_model::Timeline,
) -> Vec<(usize, &columbina_model::Clip)> {
    let mut clips: Vec<_> = timeline
        .tracks
        .iter()
        .enumerate()
        .filter(|(_, track)| track.kind == TrackKind::Video && track.enabled)
        .flat_map(|(number, track)| {
            track
                .clips
                .iter()
                .filter(|clip| clip.enabled)
                .map(move |clip| (number, clip))
        })
        .collect();
    clips.sort_by_key(|(number, clip)| (clip.start, *number));
    clips
}

/// A frame of a clip `offset` frames from its first, held within it.
fn within(clip: &columbina_model::Clip, offset: i64) -> i64 {
    clip.start + offset.clamp(0, (clip.length - 1).max(0))
}

/// Where `reference` is in the timeline `index` of `project`, the playhead at `frame`.
fn find(
    reference: &Reference,
    project: &Project,
    index: usize,
    frame: i64,
) -> Result<Found, String> {
    let timeline = &project.timelines[index];
    match reference {
        Reference::Still { path, .. } => Ok(Found::Still(PathBuf::from(
            path.as_deref().unwrap_or_default(),
        ))),
        Reference::Current => Ok(Found::Current),
        Reference::Frame { frame } => {
            let last = (resolve::duration(timeline) - 1).max(0);
            Ok(Found::Frame((*frame).clamp(0, last)))
        }
        Reference::Clip { clip, frame } => clips_in_order(timeline)
            .into_iter()
            .find(|(_, found)| found.id == *clip)
            .map(|(_, found)| Found::Frame(within(found, *frame)))
            .ok_or_else(|| format!("there is no clip {clip} on a track of picture that is on")),
        Reference::Neighbour { side, current } => {
            let clips = clips_in_order(timeline);
            // The current clip: the one named, or the one on top at the playhead.
            let current = if let Some(named) = current {
                clips
                    .iter()
                    .position(|(_, clip)| clip.id == *named)
                    .ok_or_else(|| {
                        format!("there is no clip {named} on a track of picture that is on")
                    })?
            } else {
                clips
                    .iter()
                    .enumerate()
                    .filter(|(_, (_, clip))| {
                        clip.start <= frame && frame < clip.start + clip.length
                    })
                    .max_by_key(|(_, (number, _))| *number)
                    .map(|(place, _)| place)
                    .ok_or_else(|| format!("no clip is at frame {frame}, so none is next to it"))?
            };
            let other = match side {
                Side::Previous => current.checked_sub(1),
                Side::Next => Some(current + 1).filter(|next| *next < clips.len()),
            }
            .ok_or_else(|| {
                format!(
                    "the clip at frame {frame} is the {} of the timeline",
                    if *side == Side::Previous {
                        "first"
                    } else {
                        "last"
                    }
                )
            })?;
            let offset = frame - clips[current].1.start;
            Ok(Found::Frame(within(clips[other].1, offset)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timeline_of(clips: &[(&str, usize, i64, i64)]) -> Project {
        let mut project = Project::sample();
        let timeline = &mut project.timelines[0];
        let template = timeline.tracks[0].clips[0].clone();
        let mut video = timeline.tracks[0].clone();
        video.clips.clear();
        timeline
            .tracks
            .retain(|track| track.kind != TrackKind::Video);
        let mut tracks = vec![video.clone(), video];
        tracks[1].id = "v2".into();
        for (id, track, start, length) in clips {
            let mut clip = template.clone();
            (clip.id, clip.start, clip.length) = ((*id).into(), *start, *length);
            tracks[*track].clips.push(clip);
        }
        timeline.tracks.splice(0..0, tracks);
        project
    }

    #[test]
    fn the_clip_before_or_after_is_found_at_the_same_offset_held_within_it() {
        // c1 0–23 and c2 24–47 on V1, c3 30–39 on V2 over c2, c4 48–51 on V1.
        let project = timeline_of(&[
            ("c1", 0, 0, 24),
            ("c2", 0, 24, 24),
            ("c3", 1, 30, 10),
            ("c4", 0, 48, 4),
        ]);
        let at = |side, frame| {
            find(
                &Reference::Neighbour {
                    side,
                    current: None,
                },
                &project,
                0,
                frame,
            )
        };
        // At 26 the current clip is c2, 2 frames in: c1 at 2; the next is c3, at 32.
        assert_eq!(at(Side::Previous, 26), Ok(Found::Frame(2)));
        assert_eq!(at(Side::Next, 26), Ok(Found::Frame(32)));
        // At 33 c3 is on top, 3 frames in: before it is c2 (at 27), after it c4, held in its
        // four frames (at 51).
        assert_eq!(at(Side::Previous, 33), Ok(Found::Frame(27)));
        assert_eq!(at(Side::Next, 33), Ok(Found::Frame(51)));
        // Nothing before the first, after the last, or where no clip is.
        assert!(at(Side::Previous, 5).is_err());
        assert!(at(Side::Next, 50).is_err());
        assert!(at(Side::Next, 400).is_err());
        // The current clip named by the client: c2 under c3 at 33, 9 frames into c2, so the
        // clip after it is c3 held at its last frame, 39.
        let named = |side, current: &str| {
            find(
                &Reference::Neighbour {
                    side,
                    current: Some(current.into()),
                },
                &project,
                0,
                33,
            )
        };
        assert_eq!(named(Side::Next, "c2"), Ok(Found::Frame(39)));
        assert_eq!(named(Side::Previous, "c2"), Ok(Found::Frame(9)));
        assert!(named(Side::Previous, "nope").is_err());
        // A clip at a frame, held within it; a frame of the timeline, held within it.
        let clip = |clip: &str, frame| {
            find(
                &Reference::Clip {
                    clip: clip.into(),
                    frame,
                },
                &project,
                0,
                0,
            )
        };
        assert_eq!(clip("c3", 4), Ok(Found::Frame(34)));
        assert_eq!(clip("c3", 40), Ok(Found::Frame(39)));
        assert_eq!(clip("c3", -3), Ok(Found::Frame(30)));
        assert!(clip("nope", 0).is_err());
        let last = resolve::duration(&project.timelines[0]) - 1;
        assert_eq!(
            find(&Reference::Frame { frame: 9999 }, &project, 0, 0),
            Ok(Found::Frame(last))
        );
    }

    #[test]
    fn the_params_are_read_with_their_defaults_and_checked() {
        let read = |method: &str, params: &str| {
            let raw = RawValue::from_string(params.into()).unwrap();
            Change::read(method, Some(&raw))
        };
        let Ok(Change::Wipe(wipe)) = read(
            "viewer.wipe",
            r#"{"reference": {"kind": "frame", "frame": 3}}"#,
        ) else {
            panic!("a wipe");
        };
        assert!(wipe.on);
        assert_eq!(wipe.wipe(), Wipe::default());
        assert_eq!(
            json!(wipe),
            json!({"on": true, "style": "horizontal", "position": 0.5, "angle": 45.0,
                "softness": 0.0, "border": 0.0, "border_color": [1.0, 1.0, 1.0], "invert": false,
                "gain": 1.0, "reference": {"kind": "frame", "frame": 3}})
        );
        assert!(matches!(
            read("viewer.wipe", r#"{"on": false}"#),
            Ok(Change::Wipe(WipeSettings { on: false, .. }))
        ));
        let reason =
            |result: Result<Change, Fault>| result.unwrap_err().data.unwrap()["reason"].clone();
        assert_eq!(
            reason(read("viewer.wipe", r#"{"style": "mix"}"#)),
            "reference"
        );
        assert_eq!(
            reason(read(
                "viewer.wipe",
                r#"{"position": 2, "reference": {"kind": "frame", "frame": 3}}"#
            )),
            "range"
        );
        assert_eq!(
            reason(read("viewer.wipe", r#"{"reference": {"kind": "current"}}"#)),
            "reference"
        );
        assert!(read(
            "viewer.wipe",
            r#"{"style": "alpha", "reference": {"kind": "frame", "frame": 3}}"#
        )
        .is_err());
        assert!(read(
            "viewer.wipe",
            r#"{"reference": {"kind": "frame", "frame": 3, "x": 1}}"#
        )
        .is_err());
        let Ok(Change::Split(split)) = read(
            "viewer.split",
            r#"{"items": [{"kind": "current"}, {"kind": "neighbour", "side": "next"}], "selected": 1}"#,
        ) else {
            panic!("a split");
        };
        assert!(split.outline);
        assert_eq!(reason(read("viewer.split", r#"{"items": []}"#)), "range");
        assert_eq!(
            reason(read(
                "viewer.split",
                r#"{"items": [{"kind": "current"}], "selected": 1}"#
            )),
            "range"
        );
        let many = format!(
            r#"{{"items": [{}]}}"#,
            vec![r#"{"kind": "current"}"#; 17].join(",")
        );
        assert_eq!(reason(read("viewer.split", &many)), "range");
    }
}
