//! The directory of a project on disk: the project, written whole or not at all, and the lock
//! that tells which engine has it open.
//!
//! A project is a directory:
//!
//! ```text
//! <project>/
//!   project.json   the project
//!   engine.lock    the engine that has it open: {"pid", "address", "started", "process_started"}
//!   cache/         what the engine has found out about the media, kept to be used again
//! ```
//!
//! One engine at a time has a project open. It takes the [`Lock`] first, which is refused while
//! another engine that is still running holds it, then opens the [`Store`], and saves through a
//! [`Saver`] after each change. The other programs of the studio find the engine of a project
//! through the address in its lock ([`Lock::read`]).

mod lock;
mod saver;
mod store;

pub use lock::{Lock, LockError, LockInfo, LOCK_FILE};
pub use saver::{Saver, SAVE_PERIOD};
pub use store::{Store, StoreError, CACHE_DIR, PROJECT_FILE, PROJECT_TMP};
