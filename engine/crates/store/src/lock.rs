//! The lock of a project: the file that tells which engine has the project open, and where the
//! other programs of the studio can reach it.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use sysinfo::{Pid, ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, System};

/// The name of the file of the lock, in the directory of the project.
pub const LOCK_FILE: &str = "engine.lock";

/// What the lock of a project says: which engine has the project open, and where it can be
/// reached.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockInfo {
    /// The id of the process of the engine.
    pub pid: u32,
    /// Where the engine can be reached.
    pub address: String,
    /// When the engine took the lock, in RFC 3339, in UTC.
    pub started: String,
    /// When the process of the engine began, in seconds since the start of 1970, as the system
    /// tells it; none where the system does not tell it.
    ///
    /// The id of a process that has ended may be given to another one. A lock whose process is
    /// running but began at another time was left by an engine that has ended, and is taken
    /// over (see [`Lock::acquire`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_started: Option<u64>,
}

impl LockInfo {
    /// When the engine took the lock, if `started` can be read.
    #[must_use]
    pub fn started_at(&self) -> Option<SystemTime> {
        humantime::parse_rfc3339(&self.started).ok()
    }
}

/// The lock of a project, held by this engine. It is let go, and its file taken away, when it
/// is dropped.
#[derive(Debug)]
pub struct Lock {
    path: PathBuf,
    info: LockInfo,
}

/// Why the lock of a project cannot be taken.
#[derive(Debug, thiserror::Error)]
pub enum LockError {
    /// Another engine that is still running has the project open.
    #[error("the project is open in another engine, process {pid}, at {address}")]
    Held {
        /// The id of the process of that engine.
        pid: u32,
        /// Where that engine can be reached.
        address: String,
    },
    /// The lock was left by an engine that has ended, but another engine was taking it at the
    /// same time.
    #[error("cannot take {}: another engine is taking it at the same time", path.display())]
    Contended {
        /// The file of the lock.
        path: PathBuf,
    },
    /// The file of the lock, or the directory of the project, cannot be made, written or taken
    /// away.
    #[error("cannot take {}: {source}", path.display())]
    Io {
        /// The file or directory.
        path: PathBuf,
        /// What the system said.
        source: io::Error,
    },
}

impl Lock {
    /// Take the lock of the project in `dir` for this process, which can be reached at
    /// `address`. The directory is made if it is not there.
    ///
    /// The file of the lock is made only if it is not there, so that two engines cannot both
    /// make it. Where it is there, it is read. If the process that it names is running, and
    /// began when the lock says (where both are known), another engine has the project open, and
    /// the lock is refused. Otherwise it was left by an engine that has ended: its process is
    /// gone, its id has been given to another process, or the file cannot be read or does not
    /// say who holds it. Then it is taken away, if it has not changed since it was read, and
    /// taken again, once.
    ///
    /// # Errors
    /// Another engine that is still running holds the lock; it was taken over by another engine
    /// at the same time; or the file of the lock cannot be made, written or taken away.
    pub fn acquire(dir: &Path, address: &str) -> Result<Lock, LockError> {
        fs::create_dir_all(dir).map_err(|source| LockError::Io {
            path: dir.to_path_buf(),
            source,
        })?;
        let path = dir.join(LOCK_FILE);
        let pid = std::process::id();
        let info = LockInfo {
            pid,
            address: address.to_owned(),
            started: humantime::format_rfc3339_millis(SystemTime::now()).to_string(),
            process_started: find(pid).and_then(|process| process.began),
        };
        let mut text = serde_json::to_vec_pretty(&info).map_err(|source| LockError::Io {
            path: path.clone(),
            source: source.into(),
        })?;
        text.push(b'\n');

        let mut taken_over = false;
        loop {
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    if let Err(source) = file.write_all(&text).and_then(|()| file.sync_all()) {
                        drop(file);
                        let _ = fs::remove_file(&path);
                        return Err(LockError::Io { path, source });
                    }
                    return Ok(Lock { path, info });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(source) => return Err(LockError::Io { path, source }),
            }
            let seen = match fs::read(&path) {
                Ok(bytes) => Some(bytes),
                // It was taken away since: it is taken again below.
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    if taken_over {
                        return Err(LockError::Contended { path });
                    }
                    taken_over = true;
                    continue;
                }
                Err(_) => None,
            };
            let holder = seen
                .as_deref()
                .and_then(|bytes| serde_json::from_slice::<LockInfo>(bytes).ok());
            if let Some(holder) = holder.filter(is_running) {
                return Err(LockError::Held {
                    pid: holder.pid,
                    address: holder.address,
                });
            }
            if taken_over {
                return Err(LockError::Contended { path });
            }
            taken_over = true;
            remove_if_unchanged(&path, seen.as_deref()).map_err(|source| LockError::Io {
                path: path.clone(),
                source,
            })?;
        }
    }

    /// What the lock of the project in `dir` says, if there is a lock and it can be read. The
    /// engine that it names may have ended.
    #[must_use]
    pub fn read(dir: &Path) -> Option<LockInfo> {
        let bytes = fs::read(dir.join(LOCK_FILE)).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// What this lock says.
    #[must_use]
    pub fn info(&self) -> &LockInfo {
        &self.info
    }

    /// The file of this lock.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Lock {
    /// Take the file of the lock away, but only if it still says what this lock wrote: a lock
    /// that another engine has taken since is left to it.
    fn drop(&mut self) {
        let ours = fs::read(&self.path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<LockInfo>(&bytes).ok())
            .is_some_and(|info| info == self.info);
        if ours {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// Take away the file of a lock that was judged to be left by an engine that has ended, if it
/// still holds what was read when it was judged so (`seen`, where it could be read). If it holds
/// something else, another engine has written it since, and it is left to be judged again.
fn remove_if_unchanged(path: &Path, seen: Option<&[u8]>) -> io::Result<()> {
    if let Some(seen) = seen {
        match fs::read(path) {
            Ok(now) if now != seen => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            _ => {}
        }
    }
    match fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

/// Whether the engine that a lock names is still running: its process is, and began when the
/// lock says, where both the lock and the system tell when it began.
///
/// The system tells when a process began in whole seconds, so a second either way is allowed.
fn is_running(info: &LockInfo) -> bool {
    match find(info.pid) {
        None => false,
        Some(process) => match (info.process_started, process.began) {
            (Some(said), Some(began)) => said.abs_diff(began) <= 1,
            _ => true,
        },
    }
}

/// A process that is running.
struct Running {
    /// When it began, in seconds since the start of 1970, where the system tells it.
    began: Option<u64>,
}

/// The process with this id, if one is running. One that has ended but has not yet been waited
/// for by its parent is not running.
fn find(pid: u32) -> Option<Running> {
    let pid = Pid::from_u32(pid);
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::nothing(),
    );
    let process = system.process(pid)?;
    if matches!(
        process.status(),
        ProcessStatus::Zombie | ProcessStatus::Dead
    ) {
        return None;
    }
    // The system says 0 where it cannot tell, as for a process of another user on Windows.
    let began = Some(process.start_time()).filter(|&began| began > 0);
    Some(Running { began })
}
