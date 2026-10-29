//! Timeline playback: independent track readers, composition and the monitor transport.

// The decoder pool is introduced first; the timeline worker is connected in the next part
// of Task 15. Keep it compiled in ordinary builds as well as in its real-media tests.
#[allow(dead_code, reason = "Task 15 timeline worker is not connected yet")]
mod ahead;
#[allow(dead_code, reason = "Task 15 timeline worker is not connected yet")]
mod sources;
