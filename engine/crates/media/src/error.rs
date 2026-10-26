//! Why a file cannot be read, or a frame of it cannot be given.

/// Why a file cannot be read, or a frame of it cannot be given. Nothing of this crate panics on
/// a file, whatever is in it.
#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    /// Its path is not UTF-8, which a project cannot hold, or it holds a NUL.
    #[error("its path is not UTF-8")]
    NotUtf8,
    /// It cannot be read: it is not there, or it may not be read, or it is not a file.
    #[error("it cannot be read: {0}")]
    Io(#[from] std::io::Error),
    /// libav cannot open it as media: it is not media, or it is broken.
    #[error("it cannot be opened as media: {0}")]
    Open(String),
    /// It holds neither picture nor sound.
    #[error("it holds neither picture nor sound")]
    NoStreams,
    /// It has no picture, and a picture of it was asked for.
    #[error("it has no picture")]
    NoVideo,
    /// What it holds cannot be used: no duration, a picture with no rate or size, a codec that
    /// this build of libav cannot decode.
    #[error("{0}")]
    Unsupported(String),
    /// A frame of it cannot be decoded.
    #[error("a frame cannot be decoded: {0}")]
    Decode(String),
    /// The graphics card failed: frames on it failed to decode, one after another, or a frame
    /// cannot be copied from it. A [`crate::VideoDecoder`] goes on on the processor instead,
    /// and gives this only from a [`crate::Converter`] of the caller's own.
    #[error("the graphics card failed: {0}")]
    Device(String),
    /// The file cannot be read on, after it was opened.
    #[error("{0}")]
    Read(String),
    /// A place in it cannot be sought.
    #[error("{0}")]
    Seek(String),
    /// There is no such frame: it is before the first frame or after the last one. `last` is the
    /// last frame, where it is known.
    #[error("there is no frame {frame}{}", last.map(|last| format!(": the last is {last}")).unwrap_or_default())]
    OutOfRange {
        /// The frame that was asked for.
        frame: i64,
        /// The number of the last frame, if it is known.
        last: Option<i64>,
    },
}

impl MediaError {
    /// What went wrong, for a program to read: `not_utf8`, `not_found`, `unreadable`,
    /// `not_media`, `no_streams`, `no_video`, `unsupported`, `decode`, `device`, `read`, `seek`
    /// or `out_of_range`.
    #[must_use]
    pub fn reason(&self) -> &'static str {
        match self {
            Self::NotUtf8 => "not_utf8",
            Self::Io(error) if error.kind() == std::io::ErrorKind::NotFound => "not_found",
            Self::Io(_) => "unreadable",
            Self::Open(_) => "not_media",
            Self::NoStreams => "no_streams",
            Self::NoVideo => "no_video",
            Self::Unsupported(_) => "unsupported",
            Self::Decode(_) => "decode",
            Self::Device(_) => "device",
            Self::Read(_) => "read",
            Self::Seek(_) => "seek",
            Self::OutOfRange { .. } => "out_of_range",
        }
    }
}
