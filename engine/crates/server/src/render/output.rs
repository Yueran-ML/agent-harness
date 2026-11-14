//! Staged output files, with rollback if publishing a movie and its sidecars fails.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::json;
use tempfile::TempPath;

use crate::rpc::{self, Fault};

/// The partial file that an output is written to until it is published: `<path>.part`.
pub(super) fn part_of(path: &Path) -> PathBuf {
    let mut partial = path.as_os_str().to_owned();
    partial.push(".part");
    PathBuf::from(partial)
}

/// A path as the file system names it, to tell whether two paths are one file: the canonical
/// path of the file, or of its directory with its name where it is not there yet; and on
/// Windows, whose file systems ignore case, in lower case.
fn identity(path: &Path) -> PathBuf {
    let whole = fs::canonicalize(path).ok().or_else(|| {
        let parent = fs::canonicalize(path.parent()?).ok()?;
        Some(parent.join(path.file_name()?))
    });
    let path = whole.unwrap_or_else(|| path.to_owned());
    if cfg!(windows) {
        PathBuf::from(path.to_string_lossy().to_lowercase())
    } else {
        path
    }
}

/// Refuse an output that is a file the export reads: the media of the snapshot, or a proxy of
/// it. Writing over a source while it is decoded would lose the source and the export alike.
pub(super) fn not_a_source(path: &Path, project: &columbina_model::Project) -> Result<(), Fault> {
    let output = identity(path);
    for media in &project.media {
        let proxy = media.proxy.as_ref().map(|proxy| Path::new(&proxy.path));
        for source in std::iter::once(Path::new(&media.path)).chain(proxy) {
            if identity(source) == output {
                return Err(Fault::new(
                    rpc::REFUSED,
                    format!(
                        "the export would write over {}, which media {} reads",
                        path.display(),
                        media.id
                    ),
                )
                .with_data(json!({"reason":"source", "path":path, "media":media.id})));
            }
        }
    }
    Ok(())
}

/// Check an output path before anything is written. A partial file in its way that `replace`
/// lets the export take away is returned, to be taken away with [`remove_parts`] once every
/// check of the export has passed: a refused export changes no file.
pub(super) fn check(path: &Path, project: &Path, replace: bool) -> Result<Option<PathBuf>, Fault> {
    if !path.is_absolute() || path.file_name().is_none() {
        return Err(Fault::params("the export path must name an absolute file"));
    }
    let parent = path.parent().expect("absolute file");
    let parent = fs::canonicalize(parent)
        .map_err(|_| Fault::params("the export directory must already exist"))?;
    if !parent.is_dir() {
        return Err(Fault::params("the export parent is not a directory"));
    }
    let cache = project.join("cache");
    if fs::canonicalize(&cache).is_ok_and(|cache| parent.starts_with(cache)) {
        return Err(Fault::params(
            "exports cannot be written inside the project cache",
        ));
    }
    match fs::symlink_metadata(path) {
        Ok(meta) if !meta.file_type().is_file() => {
            return Err(Fault::params("the export path is not a regular file"))
        }
        Ok(_) if !replace => {
            return Err(Fault::new(rpc::REFUSED, "the export file already exists")
                .with_data(json!({"reason":"exists", "path":path})))
        }
        Err(error) if error.kind() != io::ErrorKind::NotFound => {
            return Err(Fault::params(error.to_string()))
        }
        _ => {}
    }
    stale_part(path, replace)
}

fn part_in_the_way(part: &Path) -> Fault {
    Fault::new(
        rpc::REFUSED,
        format!("a partial file {} is in the way", part.display()),
    )
    .with_data(json!({"reason":"part_exists", "path":part}))
}

