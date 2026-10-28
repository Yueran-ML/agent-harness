//! The timers of the system made fine while the transport plays.
//!
//! A thread of the engine that waits for the time of the next frame wakes when the timers of
//! the system next tick: every 15.6 ms on Windows unless a program asks for finer ones, which is
//! as long as a frame at 60 frames a second. While it plays, the transport asks for timers of
//! 1 ms (`timeBeginPeriod`), as every player of media on Windows does, and lets them go when it
//! stops: since Windows 10 2004 this changes the timers of the engine alone. On macOS the
//! transport's thread is classified as user-interactive work, so its frame deadlines are not
//! coalesced like those of background work. It blocks without doing work when paused.

/// Classify the transport's frame deadlines for the system's scheduler.
pub fn prepare_thread() {
    #[cfg(target_os = "macos")]
    // SAFETY: this sets only the calling thread's QoS, with a documented class and priority.
    #[allow(unsafe_code)]
    unsafe {
        let result =
            libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_USER_INTERACTIVE, 0);
        if result != 0 {
            crate::log::log(
                "transport_qos_failed",
                &serde_json::json!({ "message": std::io::Error::from_raw_os_error(result).to_string() }),
            );
        }
    }
}

/// Fine timers, for as long as it is held.
pub struct Fine(());

impl Fine {
    pub fn new() -> Fine {
        #[cfg(windows)]
        // SAFETY: timeBeginPeriod takes any period, and is undone by the timeEndPeriod of the
        // same period in `drop`.
        #[allow(unsafe_code)]
        unsafe {
            windows::Win32::Media::timeBeginPeriod(1);
        }
        Fine(())
    }
}

impl Drop for Fine {
    fn drop(&mut self) {
        #[cfg(windows)]
        // SAFETY: the period asked for in `new`.
        #[allow(unsafe_code)]
        unsafe {
            windows::Win32::Media::timeEndPeriod(1);
        }
    }
}
