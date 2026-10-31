//! The grades of a timeline on the card, for its monitor and its export alike: each clip's grade
//! between the pre-clip and post-clip grades of its group, on its layer, and the timeline's grade
//! over the canvas (p. 3330 of the Reference Manual of `DaVinci Resolve` 20).
//!
//! # When a grade is made again
//!
//! A grade is made ready once ([`Prepared`]): its graph made into code, its LUTs read and put on
//! the card. A frame then takes only the parameters of each grade at that frame, keyframes and
//! all, and the compositor writes them into its buffer; the code is the same from frame to frame,
//! so nothing is compiled while a clip plays or a keyframe moves a setting. A grade is made again
//! only when the project given is another ([`Grading::update`], which is cheap when it is the
//! same project on the same card) and that grade is not equal to the one it was made of, when one
//! of its LUTs could not be read (it is looked for again at each change of the project, so that a
//! LUT put in its place is found), or when the card is another (a card that was lost and made
//! again): LUTs are textures of a card.
//!
//! # The frame of a key
//!
//! The keys of a clip's grade, and of the grades of its group as they apply to that clip, are on
//! the frames of its source, as `DaVinci` ties them (pp. 3383, 3393; the model's `keyframes`): a
//! clip of media is graded at the frame of its media whose picture its layer shows, found by the
//! model's `KeyClock` from the same place in the media that the picture is read at (the model's
//! `span::source_tick` and `span::media_frame`, which the readers of pictures use), so the
//! picture and its grade are of one frame, whatever the clip's trim, slip, speed or direction,
//! and in a transition, where a clip is seen beyond its ends. A title, a colour or a still is
//! graded at the frame of the clip from its first, 0. The keys of the timeline's grade count the
//! frames of the timeline.
//!
//! # What is graded
//!
//! Every clip of a track of picture that has a grade or a group: of media, and titles and
//! colours too, as the model lets any clip carry one. `DaVinci` grades a Text+ title "like any
//! other clip" (p. 1047) and others once made into a compound clip; the compositor grades a
//! title or a colour unpremultiplied, as it does its picture. The colour that a dip of a
//! transition goes through is no clip and is not graded by a clip's grade. The timeline's grade
//! is over everything laid on the canvas, and under the subtitles, which come after it (p.
//! 3330); an export burns no subtitles in.
//!
//! # Colour management
//!
//! The grades are done in the timeline space of the project's colour management, whose
//! primaries give their luma, with the project's S-curve of contrast and interpolation of LUTs;
//! each layer is given its input transform and the scene its output transform
//! ([`crate::managed`]). A change of those settings makes every grade ready again.
//!
//! An export makes the colour management of its project before its first frame, and waits for
//! it. A monitor ([`Grading::monitor`]) makes the first of a timeline that it loads so too; a
//! later change (of the settings, of the input space or levels of media) is made on a thread of
//! its own, which then draws the frame that the monitor last showed, by the new transforms and
//! grades, small, with a compositor of its own: the shaders that the monitor will need are so
//! compiled there, off its thread (Direct3D 12 keeps what it compiled for the device, by the
//! text of the shader). Until that is done the monitor goes on with the colour it had, so that
//! playing never waits on a compiler (an output transform of ACES 2.0 takes it up to a second);
//! then it takes the new colour and is told to draw again.
//!
//! # LUTs
//!
//! A LUT that a corrector names is looked for in the folder of the project, then in the library
//! of LUTs ([`library`]). One that cannot be read leaves its node as though it named none, and is
//! told ([`Grading::missing`]): in the state of the timeline monitor and in the progress of an
//! export (`missing_luts`), as `DaVinci` shows "Missing LUT" (p. 3413).

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use columbina_compose::grade::{CardLuts, GradeOptions, LutCache, Prepared, Roots};
use columbina_compose::ocio::CardOps;
use columbina_compose::{
    ColorOp, Compositor, Gpu, Interpolation, LayerColor, Scene, Stage, Target,
};
use columbina_model::color_management::LutInterpolation;
use columbina_model::grade::{Grade, NodeKind};
use columbina_model::keyframes::KeyClock;
use columbina_model::project::{Source as ClipSource, TrackKind};
use columbina_model::{Project, Timeline};
use serde::Serialize;
use serde_json::json;

