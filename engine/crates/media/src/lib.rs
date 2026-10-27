//! Sources: what a file is and what it holds, and the pictures in it, read with libav
//! (FFmpeg's libraries), on the graphics card where it can.
//!
//! - [`identity`] tells a file from another one at the same path, as the editing of Python
//!   does.
//! - [`probe`] says what a file holds: how long it lasts, its picture and its sound; [`gop`]
//!   how the frames of its picture are coded (key frames, B-frames).
//! - [`VideoDecoder`] gives any frame of its picture, and the frames after it one at a time,
//!   and the key frame at or before a frame, to be shown first while a frame is sought.
//! - [`AudioDecoder`] gives the samples of its sound from any place, converted to the rate and
//!   the channels that they are played with.
//!
//! # Times
//!
//! The duration of a file is counted in the time base of the stream that it is cut by: its
//! picture if it has one, else its sound. A time base of `num/den` seconds is written as the
//! scale `den`, and a count of `n` of its ticks as `n * num` ticks of that scale, so that no
//! place in the stream is lost to rounding (a time base is `1/den` in nearly every file, and
//! then a tick is a tick). A time counts from the start of the stream, its first frame, which
//! need not be at 0. So a place in a source that a client gives in the scale of the duration
//! of its media is a place in that stream, as the model requires ([`columbina_model::Clip`]).
//!
//! # Frames
//!
//! A frame of a picture is numbered by the rate of the picture ([`VideoFacts::rate`]): the
//! frame whose timestamp is `t` is `round((t - start) / ticks_per_frame)`, where `start` is the
//! start of the stream, or the timestamp of its first frame where the file does not say. The
//! rate is the one that the timestamps are laid on (`r_frame_rate`), unless the mean rate of the
//! frames (`avg_frame_rate`) is more than 1% from it: then it is the standard rate nearest to
//! that mean, as the editing of Python takes it (with 100, 119.88, 120 and 240 a second beside
//! its rates). Where frames come at changing times, a number may be had by none of them, and
//! then the frame shown there is the one before; or by two, and then
//! [`VideoDecoder::frame_at`] gives the first and [`VideoDecoder::next`] the second.
//!
//! # Rotation
//!
//! [`VideoFacts::rotation`] is how far a picture is turned **clockwise** to be shown upright,
//! as the model has it. ffprobe, and so the editing of Python, give the turn counterclockwise:
//! their 90 is 270 here, and a project of the editing of Python is moved over with
//! `(360 - r) % 360`.
//!
//! # Linking
//!
//! libav is linked dynamically: an LGPL shared build of FFmpeg 8.1 (avformat, avcodec, avutil,
//! swscale and swresample). To build, `FFMPEG_DIR` names the build on Windows (its `include`
//! and `lib`), and pkg-config finds it elsewhere; `LIBCLANG_PATH` names libclang for the
//! bindings. To run,
//! the libraries must be found by the system: on Windows the DLLs of `<FFMPEG_DIR>\bin` lie
//! beside the program, as they will once it is packaged, or that directory is on `PATH`. A test
//! of this workspace that cannot find them ends at once with `STATUS_DLL_NOT_FOUND`
//! (`0xc0000135`), before any test runs. Put the directory at the end of `PATH`, so that a
//! full `ffmpeg` found earlier on it still makes the media of the tests (see
//! `tests/support`):
//!
//! ```text
//! $env:PATH = "$env:PATH;$env:FFMPEG_DIR\bin"     # PowerShell
//! export PATH="$PATH:$FFMPEG_DIR/bin"              # Git Bash
//! ```
//!
//! libav writes nothing: its log is turned off before it is first used, since the engine's
//! stderr carries lines of JSON only.

mod audio;
mod decode;
mod error;
#[allow(unsafe_code)]
mod ffi;
mod identity;
mod probe;
mod timing;

pub use audio::{AudioDecoder, AudioFormat, Samples};
pub use columbina_model::{AudioFacts, Facts, Identity, Rate, Time, VideoFacts};
pub use decode::{video_start, Converter, Decoded, Hardware, HardwarePath, Picture, VideoDecoder};
pub use error::MediaError;
pub use identity::identity;
pub use probe::{gop, probe, Gop, DEMUXERS};

/// Make libav ready to be used, once: its log is turned off.
fn init() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        ffmpeg_next::util::log::set_level(ffmpeg_next::util::log::Level::Quiet);
    });
}