/// A partial file at `<path>.part`, left by an export that did not finish (the engine ended
/// while it ran) or by another program, is never written over silently. Without `replace` the
/// export is refused as `part_exists`; with it, the partial file is returned, to be taken away
/// once the export has passed every check, unless it is not a file.
fn stale_part(path: &Path, replace: bool) -> Result<Option<PathBuf>, Fault> {
    let part = part_of(path);
    match fs::symlink_metadata(&part) {
        Ok(meta) if replace && meta.file_type().is_file() => Ok(Some(part)),
        Ok(_) => Err(part_in_the_way(&part)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(Fault::params(error.to_string())),
    }
}

/// Take away the partial files that [`check`] found in the way of an export with `replace`,
/// once the export has passed every check. One that a program still has open (Windows refuses
/// to delete it then) refuses the export as `part_exists`.
pub(super) fn remove_parts(parts: &[PathBuf]) -> Result<(), Fault> {
    for part in parts {
        match fs::remove_file(part) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(_) => return Err(part_in_the_way(part)),
        }
    }
    Ok(())
}

pub(super) struct Files {
    files: Vec<File>,
    replace: bool,
}

struct File {
    path: PathBuf,
    partial: Option<TempPath>,
    previous: Option<TempPath>,
    published: bool,
}

impl Files {
    pub(super) fn new(replace: bool) -> Self {
        Self {
            files: Vec::new(),
            replace,
        }
    }

    pub(super) fn add(&mut self, path: &Path) -> io::Result<PathBuf> {
        let partial = part_of(path);
        // A leftover or another writer's partial is never silently truncated or deleted.
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&partial)?;
        let owned = TempPath::try_from_path(&partial)?;
        self.files.push(File {
            path: path.to_owned(),
            partial: Some(owned),
            previous: None,
            published: false,
        });
        Ok(partial)
    }

    pub(super) fn publish(mut self) -> io::Result<()> {
        // Sidecars first, the movie last. All previous files remain recoverable until all
        // renames succeed; failure restores them and removes only this job's new files.
        for file in self.files.iter_mut().rev() {
            if self.replace && file.path.exists() {
                if !fs::symlink_metadata(&file.path)?.file_type().is_file() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "the export destination is no longer a regular file",
                    ));
                }
                let backup = tempfile::Builder::new()
                    .prefix(".columbina-export-previous-")
                    .tempfile_in(file.path.parent().expect("absolute output"))?
                    .into_temp_path();
                fs::rename(&file.path, &backup)?;
                file.previous = Some(backup);
            }
            file.partial
                .take()
                .expect("not published")
                .persist_noclobber(&file.path)
                .map_err(|error| error.error)?;
            file.published = true;
        }
        for file in &mut self.files {
            file.published = false;
            file.previous = None;
        }
        Ok(())
    }
}

impl Drop for File {
    fn drop(&mut self) {
        if self.published {
            let _ = fs::remove_file(&self.path);
        }
        if let Some(previous) = self.previous.take() {
            if let Err(error) = fs::rename(&previous, &self.path) {
                // A restore failure must leave the recoverable old file on disk.
                let saved = previous.keep().ok();
                crate::log::log(
                    "render_restore_failed",
                    &json!({"path":self.path,"backup":saved,"message":error.to_string()}),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_preserves_old_outputs_and_foreign_partial_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("movie.mov");
        fs::write(&path, b"old movie").unwrap();
        let part;
        {
            let mut files = Files::new(true);
            part = files.add(&path).unwrap();
            fs::write(&part, b"unfinished movie").unwrap();
        }
        assert!(!part.exists());
        assert_eq!(fs::read(&path).unwrap(), b"old movie");
        fs::write(&part, b"another writer").unwrap();
        assert!(Files::new(true).add(&path).is_err());
        assert_eq!(fs::read(&part).unwrap(), b"another writer");
    }

    #[test]
    fn publishing_replaces_a_complete_set_or_rolls_back_every_earlier_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("movie.mov");
        let srt = path.with_extension("srt");
        fs::write(&srt, b"old subtitles").unwrap();
        let mut files = Files::new(true);
        fs::write(files.add(&path).unwrap(), b"new movie").unwrap();
        fs::write(files.add(&srt).unwrap(), b"new subtitles").unwrap();
        // Simulate the destination becoming unusable after validation and staging.
        fs::create_dir(&path).unwrap();
        fs::write(path.join("unrelated"), b"keep").unwrap();
        assert!(files.publish().is_err());
        assert_eq!(fs::read(&srt).unwrap(), b"old subtitles");
        assert_eq!(fs::read(path.join("unrelated")).unwrap(), b"keep");
        assert!(!dir.path().join("movie.mov.part").exists());
        assert!(!dir.path().join("movie.srt.part").exists());

        let movie = dir.path().join("good.mov");
        fs::write(&movie, b"old movie").unwrap();
        let mut files = Files::new(true);
        fs::write(files.add(&movie).unwrap(), b"new movie").unwrap();
        fs::write(files.add(&srt).unwrap(), b"new subtitles").unwrap();
        files.publish().unwrap();
        assert_eq!(fs::read(&movie).unwrap(), b"new movie");
        assert_eq!(fs::read(&srt).unwrap(), b"new subtitles");
        assert!(!fs::read_dir(dir.path()).unwrap().any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".columbina-export-previous-")));
    }