use crate::managed::{Colour, Key, Output};

/// How many bytes of tables of LUTs are kept read, for all the monitors and exports of an
/// engine.
const LUT_BUDGET: usize = 256 << 20;

/// The environment variable naming the folders of the library of LUTs, as a list of paths
/// (separated by `;` on Windows, `:` elsewhere).
pub const LUTS_VARIABLE: &str = "COLUMBINA_LUTS";

/// Where the LUTs of a project are read from: the folder of the project and the library, through
/// one cache for the whole engine.
#[derive(Clone)]
pub struct LutPlaces {
    pub cache: Arc<LutCache>,
    pub roots: Roots,
}

impl LutPlaces {
    /// The LUTs of the project in `dir`, and of the library ([`library`]).
    pub fn new(dir: &Path) -> LutPlaces {
        LutPlaces::with_library(dir, library())
    }

    /// The LUTs of the project in `dir`, and of the folders of `library`.
    pub fn with_library(dir: &Path, library: Vec<PathBuf>) -> LutPlaces {
        LutPlaces {
            cache: Arc::new(LutCache::new(LUT_BUDGET)),
            roots: Roots {
                project: Some(dir.to_owned()),
                library,
            },
        }
    }
}

/// The folders of the library of LUTs, the app's: those that [`LUTS_VARIABLE`] names, absolute
/// paths only; where it is not set, `luts` in `COLUMBINA_HOME`, the folder of the app's own
/// files that the desktop gives its programs, or else in `.columbina` in the user's home.
pub fn library() -> Vec<PathBuf> {
    let named = std::env::var_os(LUTS_VARIABLE).filter(|value| !value.is_empty());
    let home = std::env::var_os("COLUMBINA_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
                .filter(|value| !value.is_empty())
                .map(|home| PathBuf::from(home).join(".columbina"))
        });
    library_of(named.as_deref(), home.as_deref())
}

/// The folders of the library, given what [`LUTS_VARIABLE`] says and the folder of the app.
fn library_of(named: Option<&std::ffi::OsStr>, home: Option<&Path>) -> Vec<PathBuf> {
    match named {
        Some(named) => std::env::split_paths(named)
            .filter(|path| path.is_absolute())
            .collect(),
        None => home.map(|home| home.join("luts")).into_iter().collect(),
    }
}

/// A LUT that a grade of the timeline names and that cannot be read: its name, as the node
/// gives it, and why.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct MissingLut {
    pub name: String,
    pub message: String,
}

/// The grades that apply to a clip.
#[derive(Clone)]
struct ClipGrades {
    /// How the frame of its keys is found from a frame of the timeline.
    clock: KeyClock,
    pre: Option<Arc<Prepared>>,
    own: Option<Arc<Prepared>>,
    post: Option<Arc<Prepared>>,
}

/// The pre-clip and post-clip grades of a group, ready.
type GroupGrades = (Option<Arc<Prepared>>, Option<Arc<Prepared>>);

/// What a monitor's colour management is made on a thread of its own by.
struct Background {
    /// Tells the monitor to draw again: its colour is ready.
    wake: Arc<dyn Fn() + Send + Sync>,
    /// What the colour is to be made of, where it is not being made yet.
    wanted: Option<Key>,
    /// What is being made, and what came of it.
    job: Option<(Key, Arc<Mutex<Option<Made>>>)>,
    /// What could not be made, not to be tried again until the project changes it.
    failed: Option<Key>,
}

/// What a thread made of colour management: the colour, or why not.
type Made = Result<Arc<Colour>, String>;

/// The size that the frame is drawn at to compile its shaders: small, as only its shaders count.
const WARM: (u32, u32) = (64, 36);

