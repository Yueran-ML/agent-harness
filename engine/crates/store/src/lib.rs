//! The directory of a project on disk: the project, written whole or not at all, and the lock
//! that keeps two engines from having it open at once.
//!
//! A project is a directory:
//!
//! ```text
//! <project>/
//!   project.json      the project
//!   engine.lock       the engine that has it open: {"pid", "address", "started"}
//!   cache/            what the engine has found out about the media, kept to be used again
//!     engine.guard    the file whose lock of the system is the lock of the project
//! ```
//!
//! One engine at a time has a project open. It takes the [`Lock`] first, which is refused while
//! another engine holds it, then opens the [`Store`], and makes a [`Saver`] that it tells of
//! each change. While the saver exists, the project is saved only through it, never with
//! [`Store::save`] beside it. When the engine ends, it flushes the saver and drops it, and only
//! then drops the lock, so that no other engine opens the project before the last change is
//! written. The other programs of the studio find the engine of a project through the address
//! in `engine.lock` ([`Lock::read`]); how the lock works is told at [`Lock`].

mod lock;
mod saver;
mod store;

pub use lock::{Lock, LockError, LockInfo, GUARD_FILE, LOCK_FILE};
pub use saver::{Saver, SAVE_PERIOD};
pub use store::{Store, StoreError, CACHE_DIR, PROJECT_FILE, PROJECT_TMP};
