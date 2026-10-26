//! The lock of a project: what keeps two engines from having it open at once, and the file that
//! tells the other programs of the studio which engine has it open and where to reach it.

use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use serde::{Deserialize, Serialize};

use crate::store::{replace, retrying, CACHE_DIR};

/// The name of the file that says which engine has the project open, in its directory.
pub const LOCK_FILE: &str = "engine.lock";

/// The name of the file that `engine.lock` is written to first.
const LOCK_TMP: &str = "engine.lock.tmp";

/// The name of the file whose lock of the system is the lock of the project, in `cache/`.
pub const GUARD_FILE: &str = "engine.guard";

/// How long an engine waits for the engine that holds the guard to say who it is, before it
/// gives up.
const WAIT_FOR_HOLDER: Duration = Duration::from_secs(1);

/// How long an engine waits between two looks at the guard and `engine.lock`.
const LOOK_AGAIN: Duration = Duration::from_millis(20);

/// What `engine.lock` says: which engine has the project open, and where it can be reached.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockInfo {
    /// The id of the process of the engine.
    pub pid: u32,
    /// Where the engine can be reached.
    pub address: String,
    /// When the engine took the lock, in RFC 3339, in UTC.
    pub started: String,
}

impl LockInfo {
    /// When the engine took the lock, if `started` can be read.
    #[must_use]
    pub fn started_at(&self) -> Option<SystemTime> {
        humantime::parse_rfc3339(&self.started).ok()
    }
}

/// The lock of a project, held by this engine for as long as it is kept. When it is dropped,
/// `engine.lock` is taken away and the guard is let go.
///
/// # How it works
///
/// The lock itself is a lock of the system on a file of its own, `cache/engine.guard`, taken with
/// [`File::try_lock`] (`flock` on Unix, `LockFileEx` on Windows). Only one open file can hold it
/// at a time, and the system lets it go when the process that holds it ends, however it ends. So
/// no engine has to judge whether another one is still running from the id of its process or
/// the time that it began, and a lock left by an engine that has crashed is simply free.
///
/// The guard is never taken away. Were it deleted while it is held, another engine could make a
/// new file of the same name and lock that one, and two engines would each hold a lock.
///
/// Who holds the lock, and where it can be reached, is written to `engine.lock` in the directory
/// of the project, and only by the engine that holds the guard: to `engine.lock.tmp` first,
/// which is then put in the place of `engine.lock` in one step, so that no one reads it half
/// written. What an engine that has ended left there is written over. When an engine lets the
/// lock go, it takes `engine.lock` away if it still says what that engine wrote, and only then
/// lets the guard go; nothing else ever takes `engine.lock` away.
///
/// The two are kept apart because a lock of Windows keeps other programs from reading the file
/// that it is on. The other programs of the studio only read `engine.lock`, and never lock
/// anything. What they read may have been left by an engine that has crashed; they find that out
/// when they cannot reach it.
#[derive(Debug)]
pub struct Lock {
    /// The guard, locked.
    guard: File,
    /// `engine.lock`.
    path: PathBuf,
    /// What this engine wrote to it.
    info: LockInfo,
}

/// Why the lock of a project cannot be taken.
#[derive(Debug, thiserror::Error)]
pub enum LockError {
    /// Another engine has the project open.
    #[error("the project is open in another engine, process {pid}, at {address}")]
    Held {
        /// The id of the process of that engine.
        pid: u32,
        /// Where that engine can be reached.
        address: String,
    },
    /// Another engine holds the lock, but has not said who it is or where it can be reached.
    #[error(
        "the project is open in another engine, which has not written {} yet",
        path.display()
    )]
    Contended {
        /// `engine.lock`.
        path: PathBuf,
    },
    /// The guard or `engine.lock`, or the directory that they are in, cannot be made, locked or
    /// written.
    #[error("cannot take the lock of the project at {}: {source}", path.display())]
    Io {
        /// The file or directory.
        path: PathBuf,
        /// What the system said.
        source: io::Error,
    },
}

