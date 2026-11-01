//! `grade.export_lut`: a grade, or one node of it, baked into a 3D LUT and written as a `.cube`,
//! as Generate LUT of `DaVinci Resolve` makes one from a clip's thumbnail (RM20 pp. 3413–3414):
//! of 17, 33 or 65 points on an edge.
//!
//! # What is baked
//!
//! "Primaries palette adjustments, Custom Curves palette adjustments" and the like, "mathematically
//! combined", nodes that use Qualifiers, Windows or "incompatible filtering operations (such as
//! sharpening or blurring) … completely ignored, as will all other correction operations made
//! within these nodes" (p. 3413). So the cube is what the grade does to each colour on its own:
//! every node that works on a pixel alone, by the same maths as the card's reference on the
//! processor (`columbina_compose::grade::evaluate_with`), the Primaries in both their modes, the
//! curves (custom and HSL), Soft Clip, Shot Match and the LUT of a node (the same LUT that the
//! card reads, between its points as the project says), through the node's key settings and the
//! mixers as the grade links them. A node whose result depends on the pixels about each one, as
//! Midtone Detail makes it (p. 3063), is left out whole, as though it were off, and said in the
//! answer; so is a node with a qualifier or an enabled complete Power Window, including one
//! whose Size or Opacity is zero. Disabled windows and open Curve drafts are ignored.
//!
//! The cube is in the timeline space, as the grade works there: over 0 to 1 of each channel
//! (the domain of a cube that says none), what is beyond it clipped as any cube clips it
//! (p. 225); its values as the grade gives them, beyond 0 to 1 too. A grade's keys are read at
//! one frame.
//!
//! # Where it is written
//!
//! `path` is either a name relative to the folder of the project, as a corrector's `lut` names a
//! LUT there (plain names, no `..`, no root, no link or junction on the way, not into `cache`),
//! so that the LUT is listed by `luts.list` and can be put on a node by that name; or an
//! absolute path that the user chose (a file dialog of the desktop), as `render.start` takes,
//! whose folder must be there and not in the project's cache, and which is no file of the
//! project's media. Either is a `.cube`; an existing file is replaced only where `replace` says
//! so, and only a file. It is written whole beside its place and then put there, so that no one
//! reads half of it.

use std::fs;
use std::io::Write as _;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use columbina_compose::grade::{evaluate_with, GradeOptions, Luts};
use columbina_compose::ColorOp;
use columbina_lut::{Domain, Interpolation, Lut, Lut3d};
use columbina_model::grade::{Grade, Link, Node, NodeKind, Target, OUTPUT, SOURCE};
use columbina_model::keyframes::KeyClock;
use columbina_model::project::TrackKind;
use columbina_model::{Param, Project};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::grading::{LutPlaces, MissingLut};
use crate::rpc::{self, Fault};

/// The points on an edge of a cube that Generate LUT makes (p. 3413).
pub const SIZES: [usize; 3] = [17, 33, 65];

/// The params of `grade.export_lut`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// The grade.
    pub grade: Target,
    /// One node of it to bake alone, or none for the whole grade.
    #[serde(default)]
    pub node: Option<String>,
    /// The points on an edge: 17, 33 or 65.
    pub size: usize,
    /// Where it is written: relative to the folder of the project, or absolute.
    pub path: String,
    /// The frame that the keys of the grade are read at: of the timeline for a clip's grade
    /// and a timeline's; of its own keys for a group's. At first, the clip's first frame, or 0.
    #[serde(default)]
    pub frame: Option<i64>,
    /// Whether a file that is there is replaced.
    #[serde(default)]
    pub replace: bool,
}

/// What the method takes, for its errors.
pub const TAKES: &str =
    r#"{"grade":{"level":"clip","timeline":"t1","clip":"c1"},"size":33,"path":"luts/look.cube"}"#;

impl Request {
    /// The params of `grade.export_lut`.
    ///
    /// # Errors
    /// They are not what it takes, or the size is not one of [`SIZES`].
    pub fn read(params: Option<&serde_json::value::RawValue>) -> Result<Request, Fault> {
        let request: Request = rpc::params("grade.export_lut", TAKES, params)?;
        if !SIZES.contains(&request.size) {
            return Err(Fault::params(format!(
                "a cube is of 17, 33 or 65 points on an edge, not {}",
                request.size
            ))
            .with_data(json!({"reason": "size", "size": request.size})));
        }
        Ok(request)
    }
}