    #[test]
    fn a_stale_partial_file_is_refused_by_name_or_replaced_when_asked() {
        let dir = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let path = dir.path().join("movie.mov");
        let part = dir.path().join("movie.mov.part");
        fs::write(&part, b"left by a crash").unwrap();
        let error = check(&path, project.path(), false).unwrap_err();
        assert_eq!(error.code, rpc::REFUSED);
        assert_eq!(
            error.data,
            Some(json!({"reason":"part_exists", "path":part}))
        );
        assert_eq!(fs::read(&part).unwrap(), b"left by a crash");
        // With replace it is taken away only when asked to, after every other check.
        let found = check(&path, project.path(), true).unwrap();
        assert_eq!(found, Some(part.clone()));
        assert!(part.exists());
        remove_parts(std::slice::from_ref(&part)).unwrap();
        assert!(!part.exists());
        assert_eq!(check(&path, project.path(), true).unwrap(), None);
        let mut files = Files::new(false);
        fs::write(files.add(&path).unwrap(), b"movie").unwrap();
        files.publish().unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"movie");
        // A partial file that a program holds open, as ffmpeg holds what it writes (sharing
        // reading and writing but not deleting), is not taken away.
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            let held = fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .share_mode(1 | 2)
                .open(&part)
                .unwrap();
            let found = check(&path, project.path(), true).unwrap();
            let error = remove_parts(&found.into_iter().collect::<Vec<_>>()).unwrap_err();
            assert_eq!(error.data.unwrap()["reason"], "part_exists");
            drop(held);
            assert!(part.exists());
        }
    }

    #[test]
    fn an_export_over_its_own_media_or_proxy_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let mut project = columbina_model::Project::sample();
        let source = dir.path().join("Source.MP4");
        fs::write(&source, b"media").unwrap();
        project.media[0].path = source.to_string_lossy().into_owned();
        project.media[0].proxy = Some(columbina_model::Proxy {
            path: dir.path().join("proxy.mov").to_string_lossy().into_owned(),
            width: 64,
            height: 36,
            codec: "prores".into(),
            digest_of_source: "0".repeat(64),
        });
        let same = |name: &str| not_a_source(&dir.path().join(name), &project);
        let error = same("Source.MP4").unwrap_err();
        assert_eq!(
            error.data,
            Some(json!({"reason":"source", "path":dir.path().join("Source.MP4"), "media":"m1"}))
        );
        // By another spelling of the same directory, and of the name where case is ignored.
        let dotted = dir.path().join(".").join("Source.MP4");
        assert!(not_a_source(&dotted, &project).is_err());
        assert_eq!(same("source.mp4").is_err(), cfg!(windows));
        assert!(same("proxy.mov").is_err());
        same("other.mp4").unwrap();
    }

    #[test]
    fn a_file_created_after_validation_is_not_clobbered() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("movie.mov");
        let srt = path.with_extension("srt");
        let mut files = Files::new(false);
        fs::write(files.add(&path).unwrap(), b"movie").unwrap();
        fs::write(files.add(&srt).unwrap(), b"subtitles").unwrap();
        fs::write(&path, b"another writer").unwrap();
        assert!(files.publish().is_err());
        assert_eq!(fs::read(&path).unwrap(), b"another writer");
        assert!(!srt.exists());
    }
}
