//! Window presets: windows saved to be recalled on any node of any project (Reference Manual of
//! `DaVinci Resolve` 20, ch. 138, "Saving Window Presets", pp. 3198–3199).
//!
//! # The library
//!
//! The presets are this user's, shared by every project as the `PowerGrades` are (plan, Q10):
//! `%COLUMBINA_HOME%\window_presets`, else `%USERPROFILE%\.columbina\window_presets`, or the
//! folder that `COLUMBINA_WINDOW_PRESETS` names. No project names a preset, and none is undone
//! with a project: as the manual says of Delete Preset, "you cannot undo the deletion".
//!
//! ```text
//! <root>/presets.json   {"format": 1, "next": 3, "presets": [{"id": "wp1", "name": "Face", "windows": [...]}]}
//! <root>/.lock          held while the library is read or changed (a folder)
//! ```
//!
//! The index is written whole (to a file beside it, then put in its place), one change at a time
//! of this engine and of others that share it; one of a later format is refused. The presets are
//! in the order they were saved; their ids are `wp<n>`, never given twice.
//!
//! # What a preset holds
//!
//! Save as New Preset and Update Preset keep every window of the node, each as it is on the frame
//! of the playhead: its shape and parameters interpolated there, with no keyframes and no tracking
//! data (author decision 2026-11-01). Recalling a preset "overwrite\[s\] whatever other windows were
//! set up in that node" (p. 3199): it is worked out into a `set_node` of the node's whole window
//! list, one transaction and one undo step, which writes the windows into the project in full.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use columbina_model::grade::{NodeKind, Target, Window};
use columbina_model::Project;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use serde_json::{json, Value};

use crate::log::log;
use crate::rpc::{self, Fault};
use crate::session::{self, Connection, Engine, Reply};

/// The format of `presets.json`.
pub const FORMAT: u32 = 1;
/// The variable that names the folder of the window presets.
pub const VARIABLE: &str = "COLUMBINA_WINDOW_PRESETS";
/// The longest name of a preset, in characters.
const LONGEST_NAME: usize = 100;
/// How long a lock that is not let go is waited for before it is taken as left behind.
const STALE_LOCK: Duration = Duration::from_secs(30);
/// How long a lock is waited for at most.
const LOCK_WITHIN: Duration = Duration::from_secs(5);

/// One change of the library at a time in this engine; other engines wait on the lock folder.
static CHANGING: Mutex<()> = Mutex::new(());

/// A preset: its name and its windows, every parameter of them fixed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preset {
    pub id: String,
    pub name: String,
    pub windows: Vec<Window>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Library {
    format: u32,
    /// The number of the next id.
    next: u64,
    presets: Vec<Preset>,
}

/// The folder of the library, where there is a home to put it in.
pub fn folder() -> Option<PathBuf> {
    std::env::var_os(VARIABLE)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| crate::gallery::columbina_home().map(|home| home.join("window_presets")))
}

/// A fault of the folder of the library (`-32002`, `reason` `window_presets`).
fn library_fault(message: &str, path: &Path) -> Fault {
    Fault::new(rpc::CANNOT_READ, message.to_string()).with_data(json!({
        "reason": "window_presets",
        "path": path.to_string_lossy(),
        "message": message,
    }))
}

/// A preset that is not there (`-32602`, `reason` `missing`).
fn missing(id: &str) -> Fault {
    Fault::params(format!("there is no window preset {id}"))
        .with_data(json!({"reason": "missing", "what": "preset", "id": id}))
}

/// A refusal of a change (`-32001`).
fn refused(reason: &str, message: &str, fields: Value) -> Fault {
    let mut data = json!({"reason": reason, "message": message});
    if let (Some(data), Value::Object(fields)) = (data.as_object_mut(), fields) {
        data.extend(fields);
    }
    Fault::new(rpc::REFUSED, message.to_string()).with_data(data)
}

/// A name of 1 to 100 characters, not all spaces; it is kept as it was given, trimmed.
fn checked_name(name: &str) -> Result<String, Fault> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > LONGEST_NAME || name.chars().any(char::is_control)
    {
        return Err(Fault::params(format!(
            "a name of a window preset is of 1 to {LONGEST_NAME} characters"
        ))
        .with_data(json!({"reason": "out_of_range", "what": "name"})));
    }
    Ok(name.to_string())
}