/// The grades of one timeline of a project, ready on one card.
pub struct Grading {
    places: LutPlaces,
    options: GradeOptions,
    /// The interpolation of the LUTs of the grades.
    interpolation: LutInterpolation,
    /// The card, its LUTs, and its operations of colour management.
    card: Option<(Arc<Gpu>, CardLuts, Arc<CardOps>)>,
    /// The colour management in force.
    colour: Arc<Colour>,
    /// A monitor's: the colour management made on a thread of its own; none for an export.
    background: Option<Background>,
    /// The media of each clip of media of the timeline, by the clip.
    media: HashMap<String, String>,
    /// The project and the timeline that the grades are of.
    of: Option<(Arc<Project>, usize)>,
    clips: HashMap<String, ClipGrades>,
    /// The pre-clip and post-clip grades of each group that a clip of the timeline is in.
    groups: HashMap<String, GroupGrades>,
    timeline: Option<Arc<Prepared>>,
    missing: Vec<MissingLut>,
    /// How many grades were made ready, for the tests.
    made: u64,
}

impl Grading {
    /// The grades of an export: its colour management made before its first frame.
    pub fn new(places: LutPlaces) -> Grading {
        Grading {
            places,
            options: GradeOptions::default(),
            interpolation: LutInterpolation::default(),
            card: None,
            colour: Arc::new(Colour::unmanaged()),
            background: None,
            media: HashMap::new(),
            of: None,
            clips: HashMap::new(),
            groups: HashMap::new(),
            timeline: None,
            missing: Vec::new(),
            made: 0,
        }
    }

    /// The grades of a monitor, whose colour management, once it has some, is made again on a
    /// thread of its own; `wake` tells it to draw again when that is ready.
    pub fn monitor(places: LutPlaces, wake: Arc<dyn Fn() + Send + Sync>) -> Grading {
        let mut grading = Grading::new(places);
        grading.background = Some(Background {
            wake,
            wanted: None,
            job: None,
            failed: None,
        });
        grading
    }

