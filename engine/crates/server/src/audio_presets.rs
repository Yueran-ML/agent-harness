//! User-level audio parameters, independent of project and window presets. Source references are excluded.
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use columbina_model::audio::fx::{Dynamics, Effect, Equalizer, Sidechain};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use serde_json::{json, Value};

use crate::log::log;
use crate::rpc::{self, Fault};
use crate::session::{self, Connection, Engine, Reply};

/// The format of `presets.json`.
pub const FORMAT: u32 = 1;
/// The variable that names the folder of the audio presets.
pub const VARIABLE: &str = "COLUMBINA_AUDIO_PRESETS";
/// The longest name of a preset, in characters.
const LONGEST_NAME: usize = 100;
/// How long a lock that is not let go is waited for before it is taken as left behind.
const STALE_LOCK: Duration = Duration::from_secs(30);
/// How long a lock is waited for at most.
const LOCK_WITHIN: Duration = Duration::from_secs(5);

/// One change of the library at a time in this engine; other engines wait on the lock folder.
static CHANGING: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "settings",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Payload {
    Eq(Equalizer),
    ClipEq(Equalizer),
    Dynamics(Dynamics),
    Effect(Effect),
}

fn checked_payload(mut payload: Payload) -> Result<Payload, Fault> {
    let result = match &mut payload {
        Payload::Eq(eq) => eq.check(true),
        Payload::ClipEq(eq) => eq.check(false),
        Payload::Dynamics(dynamics) => {
            dynamics.compressor.sidechain = Sidechain::default();
            dynamics.check()
        }
        Payload::Effect(effect) => {
            if let Effect::Limiter(limiter) = effect {
                limiter.sidechain = Sidechain::default();
            }
            effect.check()
        }
    };
    result.map_err(|error| Fault::params(format!("invalid audio preset: {error}")))?;
    Ok(payload)
}

/// A preset: its name and validated audio parameter payload.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preset {
    pub id: String,
    pub name: String,
    pub payload: Payload,
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
        .or_else(|| crate::gallery::columbina_home().map(|home| home.join("audio_presets")))
}

/// A fault of the folder of the library (`-32002`, `reason` `audio_presets`).
fn library_fault(message: &str, path: &Path) -> Fault {
    Fault::new(rpc::CANNOT_READ, message.to_string()).with_data(json!({
        "reason": "audio_presets",
        "path": path.to_string_lossy(),
        "message": message,
    }))
}

/// A preset that is not there (`-32602`, `reason` `missing`).
fn missing(id: &str) -> Fault {
    Fault::params(format!("there is no audio preset {id}"))
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
            "a name of a audio preset is of 1 to {LONGEST_NAME} characters"
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
                        "audio_presets_lock_stale",
                        &json!({"path": path.to_string_lossy()}),
                    );
                    let _ = fs::remove_dir(&path);
                    continue;
                }
                if start.elapsed() > LOCK_WITHIN {
                    return Err(library_fault(
                        &format!(
                            "the audio presets {} are held by another engine",
                            root.display()
                        ),
                        root,
                    ));
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(error) => {
                return Err(library_fault(
                    &format!("cannot lock the audio presets {}: {error}", root.display()),
                    root,
                ));
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
                ));
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
                ));
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
            &format!("{} is not an index of audio presets", path.display()),
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
    let mut library: Library = serde_json::from_value(value).map_err(|_| not_one())?;
    for preset in &mut library.presets {
        checked_name(&preset.name)?;
        preset.payload = checked_payload(preset.payload.clone())?;
    }
    Ok(library)
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

/// Delete a user preset without changing project history.
pub fn delete(root: &Path, id: &str) -> Result<(), Fault> {
    with(root, |opened| {
        let at = opened.at(id)?;
        opened.library.presets.remove(at);
        opened.save()
    })
}

/// The preset `id`.
#[cfg(test)]
pub fn get(root: &Path, id: &str) -> Result<Preset, Fault> {
    with(root, |opened| {
        let at = opened.at(id)?;
        Ok(opened.library.presets[at].clone())
    })
}

/// Save as New Preset: audio `payload` under `name`, after the others. A name that a preset has
/// already is refused (`name_taken`), so that each in the option menu is known by its name.
pub fn save_new(root: &Path, name: &str, payload: Payload) -> Result<Preset, Fault> {
    let name = checked_name(name)?;
    let payload = checked_payload(payload)?;
    with(root, |opened| {
        if opened
            .library
            .presets
            .iter()
            .any(|preset| preset.name == name)
        {
            return Err(refused(
                "name_taken",
                &format!("there is a audio preset named {name} already"),
                json!({"name": name}),
            ));
        }
        let preset = Preset {
            id: format!("ap{}", opened.library.next),
            name,
            payload,
        };
        opened.library.next += 1;
        opened.library.presets.push(preset.clone());
        opened.save()?;
        Ok(preset)
    })
}