/// Bake the grade that `request` names in `project` (whose folder is `dir`, its LUTs read from
/// `places`) and write it; give the answer.
///
/// # Errors
/// See PROTOCOL.md, `grade.export_lut`.
pub fn export(
    project: &Project,
    dir: &Path,
    places: &LutPlaces,
    request: &Request,
) -> Result<Value, Fault> {
    let path = destination(&request.path, dir, project, request.replace)?;
    let (grade, frame) = grade_at(project, &request.grade, request.frame)?;
    let (baked, ignored) = match &request.node {
        Some(node) => (alone(grade, node)?, Vec::new()),
        None => without_filters(grade),
    };
    let options = GradeOptions {
        luma: crate::managed::grade_luma(&project.color)
            .map_err(|message| Fault::new(rpc::CANNOT_READ, message))?,
        s_curve_contrast: project.color.s_curve_contrast,
    };
    let interpolation = crate::managed::interpolation(project.color.lut_interpolation);
    let tables = Tables::read(&baked, places)?;
    let cube = bake(
        &baked,
        frame,
        &options,
        &tables,
        interpolation,
        request.size,
    )
    .map_err(|message| Fault::new(rpc::CANNOT_READ, message))?;
    let title = match &request.node {
        Some(node) => format!("Columbina: node {node} of {}", described(&request.grade)),
        None => format!("Columbina: {}", described(&request.grade)),
    };
    let text = Lut::new(Some(title), None, Some(cube))
        .and_then(|lut| lut.to_cube())
        .map_err(|error| Fault::new(rpc::CANNOT_READ, error.to_string()))?;
    write(&path, text.as_bytes(), request.replace)?;
    places.cache.forget(&path);
    let nodes: Vec<&str> = baked
        .nodes
        .iter()
        .filter(|node| node.enabled)
        .map(|node| node.id.as_str())
        .collect();
    Ok(json!({
        "path": path,
        "name": name_of(&path, places),
        "size": request.size,
        "nodes": nodes,
        "ignored": ignored,
    }))
}

/// What a grade is, for the title of its cube.
fn described(target: &Target) -> String {
    match target {
        Target::Clip { timeline, clip } => format!("clip {clip} of timeline {timeline}"),
        Target::PreClip { group } => format!("pre-clip grade of group {group}"),
        Target::PostClip { group } => format!("post-clip grade of group {group}"),
        Target::Timeline { timeline } => format!("timeline {timeline}"),
    }
}

/// A grade, or a node of one, that cannot be baked: `-32602` with `reason` and what more.
fn refused(message: String, data: Value) -> Fault {
    Fault::params(message).with_data(data)
}

/// The grade that `target` names, and the frame its keys are read at.
fn grade_at<'p>(
    project: &'p Project,
    target: &Target,
    frame: Option<i64>,
) -> Result<(&'p Grade, f64), Fault> {
    let timeline_of = |id: &str| {
        project
            .timelines
            .iter()
            .find(|timeline| timeline.id == id)
            .ok_or_else(|| {
                refused(
                    format!("there is no timeline {id}"),
                    json!({"reason": "missing", "timeline": id}),
                )
            })
    };
    let group_of = |id: &str| {
        project
            .groups
            .iter()
            .find(|group| group.id == id)
            .ok_or_else(|| {
                refused(
                    format!("there is no group {id}"),
                    json!({"reason": "missing", "group": id}),
                )
            })
    };
    #[allow(clippy::cast_precision_loss, reason = "frames are within 2^53")]
    let at = |frame: i64| frame as f64;
    Ok(match target {
        Target::Clip { timeline, clip } => {
            let found = timeline_of(timeline)?;
            let picked = found
                .tracks
                .iter()
                .filter(|track| track.kind == TrackKind::Video)
                .flat_map(|track| &track.clips)
                .find(|candidate| candidate.id == *clip)
                .ok_or_else(|| {
                    refused(
                        format!("there is no clip {clip} of picture in timeline {timeline}"),
                        json!({"reason": "missing", "clip": clip}),
                    )
                })?;
            let clock = KeyClock::of(picked, project, found.rate);
            let shown = frame.unwrap_or(picked.start);
            (&picked.grade, at(clock.at(shown)))
        }
        Target::PreClip { group } => (&group_of(group)?.pre, at(frame.unwrap_or(0))),
        Target::PostClip { group } => (&group_of(group)?.post, at(frame.unwrap_or(0))),
        Target::Timeline { timeline } => (&timeline_of(timeline)?.grade, at(frame.unwrap_or(0))),
    })
}