    /// Make ready the grades of timeline `index` of `project` on `gpu`, and its colour
    /// management: nothing to do where they are of that project and card already. Gives whether
    /// anything was made again.
    #[allow(
        clippy::too_many_lines,
        reason = "the card, the colour, then each grade, in the order they depend on each other"
    )]
    pub fn update(&mut self, gpu: &Arc<Gpu>, project: &Arc<Project>, index: usize) -> bool {
        let same_card = self
            .card
            .as_ref()
            .is_some_and(|(card, _, _)| Arc::ptr_eq(card, gpu));
        let ready = self.ready_colour();
        if same_card
            && !ready
            && self
                .of
                .as_ref()
                .is_some_and(|(of, at)| Arc::ptr_eq(of, project) && *at == index)
        {
            return false;
        }
        if !same_card {
            self.card = Some((
                Arc::clone(gpu),
                CardLuts::new(
                    Arc::clone(gpu),
                    Arc::clone(&self.places.cache),
                    self.places.roots.clone(),
                ),
                Arc::new(CardOps::new(Arc::clone(gpu))),
            ));
            self.clips.clear();
            self.groups.clear();
            self.timeline = None;
        }
        self.update_colour(project, same_card);
        // The options of the grades, by the colour in force: another luma, S-curve or
        // interpolation makes every grade again.
        let settings = self.colour.key().settings();
        let options = GradeOptions {
            luma: self.colour.luma(),
            s_curve_contrast: settings.s_curve_contrast,
        };
        let interpolation = settings.lut_interpolation;
        let fresh = options != self.options || interpolation != self.interpolation;
        if fresh {
            self.options = options;
            self.interpolation = interpolation;
            if let Some((_, luts, _)) = &mut self.card {
                luts.set_interpolation(match interpolation {
                    LutInterpolation::Trilinear => Interpolation::Trilinear,
                    LutInterpolation::Tetrahedral => Interpolation::Tetrahedral,
                });
            }
        }
        let made = self.made;
        let timeline = &project.timelines[index];
        self.media = video_clips(timeline)
            .filter_map(|clip| match &clip.source {
                ClipSource::Media { media, .. } => Some((clip.id.clone(), media.clone())),
                _ => None,
            })
            .collect();
        let mut missing = BTreeMap::new();
        let mut old_groups = std::mem::take(&mut self.groups);
        let mut old_clips = std::mem::take(&mut self.clips);
        let mut old_timeline = self.timeline.take();
        if fresh {
            old_groups.clear();
            old_clips.clear();
            old_timeline = None;
        }
        self.timeline = self.ready(&timeline.grade, old_timeline.as_ref(), &mut missing);
        for clip in video_clips(timeline) {
            let group = clip
                .group
                .as_ref()
                .and_then(|id| project.groups.iter().find(|group| group.id == *id));
            let (pre, post) = match group {
                Some(group) => {
                    if let Some(made) = self.groups.get(&group.id) {
                        made.clone()
                    } else {
                        let old = old_groups.get(&group.id);
                        let made = (
                            self.ready(
                                &group.pre,
                                old.and_then(|old| old.0.as_ref()),
                                &mut missing,
                            ),
                            self.ready(
                                &group.post,
                                old.and_then(|old| old.1.as_ref()),
                                &mut missing,
                            ),
                        );
                        self.groups.insert(group.id.clone(), made.clone());
                        made
                    }
                }
                None => (None, None),
            };
            let own = self.ready(
                &clip.grade,
                old_clips.get(&clip.id).and_then(|old| old.own.as_ref()),
                &mut missing,
            );
            if pre.is_some() || own.is_some() || post.is_some() {
                self.clips.insert(
                    clip.id.clone(),
                    ClipGrades {
                        clock: KeyClock::of(clip, project, timeline.rate),
                        pre,
                        own,
                        post,
                    },
                );
            }
        }
        self.missing = missing
            .into_iter()
            .map(|(name, message)| MissingLut { name, message })
            .collect();
        self.of = Some((Arc::clone(project), index));
        self.made != made
    }

    /// `grade` made ready: `old` where it was made of the same grade and all its LUTs were read;
    /// none where it changes nothing. The LUTs that it names and that cannot be read are put in
    /// `missing`.
    fn ready(
        &mut self,
        grade: &Grade,
        old: Option<&Arc<Prepared>>,
        missing: &mut BTreeMap<String, String>,
    ) -> Option<Arc<Prepared>> {
        let names = lut_names(grade);
        if grade.nodes.is_empty() {
            return None;
        }
        let (_, luts, _) = self
            .card
            .as_ref()
            .expect("a card is had before grades are made");
        let unread = |missing: &mut BTreeMap<String, String>| {
            let mut any = false;
            for name in &names {
                if let Some(error) = luts.error(name) {
                    missing.insert(name.clone(), error);
                    any = true;
                }
            }
            any
        };
        if let Some(old) = old.filter(|old| old.grade() == grade) {
            if names.is_empty() || !unread(&mut BTreeMap::new()) {
                return (!old.is_identity()).then(|| Arc::clone(old));
            }
        }
        luts.prepare(grade);
        unread(missing);
        let prepared = Prepared::new(grade, &self.options, luts);
        self.made += 1;
        (!prepared.is_identity()).then(|| Arc::new(prepared))
    }

    /// The grades of the layers of `scene` and of its timeline at `frame` of the timeline, each
    /// layer that of the clip named at its place in `clips` (one that is no clip, as the colour
    /// of a dip, has none).
    pub fn grade<S: AsRef<str>>(&self, scene: &mut Scene, clips: &[S], frame: i64) {
        let media: Vec<Option<&str>> = clips
            .iter()
            .map(|clip| self.media.get(clip.as_ref()).map(String::as_str))
            .collect();
        self.colour.apply(scene, &media);
        for (layer, clip) in scene.layers.iter_mut().zip(clips) {
            layer.color.grade = self.layer(clip.as_ref(), frame);
        }
        scene.grade = Stage {
            ops: self
                .timeline
                .iter()
                .flat_map(|grade| grade.ops_at(at(frame)))
                .collect(),
        };
    }

    /// The grade of the layer of clip `clip` at `frame` of the timeline: its group's pre-clip
    /// grade, its own and its group's post-clip grade, each at the frame of its keys there, that
    /// of the picture of its source shown.
    pub fn layer(&self, clip: &str, frame: i64) -> Stage {
        let Some(grades) = self.clips.get(clip) else {
            return Stage::default();
        };
        let frame = at(grades.clock.at(frame));
        let ops: Vec<ColorOp> = [&grades.pre, &grades.own, &grades.post]
            .into_iter()
            .flatten()
            .flat_map(|grade| grade.ops_at(frame))
            .collect();
        Stage { ops }
    }

    /// What the frames are put out as, by the colour management in force.
    pub fn output(&self) -> Output {
        self.colour.output()
    }

    /// The luma coefficients of the timeline space, which the grades take.
    pub fn luma(&self) -> [f32; 3] {
        self.colour.luma()
    }

    /// The colour management of `project`: made here for an export, for the first project of a
    /// monitor, and on a new card (`same_card` false); else wanted of a thread of its own
    /// ([`Grading::make_ahead`]), the colour in force kept meanwhile.
    fn update_colour(&mut self, project: &Project, same_card: bool) {
        let key = Key::of(project);
        let first = self.of.is_none() && !self.colour.key().managed();
        if same_card && key == *self.colour.key() {
            if let Some(background) = &mut self.background {
                // Back to what is in force: nothing more is wanted.
                background.wanted = None;
            }
            return;
        }
        if let Some(background) = &mut self.background {
            if same_card && !first {
                if background.failed.as_ref() != Some(&key)
                    && background.job.as_ref().map(|(made, _)| made) != Some(&key)
                {
                    background.wanted = Some(key);
                }
                return;
            }
        }
        let ops = Arc::clone(&self.card.as_ref().expect("a card is had").2);
        let since = Instant::now();
        match Colour::make(&ops, key) {
            Ok(colour) => {
                crate::log::log(
                    "colour_made",
                    &json!({"ms": since.elapsed().as_secs_f64() * 1e3, "waited": true}),
                );
                self.colour = Arc::new(colour);
            }
            Err(message) => crate::log::log("colour_error", &json!({ "message": message })),
        }
    }

    /// Whether a colour management made on a thread of its own is ready, and taken in place of
    /// the one in force.
    fn ready_colour(&mut self) -> bool {
        let Some(background) = &mut self.background else {
            return false;
        };
        let made = match &background.job {
            Some((_, done)) => done.lock().unwrap_or_else(PoisonError::into_inner).take(),
            None => None,
        };
        let Some(made) = made else {
            return false;
        };
        let (key, _) = background.job.take().expect("looked at above");
        match made {
            Ok(colour) => {
                self.colour = colour;
                true
            }
            Err(message) => {
                crate::log::log("colour_error", &json!({ "message": message }));
                background.failed = Some(key);
                false
            }
        }
    }

    /// Whether a colour management is being made, or wanted, on a thread of its own.
    #[cfg(test)]
    pub fn making(&self) -> bool {
        self.background
            .as_ref()
            .is_some_and(|background| background.job.is_some() || background.wanted.is_some())
    }

    /// Where another colour management is wanted and none is being made, make it on a thread of
    /// its own, and draw `scene` (the frame just drawn, of the clips `clips` at `frame` of
    /// timeline `index` of `project`) small by it there, so that its shaders are compiled off the
    /// monitor's thread. A monitor calls it after each frame it grades.
    pub fn make_ahead<S: AsRef<str>>(
        &mut self,
        project: &Arc<Project>,
        index: usize,
        (scene, clips, frame): (&Scene, &[S], i64),
    ) {
        let Some(background) = &mut self.background else {
            return;
        };
        if background.job.is_some() {
            return;
        }
        let Some(key) = background.wanted.take() else {
            return;
        };
        let Some((gpu, _, ops)) = &self.card else {
            return;
        };
        let done = Arc::new(Mutex::new(None));
        background.job = Some((key.clone(), Arc::clone(&done)));
        let warm = Warm {
            gpu: Arc::clone(gpu),
            ops: Arc::clone(ops),
            places: self.places.clone(),
            project: Arc::clone(project),
            index,
            scene: stripped(scene),
            clips: clips.iter().map(|clip| clip.as_ref().to_owned()).collect(),
            frame,
        };
        let wake = Arc::clone(&background.wake);
        let finished = Arc::clone(&done);
        let started = std::thread::Builder::new()
            .name("columbina-colour".into())
            .spawn(move || {
                let since = Instant::now();
                let made = warm.make(key);
                crate::log::log(
                    "colour_made",
                    &json!({"ms": since.elapsed().as_secs_f64() * 1e3, "waited": false,
                        "ok": made.is_ok()}),
                );
                *finished.lock().unwrap_or_else(PoisonError::into_inner) = Some(made);
                wake();
            });
        if let Err(error) = started {
            *done.lock().unwrap_or_else(PoisonError::into_inner) = Some(Err(format!(
                "the thread of colour cannot be started: {error}"
            )));
        }
    }

    /// Let go of the project and its grades (the LUTs read stay, on the card).
    pub fn clear(&mut self) {
        self.of = None;
        self.clips.clear();
        self.groups.clear();
        self.timeline = None;
        self.missing.clear();
    }

    /// The LUTs that the grades of the timeline name and that cannot be read, by name.
    pub fn missing(&self) -> &[MissingLut] {
        &self.missing
    }

    /// How many grades were made ready since it was made.
    #[cfg(test)]
    pub fn made(&self) -> u64 {
        self.made
    }
}

