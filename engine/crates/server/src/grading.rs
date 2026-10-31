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
//! The keys of a clip's grade, and of the grades of its group as they apply to that clip, count
//! the frames of the timeline from the clip's first frame, 0 (the model's `Param`); those of the
//! timeline's grade count the frames of the timeline. So a key at 10 is where the playhead is
//! ten frames into the clip, whatever the speed of the clip and whether it plays backwards: the
//! Keyframe Editor of the Color page has a ruler that "mirrors the record timecode of the
//! currently selected clip" (p. 3382), and a change of speed moves only the keys of the
//! Composite, Transform and Cropping of the Edit page with it, and only when asked (p. 1073).
//! `DaVinci` ties a key to the frame of the source under it (copied grades place their keys "at
//! matching frames that correspond to the source timecode", p. 3393; the first key is "on the
//! first frame of the master clip", p. 3383), so a trim of a clip's head there leaves its keys on
//! the same pictures; here they stay at the same distance from the clip's first frame. Within a
//! transition a clip is seen before its first frame or after its last, at frames below 0 or past
//! its length, where a grade holds its first or last key.
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
//! # LUTs
//!
//! A LUT that a corrector names is looked for in the folder of the project, then in the library
//! of LUTs ([`library`]). One that cannot be read leaves its node as though it named none, and is
//! told ([`Grading::missing`]): in the state of the timeline monitor and in the progress of an
//! export (`missing_luts`), as `DaVinci` shows "Missing LUT" (p. 3413).

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use columbina_compose::grade::{CardLuts, GradeOptions, LutCache, Prepared, Roots};
use columbina_compose::{ColorOp, Gpu, Scene, Stage};
use columbina_model::grade::{Grade, NodeKind};
use columbina_model::project::TrackKind;
use columbina_model::{Project, Timeline};
use serde::Serialize;

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
    /// The first frame of the clip on the timeline, from which its keys count.
    start: i64,
    pre: Option<Arc<Prepared>>,
    own: Option<Arc<Prepared>>,
    post: Option<Arc<Prepared>>,
}

/// The pre-clip and post-clip grades of a group, ready.
type GroupGrades = (Option<Arc<Prepared>>, Option<Arc<Prepared>>);

/// The grades of one timeline of a project, ready on one card.
pub struct Grading {
    places: LutPlaces,
    options: GradeOptions,
    /// The card, and its LUTs.
    card: Option<(Arc<Gpu>, CardLuts)>,
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
    pub fn new(places: LutPlaces) -> Grading {
        Grading {
            places,
            options: GradeOptions::default(),
            card: None,
            of: None,
            clips: HashMap::new(),
            groups: HashMap::new(),
            timeline: None,
            missing: Vec::new(),
            made: 0,
        }
    }

    /// Make ready the grades of timeline `index` of `project` on `gpu`: nothing to do where they
    /// are of that project and card already. Gives whether anything was made again.
    pub fn update(&mut self, gpu: &Arc<Gpu>, project: &Arc<Project>, index: usize) -> bool {
        let same_card = self
            .card
            .as_ref()
            .is_some_and(|(card, _)| Arc::ptr_eq(card, gpu));
        if same_card
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
            ));
            self.clips.clear();
            self.groups.clear();
            self.timeline = None;
        }
        let made = self.made;
        let timeline = &project.timelines[index];
        let mut missing = BTreeMap::new();
        let old_groups = std::mem::take(&mut self.groups);
        let old_clips = std::mem::take(&mut self.clips);
        let old_timeline = self.timeline.take();
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
                        start: clip.start,
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
        let (_, luts) = self
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
    /// grade, its own and its group's post-clip grade, each at the frame of the clip.
    pub fn layer(&self, clip: &str, frame: i64) -> Stage {
        let Some(grades) = self.clips.get(clip) else {
            return Stage::default();
        };
        let frame = at(frame.saturating_sub(grades.start));
        let ops: Vec<ColorOp> = [&grades.pre, &grades.own, &grades.post]
            .into_iter()
            .flatten()
            .flat_map(|grade| grade.ops_at(frame))
            .collect();
        Stage { ops }
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