/// Why a corrector cannot be baked, if it cannot: it reads the pixels about each one, or it has a
/// qualifier, whose node "will be completely ignored" (p. 3413).
fn unbakeable(node: &Node) -> Option<&'static str> {
    let NodeKind::Corrector(corrector) = &node.kind else {
        return None;
    };
    if corrector.qualifier.is_some() {
        return Some("qualifier");
    }
    if corrector
        .windows
        .iter()
        .any(|w| w.enabled && !w.shape.is_draft())
    {
        return Some("windows");
    }
    (!matches!(corrector.primaries.midtone_detail, Param::Fixed(value) if value == 0.0))
        .then_some("midtone_detail")
}

/// `grade` with the nodes that cannot be baked off; and those nodes, with why.
fn without_filters(grade: &Grade) -> (Grade, Vec<Value>) {
    let mut baked = grade.clone();
    let mut ignored = Vec::new();
    for node in &mut baked.nodes {
        if let Some(why) = unbakeable(node).filter(|_| node.enabled) {
            node.enabled = false;
            ignored.push(json!({"node": node.id, "why": why}));
        }
    }
    (baked, ignored)
}

/// The grade of node `id` of `grade` alone, between the source and the output, on whatever its
/// own switch says: what it does, asked for by name.
fn alone(grade: &Grade, id: &str) -> Result<Grade, Fault> {
    let node = grade
        .nodes
        .iter()
        .find(|node| node.id == id)
        .ok_or_else(|| {
            refused(
                format!("the grade has no node {id}"),
                json!({"reason": "missing", "node": id}),
            )
        })?;
    if !matches!(node.kind, NodeKind::Corrector(_)) {
        return Err(refused(
            format!("node {id} is not a corrector, and does nothing alone"),
            json!({"reason": "not_a_corrector", "node": id}),
        ));
    }
    if let Some(why) = unbakeable(node) {
        return Err(refused(
            format!("node {id} uses {why}, whose whole node is excluded from Generate LUT"),
            json!({"reason": "cannot_bake", "node": id, "why": why}),
        ));
    }
    let mut node = node.clone();
    node.enabled = true;
    Ok(Grade {
        nodes: vec![node],
        links: vec![Link::rgb(SOURCE, id), Link::rgb(id, OUTPUT)],
    })
}

/// The tables of the LUTs that the nodes of a grade name, read as the card reads them.
struct Tables {
    read: std::collections::HashMap<String, Arc<Lut>>,
}

impl Tables {
    /// Those of the correctors of `grade` that are on.
    ///
    /// # Errors
    /// A LUT cannot be read: the cube would not be what the grade does (`-32001`,
    /// `missing_luts`).
    fn read(grade: &Grade, places: &LutPlaces) -> Result<Tables, Fault> {
        let mut read = std::collections::HashMap::new();
        let mut missing = Vec::new();
        for node in grade.nodes.iter().filter(|node| node.enabled) {
            let NodeKind::Corrector(corrector) = &node.kind else {
                continue;
            };
            let Some(name) = &corrector.lut else {
                continue;
            };
            if read.contains_key(name) {
                continue;
            }
            match places.cache.load_named(&places.roots, name) {
                Ok(loaded) => {
                    read.insert(name.clone(), loaded.lut);
                }
                Err(error) => missing.push(MissingLut {
                    name: name.clone(),
                    message: error.to_string(),
                }),
            }
        }
        if !missing.is_empty() {
            missing.sort();
            missing.dedup();
            return Err(
                Fault::new(rpc::REFUSED, "a LUT that the grade names cannot be read")
                    .with_data(json!({"reason": "missing_luts", "missing_luts": missing})),
            );
        }
        Ok(Tables { read })
    }
}