/// The lock folder of the library, taken away when it is dropped.
struct Lock(PathBuf);

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_dir(&self.0);
    }
}

fn take_lock(root: &Path) -> Result<Lock, Fault> {
    let path = root.join(".lock");
    let start = Instant::now();
    loop {
        match fs::create_dir(&path) {
            Ok(()) => return Ok(Lock(path)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let stale = fs::symlink_metadata(&path)
                    .and_then(|meta| meta.modified())
                    .ok()
                    .and_then(|modified| modified.elapsed().ok())
                    .is_some_and(|age| age > STALE_LOCK);
                if stale {
                    log(
                        "window_presets_lock_stale",
                        &json!({"path": path.to_string_lossy()}),
                    );
                    let _ = fs::remove_dir(&path);
                    continue;
                }
                if start.elapsed() > LOCK_WITHIN {
                    return Err(library_fault(
                        &format!(
                            "the window presets {} are held by another engine",
                            root.display()
                        ),
                        root,
                    ));
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(error) => {
                return Err(library_fault(
                    &format!("cannot lock the window presets {}: {error}", root.display()),
                    root,
                ))
            }
        }
    }
}

/// The library opened, read while its lock is held.
struct Opened {
    file: PathBuf,
    library: Library,
    _lock: Lock,
}

impl Opened {
    fn open(root: &Path) -> Result<Opened, Fault> {
        if let Err(error) = fs::create_dir_all(root) {
            return Err(library_fault(
                &format!("cannot make the folder {}: {error}", root.display()),
                root,
            ));
        }
        match fs::symlink_metadata(root) {
            Ok(meta) if meta.is_dir() && !meta.is_symlink() => {}
            _ => {
                return Err(library_fault(
                    &format!(
                        "{} is not a folder of the engine's own (a link or a junction?)",
                        root.display()
                    ),
                    root,
                ))
            }
        }
        let lock = take_lock(root)?;
        let file = root.join("presets.json");
        let library = match fs::read(&file) {
            Ok(bytes) => read_library(&bytes, &file)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Library {
                format: FORMAT,
                next: 1,
                presets: vec![],
            },
            Err(error) => {
                return Err(library_fault(
                    &format!("cannot read {}: {error}", file.display()),
                    &file,
                ))
            }
        };
        Ok(Opened {
            file,
            library,
            _lock: lock,
        })
    }

    /// Write the index whole: to a file beside it, then put in its place.
    fn save(&self) -> Result<(), Fault> {
        let bytes = serde_json::to_vec_pretty(&self.library)
            .map_err(|error| library_fault(&error.to_string(), &self.file))?;
        let part = self.file.with_extension("json.part");
        fs::write(&part, bytes)
            .and_then(|()| fs::rename(&part, &self.file))
            .map_err(|error| {
                let _ = fs::remove_file(&part);
                library_fault(
                    &format!("cannot write {}: {error}", self.file.display()),
                    &self.file,
                )
            })
    }

    fn at(&self, id: &str) -> Result<usize, Fault> {
        self.library
            .presets
            .iter()
            .position(|preset| preset.id == id)
            .ok_or_else(|| missing(id))
    }
}

fn read_library(bytes: &[u8], path: &Path) -> Result<Library, Fault> {
    let not_one = || {
        library_fault(
            &format!("{} is not an index of window presets", path.display()),
            path,
        )
    };
    let value: Value = serde_json::from_slice(bytes).map_err(|_| not_one())?;
    let format = value["format"].as_u64().ok_or_else(not_one)?;
    if format != u64::from(FORMAT) {
        return Err(library_fault(
            &format!(
                "{} is of format {format}, and this engine reads format {FORMAT}",
                path.display()
            ),
            path,
        ));
    }
    serde_json::from_value(value).map_err(|_| not_one())
}

/// Run `work` with the library in `root` open, one change at a time in this engine.
fn with<T>(root: &Path, work: impl FnOnce(&mut Opened) -> Result<T, Fault>) -> Result<T, Fault> {
    let _changing = CHANGING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut opened = Opened::open(root)?;
    work(&mut opened)
}

/// The presets, in the order they were saved.
pub fn list(root: &Path) -> Result<Vec<Preset>, Fault> {
    with(root, |opened| Ok(opened.library.presets.clone()))
}

/// The preset `id`.
pub fn get(root: &Path, id: &str) -> Result<Preset, Fault> {
    with(root, |opened| {
        let at = opened.at(id)?;
        Ok(opened.library.presets[at].clone())
    })
}

/// Save as New Preset: `windows` under `name`, after the others. A name that a preset has
/// already is refused (`name_taken`), so that each in the option menu is known by its name.
pub fn save_new(root: &Path, name: &str, windows: Vec<Window>) -> Result<Preset, Fault> {
    let name = checked_name(name)?;
    with(root, |opened| {
        if opened
            .library
            .presets
            .iter()
            .any(|preset| preset.name == name)
        {
            return Err(refused(
                "name_taken",
                &format!("there is a window preset named {name} already"),
                json!({"name": name}),
            ));
        }
        let preset = Preset {
            id: format!("wp{}", opened.library.next),
            name,
            windows,
        };
        opened.library.next += 1;
        opened.library.presets.push(preset.clone());
        opened.save()?;
        Ok(preset)
    })
}

/// Update Preset: the windows of the preset `id` become `windows`; its name and place stay.
pub fn update(root: &Path, id: &str, windows: Vec<Window>) -> Result<Preset, Fault> {
    with(root, |opened| {
        let at = opened.at(id)?;
        opened.library.presets[at].windows = windows;
        opened.save()?;
        Ok(opened.library.presets[at].clone())
    })
}

/// Delete Preset: at once, with no warning, and not undone (p. 3199).
pub fn delete(root: &Path, id: &str) -> Result<(), Fault> {
    with(root, |opened| {
        let at = opened.at(id)?;
        opened.library.presets.remove(at);
        opened.save()
    })
}

/// The windows of the corrector `node` of `target`, each fixed as it is on `frame` of the
/// grade's keys; refused where there is no such corrector, or it has no window.
#[allow(
    clippy::cast_precision_loss,
    reason = "the model evaluates keyframes as f64"
)]
pub fn frozen(
    project: &Project,
    target: &Target,
    node: &str,
    frame: i64,
) -> Result<Vec<Window>, Fault> {
    let corrector = corrector(project, target, node)?;
    if corrector.is_empty() {
        return Err(refused(
            "no_windows",
            &format!("the node {node} has no window to save"),
            json!({"node": node}),
        ));
    }
    Ok(corrector
        .iter()
        .map(|window| window.frozen_at(frame as f64))
        .collect())
}

