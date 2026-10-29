//! Staged output files, with rollback if publishing a movie and its sidecars fails.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::json;
use tempfile::TempPath;

use crate::rpc::{self, Fault};

pub(super) fn check(path: &Path, project: &Path, replace: bool) -> Result<(), Fault> {
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
        let mut partial = path.as_os_str().to_owned();
        partial.push(".part");
        let partial = PathBuf::from(partial);
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