impl Luts for Tables {
    /// A node has its LUT where it was read; the processor does it by its table, so the
    /// operation is of no use but to say so.
    fn lut(&self, name: &str) -> Option<ColorOp> {
        self.read.contains_key(name).then(ColorOp::default)
    }
}

/// The cube of `size` points of what `grade` does at `frame`, each point on the processor; the
/// planes of blue on threads of their own, at most eight.
fn bake(
    grade: &Grade,
    frame: f64,
    options: &GradeOptions,
    tables: &Tables,
    interpolation: Interpolation,
    size: usize,
) -> Result<Lut3d, String> {
    let lut = |name: &str, rgb: [f32; 3]| {
        tables
            .read
            .get(name)
            .map_or(rgb, |table| table.eval(rgb, interpolation))
    };
    #[allow(clippy::cast_precision_loss, reason = "points of a cube")]
    let at = |i: usize| i as f64 / (size - 1) as f64;
    let threads = std::thread::available_parallelism()
        .map_or(4, std::num::NonZero::get)
        .clamp(1, 8);
    let per = size.div_ceil(threads);
    let planes: Vec<Vec<[f32; 3]>> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..threads)
            .map(|part| {
                let lut = &lut;
                scope.spawn(move || {
                    let mut rows = Vec::new();
                    for b in part * per..((part + 1) * per).min(size) {
                        for g in 0..size {
                            for r in 0..size {
                                #[allow(
                                    clippy::cast_possible_truncation,
                                    reason = "a colour of the card"
                                )]
                                let rgb = [at(r) as f32, at(g) as f32, at(b) as f32];
                                rows.push(evaluate_with(grade, frame, options, tables, lut, rgb));
                            }
                        }
                    }
                    rows
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap_or_default())
            .collect()
    });
    let table: Vec<[f32; 3]> = planes.into_iter().flatten().collect();
    Lut3d::new(size, Domain::default(), table).map_err(|error| error.to_string())
}

/// Where a cube named `path` is written: refused as `render.start` refuses its path, or as
/// `luts.list` its folder.
fn destination(path: &str, dir: &Path, project: &Project, replace: bool) -> Result<PathBuf, Fault> {
    let bad = |message: String| {
        refused(
            message.clone(),
            json!({"reason": "path", "path": path, "message": message}),
        )
    };
    let given = Path::new(path);
    let extension = given.extension().and_then(|e| e.to_str()).unwrap_or("");
    if !extension.eq_ignore_ascii_case("cube") || given.file_stem().is_none() {
        return Err(bad("a LUT is written as a .cube".into()));
    }
    let full = if given.is_absolute() {
        given.to_path_buf()
    } else {
        // A name of plain names within the folder of the project, through no link.
        if path.contains(':') || path.contains('\0') {
            return Err(bad("a name of a LUT is of plain names".into()));
        }
        let mut full = dir.to_path_buf();
        let parts: Vec<_> = given.components().collect();
        for (index, part) in parts.iter().enumerate() {
            let Component::Normal(part) = part else {
                return Err(bad(
                    "a name of a LUT is of plain names within the project".into()
                ));
            };
            if index == 0 && part.eq_ignore_ascii_case("cache") {
                return Err(bad("the cache of the project is the engine's own".into()));
            }
            full.push(part);
            if index + 1 < parts.len() {
                match fs::symlink_metadata(&full) {
                    Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {}
                    Ok(_) => return Err(bad(format!("{} is no folder", full.display()))),
                    Err(_) => return Err(bad(format!("there is no folder {}", full.display()))),
                }
            }
        }
        full
    };
    let parent = full.parent().expect("a file in a folder");
    let parent = fs::canonicalize(parent)
        .map_err(|_| bad("the folder of the LUT must already be there".into()))?;
    if !parent.is_dir() {
        return Err(bad("the folder of the LUT is not a folder".into()));
    }
    if fs::canonicalize(dir.join("cache")).is_ok_and(|cache| parent.starts_with(cache)) {
        return Err(bad("the cache of the project is the engine's own".into()));
    }
    for media in &project.media {
        let proxy = media.proxy.as_ref().map(|proxy| Path::new(&proxy.path));
        for source in std::iter::once(Path::new(&media.path)).chain(proxy) {
            if same_file(source, &full) {
                return Err(Fault::new(
                    rpc::REFUSED,
                    format!(
                        "the LUT would be written over {path}, which media {} reads",
                        media.id
                    ),
                )
                .with_data(json!({"reason": "source", "path": path, "media": media.id})));
            }
        }
    }
    match fs::symlink_metadata(&full) {
        Ok(meta) if !meta.file_type().is_file() => {
            Err(bad("the path of the LUT is not a file".into()))
        }
        Ok(_) if !replace => Err(Fault::new(rpc::REFUSED, "the LUT is there already")
            .with_data(json!({"reason": "exists", "path": full}))),
        _ => Ok(full),
    }
}

