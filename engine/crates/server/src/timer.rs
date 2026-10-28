//! The timers of the system made fine while the transport plays.
//!
//! A thread of the engine that waits for the time of the next frame wakes when the timers of
//! the system next tick: every 15.6 ms on Windows unless a program asks for finer ones, which is
//! as long as a frame at 60 frames a second. While it plays, the transport asks for timers of
//! 1 ms (`timeBeginPeriod`), as every player of media on Windows does, and lets them go when it
//! stops: since Windows 10 2004 this changes the timers of the engine alone. On macOS a
//! latency-critical activity asks for precise timers during playback. It ends on pause or
//! stop, and allows the system to sleep while idle.

#[cfg(target_os = "macos")]
use objc2::{
    rc::{autoreleasepool, Retained},
    runtime::ProtocolObject,
};
#[cfg(target_os = "macos")]
use objc2_foundation::{ns_string, NSActivityOptions, NSObjectProtocol, NSProcessInfo};

/// Fine timers, for as long as it is held.
pub struct Fine {
    #[cfg(target_os = "macos")]
    activity: Retained<ProtocolObject<dyn NSObjectProtocol>>,
}

impl Fine {
    pub fn new() -> Fine {
        #[cfg(windows)]
        // SAFETY: timeBeginPeriod takes any period, and is undone by the timeEndPeriod of the
        // same period in `drop`.
        #[allow(unsafe_code)]
        unsafe {
            windows::Win32::Media::timeBeginPeriod(1);
        }
        Fine {
            #[cfg(target_os = "macos")]
            // This Rust worker has no Cocoa run loop to drain temporary Foundation objects.
            activity: autoreleasepool(|_| {
                NSProcessInfo::processInfo().beginActivityWithOptions_reason(
                    NSActivityOptions::UserInitiatedAllowingIdleSystemSleep
                        | NSActivityOptions::LatencyCritical,
                    ns_string!("Video playback frame deadlines"),
                )
            }),
        }
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
        #[cfg(target_os = "macos")]
        // SAFETY: activity is the token returned by beginActivityWithOptions_reason and is
        // ended exactly once, when this playback guard is dropped.
        #[allow(unsafe_code)]
        autoreleasepool(|_| unsafe {
            NSProcessInfo::processInfo().endActivity(&self.activity);
        });
    }
}
