//! The project of a directory, read as it is opened and written whole or not at all.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use columbina_model::{Project, ReadError};

/// The name of the file that holds the project.
pub const PROJECT_FILE: &str = "project.json";

/// The name of the file that a project is written to before it takes the place of the last one.
pub const PROJECT_TMP: &str = "project.json.tmp";

/// The name of the directory that holds what the engine has found out about the media.
pub const CACHE_DIR: &str = "cache";

/// How many times the written project is put in the place of the last one before the engine
/// gives up, where the system refuses for a while (see [`Store::save`]).
const REPLACE_TRIES: u32 = 8;

/// How long the engine waits before it tries a second time to put the project in place. It waits
/// twice as long before each try after that, so all the tries take a little more than a second.
const REPLACE_FIRST_WAIT: Duration = Duration::from_millis(10);

/// The directory of a project.
///
/// It is opened with [`Store::open`], which reads the project in it, and the project is written
/// back with [`Store::save`]. It holds nothing but where the directory is, so it is cheap to
/// clone.
#[derive(Clone, Debug)]
pub struct Store {
    dir: PathBuf,
}

/// Why a project cannot be opened or saved. Each names the file or directory that it is about.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// The directory of the project, or a directory in it, cannot be made.
    #[error("cannot make the directory {}: {source}", path.display())]
    CreateDir {
        /// The directory.
        path: PathBuf,
        /// What the system said.
        source: io::Error,
    },
    /// The project cannot be read from its file.
    #[error("cannot read {}: {source}", path.display())]
    Read {
        /// The file.
        path: PathBuf,
        /// What the system said.
        source: io::Error,
    },
    /// The file is empty. A file of the project that this engine has written is never empty,
    /// so the system may have lost what was written last, as it can when the power fails.
    #[error(
        "cannot open {}: it is empty{}",
        path.display(),
        tmp.as_ref().map_or_else(String::new, |tmp| format!(
            "; {} may hold the last project that was saved",
            tmp.display()
        ))
    )]
    Empty {
        /// The file.
        path: PathBuf,
        /// `project.json.tmp`, where there is such a file.
        tmp: Option<PathBuf>,
    },
    /// What the file holds is not JSON, or not text at all.
    #[error("cannot open {}: it is not JSON: {source}", path.display())]
    NotJson {
        /// The file.
        path: PathBuf,
        /// What is wrong with it.
        source: serde_json::Error,
    },
    /// What the file holds is JSON, but not a project that this engine can open: one of another
    /// schema, one that is not a project, or one that the operations could not have made.
    #[error("cannot open {}: {source}", path.display())]
    NotAProject {
        /// The file.
        path: PathBuf,
        /// What is wrong with it.
        source: ReadError,
    },
    /// The project cannot be written, because there is a directory where it is written first.
    /// It has to be taken away by hand. The project that was saved last is still there as it was.
    #[error(
        "cannot write {}: there is a directory in its place, which has to be taken away",
        path.display()
    )]
    Blocked {
        /// `project.json.tmp`.
        path: PathBuf,
    },
    /// The project cannot be written. The project that was saved last is still there as it was.
    #[error("cannot write {}: {source}", path.display())]
    Write {
        /// The file that could not be written.
        path: PathBuf,
        /// What the system said.
        source: io::Error,
    },
    /// A save of the project panicked, as only a fault of the engine could make it. The project
    /// that was saved last is still there as it was.
    #[error("the save of {} failed with a panic: {message}", path.display())]
    Panicked {
        /// The file of the project.
        path: PathBuf,
        /// What the panic said.
        message: String,
    },
    /// The project was written, but it cannot be put in the place of the one that was saved
    /// last, which is still there as it was.
    #[error("cannot replace {}: {source}", path.display())]
    Replace {
        /// The file of the project.
        path: PathBuf,
        /// What the system said.
        source: io::Error,
    },
}

