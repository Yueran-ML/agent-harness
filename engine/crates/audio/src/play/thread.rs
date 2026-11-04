//! The thread that runs the mix ([`Engine::step`]) and paces it: after each step it naps for
//! [`NAP`], a fraction of the sound kept ahead of the device, so that it tops the ring up long
//! before the device empties it. The nap is a sleep for a time, not a wait for another thread: no
//! lock, no condition, nothing that another thread holds it up by. This file is not one of those
//! of the threads of the sound that `tests/rt_audit.rs` reads (it names the sleep); what the
//! thread does between naps is `engine.rs`.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::engine::Engine;

/// How long the thread of the mix naps between steps.
pub const NAP: Duration = Duration::from_millis(1);

/// The thread of the mix, running.
#[derive(Debug)]
pub struct MixThread {
    quit: Arc<AtomicBool>,
    busy: Arc<AtomicU64>,
    thread: Option<JoinHandle<()>>,
}

impl MixThread {
    /// Starts `engine` on a thread of its own, which first plans the transforms of `RNNoise`
    /// there (T12: they take memory on the first use on a thread).
    ///
    /// # Errors
    ///
    /// The thread cannot be started.
    pub fn spawn(mut engine: Engine) -> std::io::Result<MixThread> {
        let quit = Arc::new(AtomicBool::new(false));
        let busy = Arc::new(AtomicU64::new(0));
        let thread = std::thread::Builder::new()
            .name("columbina-sound-mix".into())
            .spawn({
                let quit = Arc::clone(&quit);
                let busy = Arc::clone(&busy);
                move || {
                    crate::fx::rnnoise::warm_thread();
                    while !quit.load(Ordering::Acquire) {
                        let started = Instant::now();
                        engine.step();
                        let spent = u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX);
                        busy.fetch_add(spent, Ordering::Relaxed);
                        std::thread::sleep(NAP);
                    }
                }
            })?;
        Ok(MixThread {
            quit,
            busy,
            thread: Some(thread),
        })
    }

    /// The time spent mixing so far (not napping), in nanoseconds.
    #[must_use]
    pub fn busy(&self) -> u64 {
        self.busy.load(Ordering::Relaxed)
    }
}

impl Drop for MixThread {
    fn drop(&mut self) {
        self.quit.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