impl Lock {
    /// Take the lock of the project in `dir` for this process, which can be reached at
    /// `address`. The directory and its `cache/` are made if they are not there.
    ///
    /// The guard is locked, and then `engine.lock` is written. Where another engine holds the
    /// guard, `engine.lock` is read, and the lock is refused with what it says. If it cannot be
    /// read, as when that engine has only just taken the guard and is still writing it, the
    /// engine looks again, at the guard as well, for up to a second before it gives up.
    ///
    /// # Errors
    /// Another engine holds the lock ([`LockError::Held`], or [`LockError::Contended`] where it
    /// has not said who it is), or the guard or `engine.lock` cannot be made, locked or written.
    pub fn acquire(dir: &Path, address: &str) -> Result<Lock, LockError> {
        let io_at = |path: &Path| {
            let path = path.to_path_buf();
            move |source| LockError::Io { path, source }
        };
        let cache = dir.join(CACHE_DIR);
        fs::create_dir_all(&cache).map_err(io_at(&cache))?;
        let guard_path = cache.join(GUARD_FILE);
        let guard = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&guard_path)
            .map_err(io_at(&guard_path))?;
        let path = dir.join(LOCK_FILE);

        let deadline = Instant::now() + WAIT_FOR_HOLDER;
        loop {
            match guard.try_lock() {
                Ok(()) => break,
                Err(TryLockError::WouldBlock) => {}
                Err(TryLockError::Error(source)) => {
                    return Err(LockError::Io {
                        path: guard_path,
                        source,
                    })
                }
            }
            // Any error of reading counts as not written yet: on Windows a file that is being
            // put in place is refused for a moment as `ERROR_ACCESS_DENIED`.
            if let Some(holder) = read_at(&path) {
                return Err(LockError::Held {
                    pid: holder.pid,
                    address: holder.address,
                });
            }
            if Instant::now() >= deadline {
                return Err(LockError::Contended { path });
            }
            thread::sleep(LOOK_AGAIN);
        }

        let info = LockInfo {
            pid: std::process::id(),
            address: address.to_owned(),
            started: humantime::format_rfc3339_millis(SystemTime::now()).to_string(),
        };
        let tmp = dir.join(LOCK_TMP);
        let written = serde_json::to_vec_pretty(&info)
            .map_err(io::Error::from)
            .and_then(|mut text| {
                text.push(b'\n');
                File::create(&tmp)?.write_all(&text)
            });
        if let Err(source) = written {
            let _ = fs::remove_file(&tmp);
            return Err(LockError::Io { path: tmp, source });
        }
        // A program of the studio that is reading `engine.lock` may keep it from being replaced
        // for a moment on Windows; `replace` tries again.
        if let Err(source) = replace(&tmp, &path) {
            let _ = fs::remove_file(&tmp);
            return Err(LockError::Io { path, source });
        }
        Ok(Lock { guard, path, info })
    }

    /// What `engine.lock` in `dir` says, if it is there and can be read. The engine that it
    /// names may have ended.
    #[must_use]
    pub fn read(dir: &Path) -> Option<LockInfo> {
        read_at(&dir.join(LOCK_FILE))
    }

    /// What this engine wrote to `engine.lock`.
    #[must_use]
    pub fn info(&self) -> &LockInfo {
        &self.info
    }

    /// `engine.lock`.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Lock {
    /// Take `engine.lock` away if it still says what this engine wrote, and then let the guard
    /// go. The guard itself is kept.
    fn drop(&mut self) {
        let ours = read_at(&self.path)
            .is_some_and(|info| info.pid == self.info.pid && info.started == self.info.started);
        if ours {
            let _ = retrying(|| fs::remove_file(&self.path));
        }
        // Closing the guard, as dropping it does next, lets it go too.
        let _ = self.guard.unlock();
    }
}

/// What the `engine.lock` at `path` says, if it is there and can be read.
fn read_at(path: &Path) -> Option<LockInfo> {
    let bytes = fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}
