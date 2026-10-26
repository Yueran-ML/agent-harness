//! Saving a project after it changes: on a thread of its own, no more often than once in a
//! while, and always the latest project.

use std::io;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use columbina_model::Project;

use crate::store::{Store, StoreError};

/// How often, at most, a [`Saver`] saves the project.
pub const SAVE_PERIOD: Duration = Duration::from_millis(500);

/// How many times the period is doubled, at most, while saving keeps failing: so a project that
/// cannot be saved is tried again no less often than every 64 periods (32 seconds).
const MOST_DOUBLINGS: u32 = 6;

/// What saves the project of a [`Store`] after it changes.
///
/// The engine tells it of each change with [`Saver::changed`], giving it the project as it now
/// is. It saves on a thread of its own, at most once a period ([`SAVE_PERIOD`] unless another is
/// given): a change after a quiet while is saved at once, and the changes that come within a
/// period of a save are saved together a period after it, as the latest of them. So it never
/// saves a project that is older than one it has already saved.
///
/// An error of a save on its thread is given to the function that it was made with, on that
/// thread, and the project is kept to be saved again: at the next period, or later where saving
/// keeps failing, or at once by [`Saver::flush`]. [`Saver::flush`] saves at once and waits, and
/// gives its error to the caller. When the saver is dropped, it saves the project that it has
/// not yet saved and waits for it, and gives an error to the function as well.
pub struct Saver {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

/// What a saver and its thread share.
struct Shared {
    store: Store,
    state: Mutex<State>,
    /// Told whenever the state changes.
    changed: Condvar,
}

struct State {
    /// The latest project that has not yet been saved.
    pending: Option<Project>,
    /// Whether a project is being saved now, by the thread or by [`Saver::flush`]. Only one is
    /// saved at a time.
    saving: bool,
    /// When the last save began.
    last: Option<Instant>,
    /// How many saves in a row have failed.
    failures: u32,
    /// How many saves have succeeded.
    saves: u64,
    /// Whether the saver is being dropped.
    stop: bool,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn wait<'a>(&self, state: MutexGuard<'a, State>) -> MutexGuard<'a, State> {
        self.changed
            .wait(state)
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Save `project`, which was taken from `pending` with `saving` set, and set the state by
    /// what came of it. A project that could not be saved is kept to be saved again, unless a
    /// later one has come since or the saver is being dropped.
    fn save(&self, project: Project) -> Result<(), StoreError> {
        let result = self.store.save(&project);
        let mut state = self.lock();
        state.saving = false;
        if result.is_ok() {
            state.saves += 1;
            state.failures = 0;
        } else {
            state.failures = state.failures.saturating_add(1);
            if state.pending.is_none() && !state.stop {
                state.pending = Some(project);
            }
        }
        drop(state);
        self.changed.notify_all();
        result
    }
}

impl Saver {
    /// A saver of the project of `store`, which saves at most once every [`SAVE_PERIOD`] and
    /// gives each error of a save on its thread to `report`.
    ///
    /// # Errors
    /// Its thread cannot be started.
    pub fn new(store: Store, report: impl FnMut(StoreError) + Send + 'static) -> io::Result<Saver> {
        Saver::with_period(store, SAVE_PERIOD, report)
    }

    /// A saver of the project of `store`, which saves at most once every `period` and gives
    /// each error of a save on its thread to `report`.
    ///
    /// # Errors
    /// Its thread cannot be started.
    pub fn with_period(
        store: Store,
        period: Duration,
        report: impl FnMut(StoreError) + Send + 'static,
    ) -> io::Result<Saver> {
        let shared = Arc::new(Shared {
            store,
            state: Mutex::new(State {
                pending: None,
                saving: false,
                last: None,
                failures: 0,
                saves: 0,
                stop: false,
            }),
            changed: Condvar::new(),
        });
        let thread = thread::Builder::new()
            .name("columbina-saver".into())
            .spawn({
                let shared = Arc::clone(&shared);
                move || run(&shared, period, report)
            })?;
        Ok(Saver {
            shared,
            thread: Some(thread),
        })
    }

    /// Tell the saver that the project has changed, and is now `project`. It is saved on the
    /// thread of the saver, in place of any earlier one that has not yet been saved.
    pub fn changed(&self, project: Project) {
        self.shared.lock().pending = Some(project);
        self.shared.changed.notify_all();
    }

    /// Save the latest project now, if there is one that has not been saved, and wait for it. A
    /// save that is being made on the thread of the saver is waited for first.
    ///
    /// # Errors
    /// The project cannot be saved. It is kept to be saved again.
    pub fn flush(&self) -> Result<(), StoreError> {
        let mut state = self.shared.lock();
        while state.saving {
            state = self.shared.wait(state);
        }
        let Some(project) = state.pending.take() else {
            return Ok(());
        };
        state.saving = true;
        state.last = Some(Instant::now());
        drop(state);
        self.shared.save(project)
    }

    /// How many times the saver has saved the project.
    #[must_use]
    pub fn saves(&self) -> u64 {
        self.shared.lock().saves
    }
}

impl Drop for Saver {
    /// Save the project that has not yet been saved, and wait for it and for the thread to end.
    fn drop(&mut self) {
        self.shared.lock().stop = true;
        self.shared.changed.notify_all();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The thread of a saver: wait for a change, and for the period since the last save to pass,
/// then save the latest project; until the saver is dropped and there is nothing left to save.
fn run(shared: &Shared, period: Duration, mut report: impl FnMut(StoreError)) {
    let mut state = shared.lock();
    loop {
        if state.saving || state.pending.is_none() {
            if state.stop && !state.saving {
                return;
            }
            state = shared.wait(state);
            continue;
        }
        if !state.stop {
            let wait = period.saturating_mul(1 << state.failures.min(MOST_DOUBLINGS));
            if let Some(due) = state.last.map(|last| last + wait) {
                let now = Instant::now();
                if now < due {
                    state = shared
                        .changed
                        .wait_timeout(state, due - now)
                        .unwrap_or_else(PoisonError::into_inner)
                        .0;
                    continue;
                }
            }
        }
        let Some(project) = state.pending.take() else {
            continue;
        };
        state.saving = true;
        state.last = Some(Instant::now());
        drop(state);
        if let Err(error) = shared.save(project) {
            report(error);
        }
        state = shared.lock();
    }
}