/// Whether two paths are of one file, as the file system finds them (case aside on Windows).
fn same_file(a: &Path, b: &Path) -> bool {
    let identity = |path: &Path| {
        let found = fs::canonicalize(path).ok().or_else(|| {
            let parent = fs::canonicalize(path.parent()?).ok()?;
            Some(parent.join(path.file_name()?))
        });
        let path = found.unwrap_or_else(|| path.to_owned());
        if cfg!(windows) {
            PathBuf::from(path.to_string_lossy().to_lowercase())
        } else {
            path
        }
    };
    identity(a) == identity(b)
}

/// Write `bytes` to `path` whole: into a file of its own beside it, then put in its place
/// (over a file there only where `replace` says so).
fn write(path: &Path, bytes: &[u8], replace: bool) -> Result<(), Fault> {
    let failed = |message: String| {
        Fault::new(rpc::REFUSED, message.clone())
            .with_data(json!({"reason": "write", "path": path, "message": message}))
    };
    let parent = path.parent().expect("a file in a folder");
    let mut file = tempfile::Builder::new()
        .prefix(".columbina-lut-")
        .suffix(".part")
        .tempfile_in(parent)
        .map_err(|error| failed(error.to_string()))?;
    file.write_all(bytes)
        .and_then(|()| file.as_file().sync_all())
        .map_err(|error| failed(error.to_string()))?;
    let kept = if replace {
        file.persist(path).map(|_| ())
    } else {
        file.persist_noclobber(path).map(|_| ())
    };
    kept.map_err(|error| {
        if error.error.kind() == std::io::ErrorKind::AlreadyExists {
            Fault::new(rpc::REFUSED, "the LUT is there already")
                .with_data(json!({"reason": "exists", "path": path}))
        } else {
            failed(error.error.to_string())
        }
    })
}