/// What a thread of its own makes a monitor's colour management by.
struct Warm {
    gpu: Arc<Gpu>,
    ops: Arc<CardOps>,
    places: LutPlaces,
    project: Arc<Project>,
    index: usize,
    scene: Scene,
    clips: Vec<String>,
    frame: i64,
}

impl Warm {
    /// The colour management of `key`, its shaders and those of the grades by it compiled by
    /// drawing the frame small.
    fn make(mut self, key: Key) -> Made {
        let colour = Arc::new(Colour::make(&self.ops, key)?);
        let mut grading = Grading::new(self.places.clone());
        grading.card = Some((
            Arc::clone(&self.gpu),
            CardLuts::new(
                Arc::clone(&self.gpu),
                Arc::clone(&self.places.cache),
                self.places.roots.clone(),
            ),
            Arc::clone(&self.ops),
        ));
        grading.colour = Arc::clone(&colour);
        grading.update(&self.gpu, &self.project, self.index);
        grading.grade(&mut self.scene, &self.clips, self.frame);
        let target = Target::preview(&self.gpu, WARM.0, WARM.1).map_err(|e| e.to_string())?;
        Compositor::new(Arc::clone(&self.gpu))
            .render(&self.scene, &target)
            .map_err(|error| error.to_string())?;
        Ok(colour)
    }
}