impl Store {
    /// Open the project of a directory.
    ///
    /// The directory and its `cache/` are made if they are not there. Where there is no
    /// `project.json`, the project is a new one with nothing in it, which is not written until it
    /// is first saved. Otherwise the file is read as JSON and then as a project by
    /// [`Project::from_value`], which checks it; a file that is not a project that this engine
    /// can open is refused, and left as it is.
    ///
    /// A `project.json.tmp` that a save left when the engine ended in the middle of it is taken
    /// away once the project has been read, where it is a file; until then it is kept, as the
    /// last project that was saved may be in it. A directory of that name is left alone, and
    /// saves fail until it is taken away.
    ///
    /// # Errors
    /// A directory cannot be made, the file cannot be read, or what it holds is not JSON or not
    /// a project that this engine can open. Each error names the file.
    pub fn open(dir: impl Into<PathBuf>) -> Result<(Store, Project), StoreError> {
        let store = Store { dir: dir.into() };
        for path in [store.dir.clone(), store.cache_dir()] {
            fs::create_dir_all(&path).map_err(|source| StoreError::CreateDir { path, source })?;
        }
        let project = store.read()?;
        let tmp = store.dir.join(PROJECT_TMP);
        if fs::symlink_metadata(&tmp).is_ok_and(|meta| meta.is_file()) {
            // The next save writes over it anyway, so it does not matter if this fails.
            let _ = fs::remove_file(&tmp);
        }
        Ok((store, project))
    }

    /// Read the project from its file, or make a new one where there is no file.
    fn read(&self) -> Result<Project, StoreError> {
        let path = self.project_path();
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Project::new()),
            Err(source) => return Err(StoreError::Read { path, source }),
        };
        if bytes.iter().all(u8::is_ascii_whitespace) {
            let tmp = self.dir.join(PROJECT_TMP);
            let tmp = tmp.is_file().then_some(tmp);
            return Err(StoreError::Empty { path, tmp });
        }
        let value = match serde_json::from_slice::<serde_json::Value>(&bytes) {
            Ok(value) => value,
            Err(source) => return Err(StoreError::NotJson { path, source }),
        };
        Project::from_value(value).map_err(|source| StoreError::NotAProject { path, source })
    }

    /// The directory of the project.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The file that holds the project.
    #[must_use]
    pub fn project_path(&self) -> PathBuf {
        self.dir.join(PROJECT_FILE)
    }

    /// The directory that holds what the engine has found out about the media.
    #[must_use]
    pub fn cache_dir(&self) -> PathBuf {
        self.dir.join(CACHE_DIR)
    }

    /// Write the project, whole or not at all.
    ///
    /// It is written as pretty JSON with a newline at the end to `project.json.tmp`, which is
    /// flushed to the disk and then put in the place of `project.json` in one step, so that the
    /// file of the project is always either the one that was saved last or this one. On Unix the
    /// directory is flushed too, where the system allows it. If anything fails, `project.json` is
    /// left as it was and `project.json.tmp` is taken away.
    ///
    /// On Windows a program that looks into files as they are written, such as a scanner of
    /// viruses or an indexer, may hold `project.json` open for a moment and so keep it from being
    /// replaced. Where the system refuses so, the engine tries again a few times, waiting a little
    /// longer each time, for a little more than a second in all.
    ///
    /// Two saves of the same directory must not be made at once, as both write
    /// `project.json.tmp`: while a [`Saver`](crate::Saver) of the directory exists, the project
    /// is saved only through it.
    ///
    /// # Errors
    /// The project cannot be written, or cannot be put in place. The project that was saved last
    /// is still there as it was.
    pub fn save(&self, project: &Project) -> Result<(), StoreError> {
        self.save_with(project, replace)
    }

    /// [`Store::save`], with what puts the written file in the place of the last one given, so
    /// that the tests can make it fail.
    fn save_with(
        &self,
        project: &Project,
        replace: impl FnOnce(&Path, &Path) -> io::Result<()>,
    ) -> Result<(), StoreError> {
        let tmp = self.dir.join(PROJECT_TMP);
        let written = serde_json::to_vec_pretty(project)
            .map_err(io::Error::from)
            .and_then(|mut bytes| {
                bytes.push(b'\n');
                let mut file = File::create(&tmp)?;
                file.write_all(&bytes)?;
                file.sync_all()
            });
        if let Err(source) = written {
            if tmp.is_dir() {
                return Err(StoreError::Blocked { path: tmp });
            }
            // It may not be there: either way there is nothing more to do.
            let _ = fs::remove_file(&tmp);
            return Err(StoreError::Write { path: tmp, source });
        }
        let path = self.project_path();
        if let Err(source) = replace(&tmp, &path) {
            let _ = fs::remove_file(&tmp);
            return Err(StoreError::Replace { path, source });
        }
        #[cfg(unix)]
        {
            // So that the new name of the file is on the disk too. Not every file system allows
            // a directory to be flushed, and the project is written either way.
            if let Ok(dir) = File::open(&self.dir) {
                let _ = dir.sync_all();
            }
        }
        Ok(())
    }
}

