//! The thread that runs the mix ([`Engine::step`]) and paces it: after each step it naps for
//! [`NAP`], a fraction of the sound kept ahead of the device, so that it tops the ring up long
//! before the device empties it. The nap is a sleep for a time, not a wait for another thread: no
//! lock, no condition, nothing that another thread holds it up by. This file is not one of those
//! of the threads of the sound that `tests/rt_audit.rs` reads (it names the sleep); what the
//! thread does between naps is `engine.rs`.
//!
//! # Priority (Windows)
//!
//! The thread asks the Multimedia Class Scheduler for the task "Pro Audio"
//! (`AvSetMmThreadCharacteristicsW`), as the threads of the sound of other programs do, so that
//! a video that plays or a game does not hold it up; where that is refused it raises its own
//! priority to time critical (`SetThreadPriority`). Either is taken back when the thread ends.
//! [`MixThread::priority`] says which it got. Elsewhere the thread keeps the priority it has.

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::engine::Engine;

/// How long the thread of the mix naps between steps.
pub const NAP: Duration = Duration::from_millis(1);

/// The priority that the thread of the mix got.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Priority {
    /// Not raised yet, or not on this system.
    Normal,
    /// The task "Pro Audio" of the Multimedia Class Scheduler.
    ProAudio,
    /// The priority of a thread, time critical.
    TimeCritical,
}

impl Priority {
    fn of(code: u8) -> Priority {
        match code {
            1 => Priority::ProAudio,
            2 => Priority::TimeCritical,
            _ => Priority::Normal,
        }
    }
}

/// The thread of the mix, running.
#[derive(Debug)]
pub struct MixThread {
    quit: Arc<AtomicBool>,
    busy: Arc<AtomicU64>,
    cpu: Arc<AtomicU64>,
    priority: Arc<AtomicU8>,
    thread: Option<JoinHandle<()>>,
}

impl MixThread {
    /// Starts `engine` on a thread of its own, at the priority of audio (see the module), which
    /// first plans the transforms of `RNNoise` there (T12: they take memory on the first use on
    /// a thread).
    ///
    /// # Errors
    ///
    /// The thread cannot be started.
    pub fn spawn(mut engine: Engine) -> std::io::Result<MixThread> {
        let quit = Arc::new(AtomicBool::new(false));
        let busy = Arc::new(AtomicU64::new(0));
        let cpu = Arc::new(AtomicU64::new(0));
        let priority = Arc::new(AtomicU8::new(0));
        let thread = std::thread::Builder::new()
            .name("columbina-sound-mix".into())
            .spawn({
                let quit = Arc::clone(&quit);
                let busy = Arc::clone(&busy);
                let cpu = Arc::clone(&cpu);
                let priority = Arc::clone(&priority);
                move || {
                    let raised = raise();
                    priority.store(raised.code(), Ordering::Release);
                    crate::fx::rnnoise::warm_thread();
                    while !quit.load(Ordering::Acquire) {
                        let started = Instant::now();
                        engine.step();
                        let spent = u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX);
                        busy.fetch_add(spent, Ordering::Relaxed);
                        cpu.store(processor_time(), Ordering::Relaxed);
                        std::thread::sleep(NAP);
                    }
                    raised.lower();
                }
            })?;
        Ok(MixThread {
            quit,
            busy,
            cpu,
            priority,
            thread: Some(thread),
        })
    }

    /// The time spent mixing so far (not napping), in nanoseconds of the clock: preemption by
    /// other threads counts.
    #[must_use]
    pub fn busy(&self) -> u64 {
        self.busy.load(Ordering::Relaxed)
    }

    /// The time of the processor that the thread has taken so far, in nanoseconds (Windows; 0
    /// elsewhere).
    #[must_use]
    pub fn cpu(&self) -> u64 {
        self.cpu.load(Ordering::Relaxed)
    }

    /// The priority that the thread got (once it started).
    #[must_use]
    pub fn priority(&self) -> Priority {
        Priority::of(self.priority.load(Ordering::Acquire))
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

/// The priority that the calling thread was raised to, to be taken back.
enum Raised {
    Normal,
    #[cfg(windows)]
    ProAudio(windows::Win32::Foundation::HANDLE),
    #[cfg(windows)]
    TimeCritical(i32),
}

impl Raised {
    fn code(&self) -> u8 {
        match self {
            Raised::Normal => 0,
            #[cfg(windows)]
            Raised::ProAudio(_) => 1,
            #[cfg(windows)]
            Raised::TimeCritical(_) => 2,
        }
    }

    /// Takes the priority back.
    #[allow(unsafe_code)]
    fn lower(self) {
        match self {
            Raised::Normal => {}
            #[cfg(windows)]
            Raised::ProAudio(handle) => {
                // SAFETY: the handle is the one that `AvSetMmThreadCharacteristicsW` gave this
                // thread, taken back once.
                let _ = unsafe {
                    windows::Win32::System::Threading::AvRevertMmThreadCharacteristics(handle)
                };
            }
            #[cfg(windows)]
            Raised::TimeCritical(before) => {
                use windows::Win32::System::Threading::{
                    GetCurrentThread, SetThreadPriority, THREAD_PRIORITY,
                };
                // SAFETY: the pseudo handle of the calling thread, always valid on it.
                let _ = unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY(before)) };
            }
        }
    }
}

/// Raises the calling thread to the priority of audio: see the module.
#[cfg(windows)]
#[allow(unsafe_code)]
fn raise() -> Raised {
    use windows::Win32::System::Threading::{
        AvSetMmThreadCharacteristicsW, GetCurrentThread, GetThreadPriority, SetThreadPriority,
        THREAD_PRIORITY_TIME_CRITICAL,
    };
    let mut index = 0_u32;
    // SAFETY: a constant name of a task, and a place for its index that outlives the call.
    if let Ok(handle) =
        unsafe { AvSetMmThreadCharacteristicsW(windows::core::w!("Pro Audio"), &raw mut index) }
    {
        return Raised::ProAudio(handle);
    }
    // SAFETY: the pseudo handle of the calling thread, always valid on it.
    let before = unsafe { GetThreadPriority(GetCurrentThread()) };
    // SAFETY: as above.
    match unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL) } {
        Ok(()) => Raised::TimeCritical(before),
        Err(_) => Raised::Normal,
    }
}

#[cfg(not(windows))]
fn raise() -> Raised {
    Raised::Normal
}

/// The time of the processor that the calling thread has taken, in nanoseconds.
#[cfg(windows)]
#[allow(unsafe_code)]
fn processor_time() -> u64 {
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::System::Threading::{GetCurrentThread, GetThreadTimes};
    let (mut created, mut ended, mut kernel, mut user) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );
    // SAFETY: the pseudo handle of the calling thread, and four places that outlive the call.
    let read = unsafe {
        GetThreadTimes(
            GetCurrentThread(),
            &raw mut created,
            &raw mut ended,
            &raw mut kernel,
            &raw mut user,
        )
    };
    if read.is_err() {
        return 0;
    }
    let hundreds =
        |time: FILETIME| u64::from(time.dwLowDateTime) | (u64::from(time.dwHighDateTime) << 32);
    (hundreds(kernel) + hundreds(user)) * 100
}

#[cfg(not(windows))]
fn processor_time() -> u64 {
    0
}