/// A scene as it was before it was graded, small: its layers with their decoding only.
fn stripped(scene: &Scene) -> Scene {
    Scene {
        width: WARM.0,
        height: WARM.1,
        layers: scene
            .layers
            .iter()
            .map(|layer| {
                let mut layer = layer.clone();
                layer.color = LayerColor {
                    coding: layer.color.coding,
                    ..LayerColor::default()
                };
                layer
            })
            .collect(),
        ..Scene::default()
    }
}

/// A frame as a grade takes it.
#[allow(clippy::cast_precision_loss, reason = "frames are within 2^53")]
fn at(frame: i64) -> f64 {
    frame as f64
}

/// The clips of the tracks of picture of a timeline.
fn video_clips(timeline: &Timeline) -> impl Iterator<Item = &columbina_model::Clip> {
    timeline
        .tracks
        .iter()
        .filter(|track| track.kind == TrackKind::Video)
        .flat_map(|track| &track.clips)
}

/// The names of the LUTs of the correctors of `grade`.
fn lut_names(grade: &Grade) -> Vec<String> {
    grade
        .nodes
        .iter()
        .filter_map(|node| match &node.kind {
            NodeKind::Corrector(corrector) => corrector.lut.clone(),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_library_is_the_folders_named_or_the_apps_own() {
        let home = Path::new(if cfg!(windows) { r"C:\h" } else { "/h" });
        assert_eq!(library_of(None, Some(home)), vec![home.join("luts")]);
        assert_eq!(library_of(None, None), Vec::<PathBuf>::new());
        let (a, b) = if cfg!(windows) {
            (r"C:\a", r"D:\b c")
        } else {
            ("/a", "/b c")
        };
        let list = std::env::join_paths([a, "relative", b]).unwrap();
        assert_eq!(
            library_of(Some(&list), Some(home)),
            vec![PathBuf::from(a), PathBuf::from(b)]
        );
    }
}