/// The windows of the corrector `node` of `target`.
fn corrector<'a>(project: &'a Project, target: &Target, node: &str) -> Result<&'a [Window], Fault> {
    let grade = crate::stills::grade_of(project, target)?;
    let found = grade.node(node).ok_or_else(|| {
        refused(
            "missing",
            &format!("there is no node {node}"),
            json!({"what": "node", "id": node}),
        )
    })?;
    match &found.kind {
        NodeKind::Corrector(c) => Ok(&c.windows),
        _ => Err(refused(
            "wrong_node",
            &format!("the node {node} is not a corrector, which has windows"),
            json!({"node": node}),
        )),
    }
}

/// The change of recalling a preset: the node's whole window list becomes the preset's.
fn recall(
    project: &Project,
    target: &Target,
    node: &str,
    preset: &Preset,
) -> Result<Vec<Value>, Fault> {
    corrector(project, target, node)?;
    Ok(vec![json!({
        "op": "set_node", "grade": target, "id": node,
        "params": {"windows": preset.windows},
    })])
}

/// Whether `method` is one of this module's.
pub fn is_method(method: &str) -> bool {
    method.starts_with("window_presets.")
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Save {
    name: String,
    grade: Target,
    node: String,
    frame: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Update {
    preset: String,
    grade: Target,
    node: String,
    frame: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Apply {
    preset: String,
    grade: Target,
    node: String,
    revision: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Named {
    preset: String,
}

/// Run `work` on a thread of its own, where it may read and write files.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, Fault> + Send + 'static,
) -> Result<T, Fault> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| Fault::new(rpc::CANNOT_READ, error.to_string()))?
}

/// `window_presets.list`, `.save`, `.update`, `.delete` and `.apply`.
pub async fn call(
    engine: &Engine,
    connection: &Connection,
    kind: session::Kind,
    id: &RawValue,
    method: &str,
    params: Option<&RawValue>,
) -> Result<Reply, Reply> {
    let root = folder().ok_or_else(|| {
        Fault::new(
            rpc::CANNOT_READ,
            "there is no folder for window presets: set COLUMBINA_HOME or COLUMBINA_WINDOW_PRESETS",
        )
        .with_data(json!({"reason": "window_presets"}))
    })?;
    let result = match method {
        "window_presets.list" => {
            if params.is_some_and(|p| !matches!(p.get().trim(), "{}" | "null")) {
                return Err(Fault::params("window_presets.list takes {}").into());
            }
            let presets = blocking(move || list(&root)).await?;
            return Ok(answer(connection, id, &json!({"presets": presets})));
        }
        "window_presets.save" => {
            let request: Save = rpc::params(
                method,
                r#"{"name": "Face", "grade": {"level": "clip", "timeline": "t1", "clip": "c3"}, "node": "n4", "frame": 120}"#,
                params,
            )?;
            checked_name(&request.name)?;
            let windows = session::read_project(engine, |project| {
                frozen(project, &request.grade, &request.node, request.frame)
            })
            .await?;
            let name = request.name;
            let preset = blocking(move || save_new(&root, &name, windows)).await?;
            json!(preset)
        }
        "window_presets.update" => {
            let request: Update = rpc::params(
                method,
                r#"{"preset": "wp1", "grade": {"level": "clip", "timeline": "t1", "clip": "c3"}, "node": "n4", "frame": 120}"#,
                params,
            )?;
            let windows = session::read_project(engine, |project| {
                frozen(project, &request.grade, &request.node, request.frame)
            })
            .await?;
            let preset = request.preset;
            json!(blocking(move || update(&root, &preset, windows)).await?)
        }
        "window_presets.delete" => {
            let request: Named = rpc::params(method, r#"{"preset": "wp1"}"#, params)?;
            let preset = request.preset.clone();
            blocking(move || delete(&root, &preset)).await?;
            json!({"deleted": request.preset})
        }
        "window_presets.apply" => {
            let request: Apply = rpc::params(
                method,
                r#"{"preset": "wp1", "grade": {"level": "clip", "timeline": "t1", "clip": "c3"}, "node": "n4", "revision"?: 7}"#,
                params,
            )?;
            let name = request.preset.clone();
            let preset = blocking(move || get(&root, &name)).await?;
            return session::apply_worked_out(
                engine,
                connection,
                kind,
                id,
                request.revision,
                |project| recall(project, &request.grade, &request.node, &preset),
            )
            .await;
        }
        _ => {
            return Err(Fault::new(
                rpc::METHOD_NOT_FOUND,
                format!("there is no method {method}"),
            )
            .into())
        }
    };
    session::tell_everyone(engine, "window_presets.changed", &json!({})).await;
    Ok(answer(connection, id, &result))
}

fn answer(connection: &Connection, id: &RawValue, result: &Value) -> Reply {
    match connection.queue(rpc::result_line(id, result)) {
        Ok(()) => Reply::Queued,
        Err(refused) => Reply::Refused(refused),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use columbina_model::grade::{Interp, Key, Node, Param, WindowShape};
    use columbina_model::{apply, Link, Op};

    fn circle(id: &str, size: f64) -> Window {
        let mut window = Window::new(id, WindowShape::circular());
        window.transform.size = Param::Fixed(size);
        window
    }

    fn reason(fault: &Fault) -> Value {
        serde_json::to_value(fault).unwrap()["data"]["reason"].clone()
    }

    #[test]
    fn presets_are_saved_in_order_updated_in_place_and_deleted_for_good() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("window_presets");
        assert_eq!(list(&root).unwrap(), vec![]);
        let face = save_new(&root, "  Face  ", vec![circle("w1", 40.0)]).unwrap();
        assert_eq!((face.id.as_str(), face.name.as_str()), ("wp1", "Face"));
        let pair = save_new(&root, "Pair", vec![circle("w1", 10.0), circle("w2", 20.0)]).unwrap();
        assert_eq!(pair.id, "wp2");
        let taken = save_new(&root, "Face", vec![circle("w1", 1.0)]).unwrap_err();
        assert_eq!(reason(&taken), "name_taken");
        assert_eq!(
            reason(&save_new(&root, " ", vec![]).unwrap_err()),
            "out_of_range"
        );
        assert_eq!(
            reason(&save_new(&root, &"x".repeat(101), vec![]).unwrap_err()),
            "out_of_range"
        );
        let updated = update(&root, "wp1", vec![circle("w3", 70.0)]).unwrap();
        assert_eq!(updated.name, "Face");
        let names: Vec<_> = list(&root)
            .unwrap()
            .into_iter()
            .map(|p| (p.id, p.name))
            .collect();
        assert_eq!(
            names,
            [("wp1".into(), "Face".into()), ("wp2".into(), "Pair".into())]
        );
        assert_eq!(get(&root, "wp1").unwrap().windows, vec![circle("w3", 70.0)]);
        delete(&root, "wp1").unwrap();
        assert_eq!(reason(&get(&root, "wp1").unwrap_err()), "missing");
        assert_eq!(reason(&delete(&root, "wp1").unwrap_err()), "missing");
        // An id is never given twice, even once its preset is gone.
        assert_eq!(save_new(&root, "Face", vec![]).unwrap().id, "wp3");
        let written: Value =
            serde_json::from_slice(&fs::read(root.join("presets.json")).unwrap()).unwrap();
        assert_eq!(written["format"], 1);
        assert_eq!(written["next"], 4);
        assert!(!root.join(".lock").exists());
        assert!(!root.join("presets.json.part").exists());
    }

    #[test]
    fn a_library_of_a_later_format_or_not_one_is_refused_and_left_as_it_is() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let later = r#"{"format": 2, "next": 1, "presets": []}"#;
        fs::write(root.join("presets.json"), later).unwrap();
        let fault = list(&root).unwrap_err();
        assert_eq!(reason(&fault), "window_presets");
        assert!(fault.message.contains("format 2"), "{}", fault.message);
        assert_eq!(
            fs::read_to_string(root.join("presets.json")).unwrap(),
            later
        );
        fs::write(root.join("presets.json"), "not json").unwrap();
        assert_eq!(reason(&list(&root).unwrap_err()), "window_presets");
        assert!(!root.join(".lock").exists());
    }

    /// A project whose timeline grade has one corrector, `n3`, with `windows`.
    fn project_with(windows: Vec<Window>) -> (Project, Target) {
        let mut project = Project::sample();
        let target = Target::Timeline {
            timeline: "t1".into(),
        };
        let mut node = Node::corrector("");
        let NodeKind::Corrector(c) = &mut node.kind else {
            unreachable!()
        };
        c.windows = windows;
        apply(
            &mut project,
            Op::AddNode {
                grade: target.clone(),
                node,
                index: None,
                links: vec![Link::rgb("source", ""), Link::rgb("", "output")],
            },
        )
        .unwrap();
        assert_eq!(project.timelines[0].grade.nodes[0].id, "n3");
        (project, target)
    }

    #[test]
    fn saving_freezes_every_window_at_the_frame_and_refuses_what_has_none() {
        let mut keyed = circle("w2", 50.0);
        keyed.transform.size = Param::Keys(vec![
            Key {
                frame: 0,
                value: 20.0,
                interp: Interp::Linear,
            },
            Key {
                frame: 10,
                value: 40.0,
                interp: Interp::Linear,
            },
        ]);
        let (project, target) = project_with(vec![circle("w1", 30.0), keyed]);
        let saved = frozen(&project, &target, "n3", 5).unwrap();
        assert_eq!(saved, vec![circle("w1", 30.0), circle("w2", 30.0)]);
        let fault = frozen(&project, &target, "n99", 5).unwrap_err();
        assert_eq!(reason(&fault), "missing");
        let (empty, target) = project_with(vec![]);
        assert_eq!(
            reason(&frozen(&empty, &target, "n3", 0).unwrap_err()),
            "no_windows"
        );
    }

    #[test]
    fn recalling_replaces_the_whole_window_list_of_the_node() {
        let (project, target) = project_with(vec![circle("w1", 30.0), circle("w2", 60.0)]);
        let preset = Preset {
            id: "wp1".into(),
            name: "Face".into(),
            windows: vec![circle("w1", 80.0)],
        };
        let ops = recall(&project, &target, "n3", &preset).unwrap();
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0]["op"], "set_node");
        assert_eq!(ops[0]["id"], "n3");
        assert_eq!(
            ops[0]["params"]["windows"],
            serde_json::to_value(&preset.windows).unwrap()
        );
        assert_eq!(
            reason(&recall(&project, &target, "n7", &preset).unwrap_err()),
            "missing"
        );
    }
}