/// Put the file `from` in the place of the file `to`, in one step.
///
/// [`fs::rename`] replaces a file that is there on Windows as well as on Unix. Where Windows
/// refuses because another program has `to` open for a moment, it is tried again.
pub(crate) fn replace(from: &Path, to: &Path) -> io::Result<()> {
    retrying(|| fs::rename(from, to))
}

/// Do something to a file, and do it again a few times, waiting a little longer each time, while
/// Windows refuses it because another program has the file open.
pub(crate) fn retrying<T>(mut act: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    let mut wait = REPLACE_FIRST_WAIT;
    for _ in 1..REPLACE_TRIES {
        match act() {
            Err(error) if refused_for_a_while(&error) => {
                thread::sleep(wait);
                wait *= 2;
            }
            done => return done,
        }
    }
    act()
}

/// Whether the system refused to replace a file because another program has it open, which it
/// may stop doing in a moment.
fn refused_for_a_while(error: &io::Error) -> bool {
    /// What Windows says when another program has the file open and does not let it be
    /// replaced: `ERROR_SHARING_VIOLATION`, and `ERROR_LOCK_VIOLATION`. A file that is being
    /// deleted, or that a scanner has open in another way, is refused as `ERROR_ACCESS_DENIED`,
    /// which is the kind `PermissionDenied`.
    const SHARING_VIOLATION: i32 = 32;
    const LOCK_VIOLATION: i32 = 33;
    cfg!(windows)
        && (error.kind() == io::ErrorKind::PermissionDenied
            || matches!(
                error.raw_os_error(),
                Some(SHARING_VIOLATION | LOCK_VIOLATION)
            ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_project_that_cannot_be_put_in_place_leaves_the_last_one_and_no_tmp() {
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = Store::open(dir.path()).unwrap();
        store.save(&Project::new()).unwrap();
        let before = fs::read(store.project_path()).unwrap();

        let error = store
            .save_with(&Project::sample(), |from, _| {
                // The project was written whole before it was to be put in place.
                assert_eq!(
                    fs::read(from).unwrap(),
                    [
                        serde_json::to_vec_pretty(&Project::sample()).unwrap(),
                        b"\n".to_vec()
                    ]
                    .concat()
                );
                Err(io::Error::other("the disk is gone"))
            })
            .unwrap_err();

        assert!(matches!(error, StoreError::Replace { .. }), "{error}");
        let message = error.to_string();
        assert!(message.contains(PROJECT_FILE), "{message}");
        assert!(message.contains("the disk is gone"), "{message}");
        assert_eq!(fs::read(store.project_path()).unwrap(), before);
        assert!(!dir.path().join(PROJECT_TMP).exists());
    }

    #[test]
    fn only_a_refusal_of_windows_that_may_pass_is_tried_again() {
        let not_found = io::Error::from(io::ErrorKind::NotFound);
        assert!(!refused_for_a_while(&not_found));
        let sharing = io::Error::from_raw_os_error(32);
        assert_eq!(refused_for_a_while(&sharing), cfg!(windows));
    }

    #[test]
    fn an_error_that_will_not_pass_is_given_back_at_once() {
        let mut tries = 0;
        let error = retrying(|| -> io::Result<()> {
            tries += 1;
            Err(io::Error::from(io::ErrorKind::NotFound))
        })
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert_eq!(tries, 1);
    }
}