/// The name that a corrector gives to the LUT at `path`: relative to the first root it is in,
/// with `/`, or the path itself.
fn name_of(path: &Path, places: &LutPlaces) -> String {
    let roots = places
        .roots
        .project
        .iter()
        .chain(&places.roots.library)
        .filter_map(|root| fs::canonicalize(root).ok());
    let file = fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
    for root in roots {
        if let Ok(rest) = file.strip_prefix(&root) {
            let parts: Vec<String> = rest
                .components()
                .map(|part| part.as_os_str().to_string_lossy().into_owned())
                .collect();
            return parts.join("/");
        }
    }
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use columbina_lut::parse_cube;
    use columbina_model::{Corrector, Param};

    fn places(dir: &Path) -> LutPlaces {
        LutPlaces::with_library(dir, vec![])
    }

    /// A project of one timeline whose clip of colour has `grade`.
    fn project_with(grade: Grade) -> Project {
        let mut project = Project::sample();
        project.timelines[0].tracks[0].clips[0].grade = grade;
        project
    }

    fn corrector(id: &str, set: impl FnOnce(&mut Corrector)) -> Node {
        let mut node = Node::corrector(id);
        let NodeKind::Corrector(corrector) = &mut node.kind else {
            unreachable!("a corrector")
        };
        set(corrector);
        node
    }

    fn serial(nodes: Vec<Node>) -> Grade {
        let mut links = Vec::new();
        let mut from = SOURCE.to_owned();
        for node in &nodes {
            links.push(Link::rgb(from.clone(), node.id.clone()));
            from = node.id.clone();
        }
        links.push(Link::rgb(from, OUTPUT));
        Grade { nodes, links }
    }

    fn target(project: &Project) -> Target {
        Target::Clip {
            timeline: project.timelines[0].id.clone(),
            clip: project.timelines[0].tracks[0].clips[0].id.clone(),
        }
    }

    fn request(project: &Project, path: &str, size: usize) -> Request {
        Request {
            grade: target(project),
            node: None,
            size,
            path: path.into(),
            frame: None,
            replace: false,
        }
    }

    /// A grade of a contrast, a lift of red, a saturation and a Hue vs Sat curve.
    fn graded() -> Grade {
        serial(vec![
            corrector("n1", |c| {
                c.primaries.contrast = Param::Fixed(1.3);
                c.primaries.lift.r = Param::Fixed(0.04);
            }),
            corrector("n2", |c| c.primaries.saturation = Param::Fixed(70.0)),
        ])
    }

    #[test]
    fn a_grade_is_baked_into_a_cube_of_each_size_that_reads_back_as_the_grade() {
        let dir = tempfile::tempdir().unwrap();
        let project = project_with(graded());
        let options = GradeOptions::default();
        let mut errors = Vec::new();
        for (size, within) in [(17, 2e-2), (33, 8e-3), (65, 4e-3)] {
            let name = format!("look{size}.cube");
            let answer = export(
                &project,
                dir.path(),
                &places(dir.path()),
                &request(&project, &name, size),
            )
            .unwrap();
            assert_eq!(answer["name"], name.as_str(), "{answer}");
            assert_eq!(answer["size"], size);
            let text = fs::read(dir.path().join(&name)).unwrap();
            let lut = parse_cube(&text).unwrap();
            let cube = lut.cube().unwrap();
            assert_eq!(cube.size(), size);
            assert!(lut.shaper().is_none());
            // Each point is the grade's, to the float.
            let grade = &project.timelines[0].tracks[0].clips[0].grade;
            #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
            let at = |i: usize| (i as f64 / (size - 1) as f64) as f32;
            for (r, g, b) in [(0, 0, 0), (size - 1, 3, 7), (5, size / 2, size - 1)] {
                let wanted =
                    columbina_compose::grade::evaluate(grade, 0.0, &options, [at(r), at(g), at(b)]);
                assert_eq!(cube.at(r, g, b), wanted);
            }
            // Between the points: within the grade at the corners of its cell, and nearer the
            // grade the more points the cube has.
            let mut most = 0.0f32;
            for i in 0..2000u32 {
                #[allow(clippy::cast_precision_loss)]
                let x = |k: u32| ((i * 7919 + k * 104_729) % 1000) as f32 / 999.0;
                let rgb = [x(1), x(2), x(3)];
                let wanted = columbina_compose::grade::evaluate(grade, 0.0, &options, rgb);
                let read = lut.eval(rgb, Interpolation::Trilinear);
                #[allow(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    clippy::cast_precision_loss
                )]
                let cell = rgb.map(|v| ((v * (size - 1) as f32) as usize).min(size - 2));
                let mut low = [f32::MAX; 3];
                let mut high = [f32::MIN; 3];
                for corner in 0..8 {
                    let point = cube.at(
                        cell[0] + (corner & 1),
                        cell[1] + (corner >> 1 & 1),
                        cell[2] + (corner >> 2),
                    );
                    for c in 0..3 {
                        low[c] = low[c].min(point[c]);
                        high[c] = high[c].max(point[c]);
                    }
                }
                for c in 0..3 {
                    assert!(
                        read[c] >= low[c] - 1e-6 && read[c] <= high[c] + 1e-6,
                        "{size} {rgb:?}"
                    );
                    most = most.max((read[c] - wanted[c]).abs());
                }
            }
            assert!(most < within, "{size}: {most}");
            errors.push(most);
        }
        assert!(errors[0] > errors[1] && errors[1] > errors[2], "{errors:?}");
    }

    #[test]
    fn a_node_that_reads_the_pixels_about_it_is_left_out_and_said() {
        let dir = tempfile::tempdir().unwrap();
        let mut grade = graded();
        let NodeKind::Corrector(c) = &mut grade.nodes[1].kind else {
            unreachable!()
        };
        c.primaries.midtone_detail = Param::Fixed(40.0);
        let project = project_with(grade.clone());
        let answer = export(
            &project,
            dir.path(),
            &places(dir.path()),
            &request(&project, "a.cube", 17),
        )
        .unwrap();
        assert_eq!(
            answer["ignored"],
            json!([{"node": "n2", "why": "midtone_detail"}])
        );
        assert_eq!(answer["nodes"], json!(["n1"]));
        // Baked as the first node alone.
        let mut alone = request(&project, "b.cube", 17);
        alone.node = Some("n1".into());
        export(&project, dir.path(), &places(dir.path()), &alone).unwrap();
        let a = parse_cube(&fs::read(dir.path().join("a.cube")).unwrap()).unwrap();
        let b = parse_cube(&fs::read(dir.path().join("b.cube")).unwrap()).unwrap();
        assert_eq!(a.cube().unwrap().table(), b.cube().unwrap().table());
        // That node alone cannot be.
        let mut that = request(&project, "c.cube", 17);
        that.node = Some("n2".into());
        let error = export(&project, dir.path(), &places(dir.path()), &that).unwrap_err();
        assert_eq!(error.data.unwrap()["reason"], "cannot_bake");
    }

    #[test]
    fn a_node_with_a_qualifier_is_left_out_and_said() {
        let dir = tempfile::tempdir().unwrap();
        let mut grade = graded();
        let NodeKind::Corrector(c) = &mut grade.nodes[1].kind else {
            unreachable!()
        };
        c.qualifier = Some(columbina_model::Qualifier::default());
        let project = project_with(grade);
        let answer = export(
            &project,
            dir.path(),
            &places(dir.path()),
            &request(&project, "a.cube", 17),
        )
        .unwrap();
        assert_eq!(
            answer["ignored"],
            json!([{"node": "n2", "why": "qualifier"}])
        );
        assert_eq!(answer["nodes"], json!(["n1"]));
    }

    #[test]
    fn complete_windows_exclude_the_whole_node_even_when_collapsed_or_transparent() {
        use columbina_model::grade::{Window, WindowShape};
        let mut grade = graded();
        let NodeKind::Corrector(c) = &mut grade.nodes[1].kind else {
            unreachable!()
        };
        let mut window = Window::new("w", WindowShape::circular());
        window.transform.size = 0.0.into();
        window.opacity = 0.0.into();
        c.windows.push(window);
        c.primaries.midtone_detail = 40.0.into();
        let (baked, ignored) = without_filters(&grade);
        assert!(!baked.nodes[1].enabled);
        assert_eq!(ignored, vec![json!({"node":"n2","why":"windows"})]);
        assert_eq!(
            alone(&grade, "n2").unwrap_err().data.unwrap()["why"],
            "windows"
        );
        let NodeKind::Corrector(c) = &mut grade.nodes[1].kind else {
            unreachable!()
        };
        c.qualifier = Some(columbina_model::Qualifier::default());
        let (baked, ignored) = without_filters(&grade);
        assert!(
            !baked.nodes[1].enabled,
            "all corrections in the node are excluded"
        );
        assert!(baked.nodes[0].enabled);
        assert_eq!(ignored, vec![json!({"node":"n2","why":"qualifier"})]);
        assert_eq!(
            alone(&grade, "n2").unwrap_err().data.unwrap()["why"],
            "qualifier"
        );
        let NodeKind::Corrector(c) = &mut grade.nodes[1].kind else {
            unreachable!()
        };
        c.qualifier = None;
        c.primaries.midtone_detail = 0.0.into();
        c.windows[0].shape = WindowShape::curve();
        c.windows[0].invert = true;
        assert!(without_filters(&grade).1.is_empty());
    }

    #[test]
    fn a_lut_of_a_node_is_baked_and_one_missing_refuses() {
        let dir = tempfile::tempdir().unwrap();
        // A cube that swaps red and blue, of 2 points.
        fs::write(
            dir.path().join("swap.cube"),
            "LUT_3D_SIZE 2\n0 0 0\n0 0 1\n0 1 0\n0 1 1\n1 0 0\n1 0 1\n1 1 0\n1 1 1\n",
        )
        .unwrap();
        let project = project_with(serial(vec![corrector("n1", |c| {
            c.lut = Some("swap.cube".into());
        })]));
        export(
            &project,
            dir.path(),
            &places(dir.path()),
            &request(&project, "out.cube", 17),
        )
        .unwrap();
        let out = parse_cube(&fs::read(dir.path().join("out.cube")).unwrap()).unwrap();
        let read = out.eval([0.2, 0.5, 0.9], Interpolation::Trilinear);
        for (c, wanted) in [0.9, 0.5, 0.2].into_iter().enumerate() {
            assert!((read[c] - wanted).abs() < 1e-5, "{read:?}");
        }
        let project = project_with(serial(vec![corrector("n1", |c| {
            c.lut = Some("gone.cube".into());
        })]));
        let error = export(
            &project,
            dir.path(),
            &places(dir.path()),
            &request(&project, "x.cube", 17),
        )
        .unwrap_err();
        assert_eq!(error.code, rpc::REFUSED);
        let data = error.data.unwrap();
        assert_eq!(data["reason"], "missing_luts");
        assert_eq!(data["missing_luts"][0]["name"], "gone.cube");
        assert!(!dir.path().join("x.cube").exists());
    }

    #[test]
    fn a_cube_is_written_only_where_it_may_be() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("cache")).unwrap();
        fs::create_dir(dir.path().join("luts")).unwrap();
        let mut project = project_with(graded());
        let media = outside.path().join("film.cube");
        fs::write(&media, "not a film").unwrap();
        project.media[0].path = media.to_string_lossy().into_owned();
        let try_at = |path: &str, replace: bool| {
            let mut asked = request(&project, path, 17);
            asked.replace = replace;
            export(&project, dir.path(), &places(dir.path()), &asked)
        };
        let reason = |result: Result<Value, Fault>| {
            result.unwrap_err().data.unwrap()["reason"]
                .as_str()
                .unwrap()
                .to_owned()
        };
        for path in [
            "../up.cube",
            "luts/../../up.cube",
            "cache/a.cube",
            "Cache/a.cube",
            "a.txt",
            "nowhere/a.cube",
            "C:a.cube",
            ".cube",
        ] {
            assert_eq!(reason(try_at(path, false)), "path", "{path}");
        }
        let in_cache = dir.path().join("cache").join("a.cube");
        assert_eq!(reason(try_at(in_cache.to_str().unwrap(), false)), "path");
        let lost = outside.path().join("no").join("a.cube");
        assert_eq!(reason(try_at(lost.to_str().unwrap(), false)), "path");
        assert_eq!(reason(try_at(media.to_str().unwrap(), true)), "source");
        // Into a folder of the project, and into one that the user chose.
        assert_eq!(
            try_at("luts/look.cube", false).unwrap()["name"],
            "luts/look.cube"
        );
        assert_eq!(reason(try_at("luts/look.cube", false)), "exists");
        try_at("luts/look.cube", true).unwrap();
        let chosen = outside.path().join("chosen.cube");
        let answer = try_at(chosen.to_str().unwrap(), false).unwrap();
        assert_eq!(answer["name"], chosen.to_string_lossy().as_ref());
        assert!(parse_cube(&fs::read(&chosen).unwrap()).is_ok());
        // A folder is not a file to replace; nothing is left beside it.
        fs::create_dir(dir.path().join("dir.cube")).unwrap();
        assert_eq!(reason(try_at("dir.cube", true)), "path");
        let left: Vec<_> = fs::read_dir(dir.path().join("luts"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(left, vec![std::ffi::OsString::from("look.cube")]);
        // Not through a junction (or a link) out of the project.
        #[cfg(windows)]
        {
            let junction = dir.path().join("out");
            let made = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(&junction)
                .arg(outside.path())
                .output()
                .unwrap();
            assert!(made.status.success());
            assert_eq!(reason(try_at("out/a.cube", false)), "path");
        }
    }

    #[test]
    fn the_size_must_be_one_of_generate_luts() {
        for (size, ok) in [(17, true), (33, true), (65, true), (16, false), (64, false)] {
            let params = serde_json::value::RawValue::from_string(format!(
                r#"{{"grade":{{"level":"timeline","timeline":"t1"}},"size":{size},"path":"a.cube"}}"#
            ))
            .unwrap();
            assert_eq!(Request::read(Some(&params)).is_ok(), ok, "{size}");
        }
    }
}