/// Update Preset: the audio settings of preset `id` become `payload`; its name and place stay.
pub fn update(root: &Path, id: &str, payload: Payload) -> Result<Preset, Fault> {
    with(root, |opened| {
        let at = opened.at(id)?;
        if payload_kind(&opened.library.presets[at].payload) != payload_kind(&payload) {
            return Err(Fault::params(
                "an audio preset update must keep its processor kind",
            ));
        }
        opened.library.presets[at].payload = checked_payload(payload)?;
        opened.save()?;
        Ok(opened.library.presets[at].clone())
    })
}

fn payload_kind(payload: &Payload) -> &str {
    match payload {
        Payload::Eq(_) => "eq",
        Payload::ClipEq(_) => "clip_eq",
        Payload::Dynamics(_) => "dynamics",
        Payload::Effect(effect) => effect.name(),
    }
}

pub fn is_method(method: &str) -> bool {
    method.starts_with("audio_presets.")
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Save {
    name: String,
    payload: Payload,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Update {
    preset: String,
    payload: Payload,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Named {
    preset: String,
}

pub async fn call(
    engine: &Engine,
    connection: &Connection,
    id: &RawValue,
    method: &str,
    params: Option<&RawValue>,
) -> Result<Reply, Reply> {
    let root = folder()
        .ok_or_else(|| Fault::new(rpc::CANNOT_READ, "there is no folder for audio presets"))?;
    let result = match method {
        "audio_presets.list" => {
            if params.is_some_and(|p| !matches!(p.get().trim(), "{}" | "null")) {
                return Err(Fault::params("audio_presets.list takes {}").into());
            }
            let presets = blocking(move || list(&root)).await?;
            return Ok(answer(connection, id, &json!({"presets": presets})));
        }
        "audio_presets.save" => {
            let request: Save = rpc::params(
                method,
                r#"{"name":"Voice","payload":{"kind":"effect","settings":{"kind":"gain","gain_db":-3}}}"#,
                params,
            )?;
            json!(blocking(move || save_new(&root, &request.name, request.payload)).await?)
        }
        "audio_presets.update" => {
            let request: Update = rpc::params(
                method,
                r#"{"preset":"ap1","payload":{"kind":"effect","settings":{"kind":"gain","gain_db":-2}}}"#,
                params,
            )?;
            json!(blocking(move || update(&root, &request.preset, request.payload)).await?)
        }
        "audio_presets.delete" => {
            let request: Named = rpc::params(method, r#"{"preset":"ap1"}"#, params)?;
            let preset = request.preset.clone();
            blocking(move || delete(&root, &preset)).await?;
            json!({"deleted":request.preset})
        }
        _ => {
            return Err(Fault::new(
                rpc::METHOD_NOT_FOUND,
                format!("there is no method {method}"),
            )
            .into());
        }
    };
    session::tell_everyone(engine, "audio_presets.changed", &json!({})).await;
    Ok(answer(connection, id, &result))
}
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, Fault> + Send + 'static,
) -> Result<T, Fault> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| Fault::new(rpc::CANNOT_READ, error.to_string()))?
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
    use columbina_model::audio::fx::{GainFx, LimiterFx};
    #[test]
    fn persistent_library_validates_kinds_and_excludes_project_sources() {
        let root = tempfile::tempdir().unwrap();
        let mut dynamics = Dynamics::default();
        dynamics.compressor.sidechain.source =
            Some(columbina_model::audio::SourceRef::Track("a1".into()));
        dynamics.compressor.sidechain.on = true;
        let saved = save_new(root.path(), "Speech", Payload::Dynamics(dynamics)).unwrap();
        let Payload::Dynamics(read) = get(root.path(), &saved.id).unwrap().payload else {
            panic!("kind")
        };
        assert!(read.compressor.sidechain.source.is_none());
        assert!(!read.compressor.sidechain.on);
        assert!(update(
            root.path(),
            &saved.id,
            Payload::Effect(Effect::Gain(GainFx::default()))
        )
        .is_err());
        let gain = save_new(
            root.path(),
            "Gain",
            Payload::Effect(Effect::Gain(GainFx { gain_db: -3.0 })),
        )
        .unwrap();
        update(
            root.path(),
            &gain.id,
            Payload::Effect(Effect::Gain(GainFx { gain_db: -6.0 })),
        )
        .unwrap();
        assert_eq!(list(root.path()).unwrap().len(), 2);
        assert!(save_new(
            root.path(),
            "Invalid",
            Payload::Effect(Effect::Limiter(LimiterFx {
                ceiling_db: 2.0,
                ..Default::default()
            }))
        )
        .is_err());
        assert_eq!(list(root.path()).unwrap().len(), 2);
        delete(root.path(), &saved.id).unwrap();
        assert_eq!(list(root.path()).unwrap().len(), 1);
        assert!(!root.path().join(".lock").exists());
    }
}
