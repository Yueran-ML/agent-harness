//! The pictures of a file: any frame of it, and the frames after it one at a time, decoded on
//! the graphics card where it can.

use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};

use columbina_model::Rate;
use ff::format::context::Input;
use ff::format::Pixel;
use ffmpeg_next as ff;

use crate::ffi::{self, Device, Planes, Scaler};
use crate::probe;
use crate::timing::Timing;
use crate::MediaError;

/// Whether a decoder may use the graphics card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hardware {
    /// The graphics card where it can decode the file, else the processor. On Windows CUDA is
    /// tried first (an NVIDIA card), then Direct3D 11; on macOS VideoToolbox; on Linux CUDA
    /// (an NVIDIA card), and no other device. Direct3D 12 is never tried: it cannot decode HEVC
    /// Main 10 at 4K on the drivers of NVIDIA of 2026.
    Auto,
    /// The processor only.
    Off,
}

/// What a decoder decodes with, as it is told in a log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HardwarePath {
    /// The graphics card, through CUDA.
    Cuda,
    /// The graphics card, through Direct3D 11.
    D3d11va,
    /// The graphics card, through VideoToolbox.
    VideoToolbox,
    /// The processor.
    Software,
}

impl HardwarePath {
    /// Its name, as libav names the device (`cuda`, `d3d11va`, `videotoolbox`), or `software`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Cuda => "cuda",
            Self::D3d11va => "d3d11va",
            Self::VideoToolbox => "videotoolbox",
            Self::Software => "software",
        }
    }
}

impl fmt::Display for HardwarePath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

/// The devices that [`Hardware::Auto`] tries, in order, before the processor.
const TRIED: &[HardwarePath] = if cfg!(windows) {
    &[HardwarePath::Cuda, HardwarePath::D3d11va]
} else if cfg!(target_os = "macos") {
    &[HardwarePath::VideoToolbox]
} else {
    &[HardwarePath::Cuda]
};

/// The frames that are kept on the card beyond those that the decoder needs: one waiting to be
/// given, the last one given, up to [`VideoDecoder::MOST_HELD`] held by whoever was given them,
/// and one to spare.
const EXTRA_FRAMES: i32 = 7;

/// How many frames on from the next one a decoder decodes its way to, rather than seek, in a
/// file whose key frames are not indexed, where it has decoded no key frame between them: one
/// that it has not decoded may lie between.
const FORWARD_UNINDEXED: i64 = 30;

/// How many frames or packets in a row may fail to decode, each passed over as ffmpeg passes
/// them over, before decoding is given up.
const MOST_FAILURES: u32 = 16;

/// How many times a decoder seeks for one frame, each time from further back, before it
/// decodes from the start of the file.
const MOST_SEEKS: u32 = 8;

/// A decoded frame in memory: 8 or 10 bit Y'CbCr 4:2:0, as NV12 (8 bits) or P010 (10 bits, in
/// the high bits of 16), in two planes: luma, then Cb and Cr interleaved at half the width and
/// half the height. Its colours are as the file has them: the viewer takes them as BT.709 of
/// limited range.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Picture {
    /// Its number, at the rate of its picture.
    pub frame: i64,
    /// Its width in pixels, as it is stored.
    pub width: u32,
    /// Its height in pixels, as it is stored.
    pub height: u32,
    /// 8 for NV12, 10 for P010.
    pub bits: u8,
    /// The plane of luma: `height` rows of `luma_stride` bytes.
    pub luma: Vec<u8>,
    /// The plane of chroma: `(height + 1) / 2` rows of `chroma_stride` bytes.
    pub chroma: Vec<u8>,
    /// The bytes from a row of luma to the next one.
    pub luma_stride: usize,
    /// The bytes from a row of chroma to the next one.
    pub chroma_stride: usize,
}

impl fmt::Debug for Picture {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Picture")
            .field("frame", &self.frame)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("bits", &self.bits)
            .field("luma_stride", &self.luma_stride)
            .field("chroma_stride", &self.chroma_stride)
            .finish_non_exhaustive()
    }
}

impl Picture {
    /// Shape the picture for a frame of `width` × `height` in 8 or 10 bits, and give its planes
    /// to be written. The memory that it holds is kept, and only grown.
    fn shape(&mut self, frame: i64, width: u32, height: u32, ten: bool) -> Planes<'_> {
        let bytes = if ten { 2 } else { 1 };
        // A row of chroma holds Cb and Cr of half the pixels, rounded up: at least a row of
        // luma. Rows start 64 bytes apart, as libav lays them.
        let stride = (width as usize).div_ceil(2) * 2 * bytes;
        let stride = stride.next_multiple_of(64);
        let rows = height as usize;
        self.frame = frame;
        self.width = width;
        self.height = height;
        self.bits = if ten { 10 } else { 8 };
        self.luma_stride = stride;
        self.chroma_stride = stride;
        // Memory of another size is asked for anew, already cleared by the system, rather than
        // cleared here a byte at a time.
        for (plane, length) in [
            (&mut self.luma, stride * rows),
            (&mut self.chroma, stride * rows.div_ceil(2)),
        ] {
            if plane.len() != length {
                *plane = vec![0; length];
            }
        }
        Planes {
            luma: &mut self.luma,
            luma_stride: stride,
            chroma: &mut self.chroma,
            chroma_stride: stride,
        }
    }
}

/// A frame as the decoder gave it, on the graphics card or in memory, before it is made a
/// [`Picture`] by a [`Converter`]. It may be sent to another thread to be made one there while
/// the decoder decodes the next: copying a frame of 4K from the card takes 5 to 12 ms.
pub struct Decoded {
    frame: i64,
    raw: ff::frame::Video,
    /// Whether the frame that came out before it has the same number, so that it is not the
    /// frame shown at its number.
    repeat: bool,
}

impl Decoded {
    /// Its number, at the rate of its picture.
    #[must_use]
    pub fn frame(&self) -> i64 {
        self.frame
    }

    /// Whether it is on the graphics card.
    #[must_use]
    pub fn on_device(&self) -> bool {
        ffi::on_device(self.raw.format())
    }

    /// Another of the same frame, which shares its memory.
    fn again(&self) -> Result<Self, MediaError> {
        Ok(Self {
            frame: self.frame,
            raw: ffi::reference(&self.raw).map_err(MediaError::Decode)?,
            repeat: self.repeat,
        })
    }
}

/// What makes [`Decoded`] frames into [`Picture`]s: it copies them from the graphics card, and
/// converts those of other formats (4:2:2, 4:4:4, RGB, 12 bits) with swscale, to P010 if they
/// have more than 8 bits and NV12 if not, writing straight into the picture. It keeps what it
/// made for the last frame, so that the next one of the same shape is made without starting
/// again; it takes frames of any file, one after another. One is used by one thread at a time,
/// and each thread that makes pictures has its own.
///
/// A converter that is not a decoder's own cannot tell the decoder that the card failed: where
/// it gives [`MediaError::Device`], the caller tells the decoder with
/// [`VideoDecoder::device_failed`] and asks it for the frame again with
/// [`VideoDecoder::decoded_at`], and the decoder goes on on the processor.
#[derive(Default)]
pub struct Converter {
    /// A frame copied from the card in a format other than NV12 and P010, to be converted.
    copied: Option<ff::frame::Video>,
    scaler: Option<Scaler>,
}

impl Converter {
    /// A converter that has made nothing yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Make a frame a picture.
    ///
    /// # Errors
    /// It cannot be copied from the card ([`MediaError::Device`]), or converted.
    pub fn picture(&mut self, decoded: &Decoded) -> Result<Picture, MediaError> {
        let mut picture = Picture::default();
        self.picture_into(decoded, &mut picture)?;
        Ok(picture)
    }

    /// Make a frame a picture in the place of `into`, whose planes are written over and kept:
    /// so that the memory of a picture that was shown is used for the next, and no new memory
    /// is touched for each frame (a picture of 4K in P010 is 25 MB).
    ///
    /// # Errors
    /// As [`Converter::picture`]; `into` is then written over in part.
    pub fn picture_into(
        &mut self,
        decoded: &Decoded,
        into: &mut Picture,
    ) -> Result<(), MediaError> {
        let raw = &decoded.raw;
        if !ffi::on_device(raw.format()) {
            return convert(&mut self.scaler, raw, decoded.frame, into);
        }
        let memory = ffi::memory_format(raw)
            .ok_or_else(|| MediaError::Device("a frame on the card has no format".into()))?;
        if matches!(memory, Pixel::NV12 | Pixel::P010LE) {
            let mut planes = into.shape(
                decoded.frame,
                raw.width(),
                raw.height(),
                memory == Pixel::P010LE,
            );
            return ffi::transfer_into(raw, &mut planes).map_err(MediaError::Device);
        }
        let copied = self.copied.get_or_insert_with(ff::frame::Video::empty);
        ffi::transfer(copied, raw).map_err(MediaError::Device)?;
        convert(&mut self.scaler, copied, decoded.frame, into)
    }
}

/// Write a frame in memory into a picture: copied if it is NV12 or P010, else converted to the
/// one of them with as many bits, with the scaler that was kept if it was made for frames of
/// this format and size.
fn convert(
    scaler: &mut Option<Scaler>,
    source: &ff::frame::Video,
    frame: i64,
    into: &mut Picture,
) -> Result<(), MediaError> {
    let format = source.format();
    let (width, height) = (source.width(), source.height());
    let ten = ffi::depth(format).unwrap_or(8) > 8;
    if matches!(format, Pixel::NV12 | Pixel::P010LE) {
        let planes = into.shape(frame, width, height, format == Pixel::P010LE);
        copy_planes(source, planes);
        return Ok(());
    }
    let scaler = match scaler {
        Some(scaler)
            if {
                let input = scaler.0.input();
                (input.format, input.width, input.height) == (format, width, height)
            } =>
        {
            scaler
        }
        slot => {
            let made = ff::software::scaling::Context::get(
                format,
                width,
                height,
                if ten { Pixel::P010LE } else { Pixel::NV12 },
                width,
                height,
                ff::software::scaling::Flags::BILINEAR,
            )
            .map_err(|error| {
                MediaError::Decode(format!(
                    "frames of {} cannot be converted: {error}",
                    format
                        .descriptor()
                        .map_or("a format", |described| described.name())
                ))
            })?;
            slot.insert(Scaler(made))
        }
    };
    let mut planes = into.shape(frame, width, height, ten);
    ffi::scale_into(scaler, source, &mut planes).map_err(MediaError::Decode)
}

/// Copy the planes of a frame of NV12 or P010 into planes of the same shape, a row at a time.
fn copy_planes(source: &ff::frame::Video, planes: Planes<'_>) {
    let bytes = if source.format() == Pixel::P010LE {
        2
    } else {
        1
    };
    let width = source.width() as usize;
    let height = source.height() as usize;
    for (plane, into, stride, rows, row) in [
        (0, planes.luma, planes.luma_stride, height, width * bytes),
        (
            1,
            planes.chroma,
            planes.chroma_stride,
            height.div_ceil(2),
            width.div_ceil(2) * 2 * bytes,
        ),
    ] {
        let (data, from_stride) = (source.data(plane), source.stride(plane));
        for y in 0..rows {
            into[y * stride..y * stride + row]
                .copy_from_slice(&data[y * from_stride..y * from_stride + row]);
        }
    }
}

/// The decoder of the picture of a file.
///
/// It gives the frame at any number ([`VideoDecoder::frame_at`]), and the frames after the last
/// one it gave ([`VideoDecoder::next`]). To give a frame further on it decodes on from where it
/// is, where the key frame at or before that frame is not after the frame it is at, since
/// decoding from there is what seeking would do; else it seeks to that key frame and decodes on
/// from it. It knows key frames from the index of the file, where the file is of a format
/// whose index holds them all when it is opened (MP4, Matroska, AVI), and from those it has
/// decoded. A file of MPEG-TS or MPEG-PS is sought in by time, with no index of its key frames:
/// it is sought from a key frame that was decoded, where one is known near enough, else from
/// further and further back until the key frame found is not after the frame; its start is
/// found by seeking to its first byte.
///
/// A frame or a packet that fails to decode is passed over, as ffmpeg passes it over, unless
/// 16 fail in a row. A decoder on the graphics card that fails so, or whose
/// frames cannot be copied from the card, is opened again on the processor where it was, and
/// goes on there ([`VideoDecoder::fallbacks`] tells it): the card never breaks a caller.
///
/// What it gives is decoded: it is a [`Decoded`] from [`VideoDecoder::decoded_at`] and
/// [`VideoDecoder::next_decoded`], and a [`Picture`] from the others, which make it one with a
/// [`Converter`] of the decoder. Making it one on another thread while the decoder goes on is
/// left to whoever plays the file.
#[allow(clippy::struct_excessive_bools)]
pub struct VideoDecoder {
    source: PathBuf,
    input: Input,
    stream: usize,
    decoder: ff::decoder::Video,
    /// The device that the decoder decodes on, kept as long as the decoder is.
    _device: Option<Device>,
    path: HardwarePath,
    fallbacks: Vec<(HardwarePath, String)>,
    timing: Timing,
    /// Whether the index of the file holds every key frame from when it is opened.
    indexed: bool,
    /// Whether the start of the file is sought by its first byte.
    start_by_byte: bool,
    /// The number of the first frame of the file.
    first_frame: i64,
    /// The last frame that was given.
    given: Option<i64>,
    /// Another of the last frame that was given, to give it again without decoding.
    shown: Option<Decoded>,
    /// A frame that was decoded and not given yet: the next to be given.
    waiting: Option<Decoded>,
    /// Whether the decoder has given every frame up to the end of the file.
    at_end: bool,
    /// Whether the end of the file was sent to the decoder.
    sent_end: bool,
    /// The number of the last frame of the file, once it is known.
    last: Option<i64>,
    /// The key frames that were decoded.
    key_frames: BTreeSet<i64>,
    /// The last key frame that came out since the decoder last sought.
    last_key: Option<i64>,
    /// The most frames from a key frame to the next one, of two that came out one after the
    /// other.
    longest_group: Option<i64>,
    /// How much later a key frame is shown than it is decoded, in ticks, as its packets say: an
    /// index of MP4, and the seeking of MPEG-TS, go by the time a frame is decoded, a few frames
    /// before it is shown where frames are reordered.
    key_shift: i64,
    /// Until the first frame comes out after the decoder was emptied, the timestamps of the
    /// packets sent to it since, so that a frame that is none of them, left from before, is
    /// dropped. None once one came out, or after a packet without a timestamp.
    checking: Option<BTreeSet<i64>>,
    /// The number of the last frame that came out, for a frame without a timestamp.
    previous: Option<i64>,
    /// How many frames or packets in a row failed to decode.
    failures: u32,
    seeks: u64,
    /// Whether the decoder on the card is made to fail, by a test.
    failing: bool,
    /// A failure of the card that a caller saw elsewhere, as when a [`Converter`] of its own
    /// could not copy a frame: the next call goes on on the processor.
    failed_elsewhere: Option<String>,
    converter: Converter,
}

impl VideoDecoder {
    /// How many [`Decoded`] frames that are on the graphics card may be held at once, beside
    /// those that a decoder holds. A decoder that is given none back cannot decode on.
    pub const MOST_HELD: usize = 4;

    /// Open the picture of a file, with the graphics card or without it, and decode its first
    /// frame, which [`VideoDecoder::next`] gives first.
    ///
    /// With [`Hardware::Auto`] each device is tried in turn: it is made, a decoder is set up on
    /// it, and the first frame is decoded and copied from it. If any of these fails, the next
    /// device is tried, and at last the processor; [`VideoDecoder::fallbacks`] says why each
    /// failed, and [`VideoDecoder::path_used`] what was used. Hardware that fails is never an
    /// error.
    ///
    /// # Errors
    /// The file cannot be opened (as [`crate::probe`]), it has no picture, its picture cannot be
    /// decoded by this build of libav, or its first frame cannot be decoded.
    pub fn open(path: &Path, hardware: Hardware) -> Result<Self, MediaError> {
        let devices = match hardware {
            Hardware::Auto => TRIED,
            Hardware::Off => &[],
        };
        Self::open_trying(path, devices)
    }

    /// As [`VideoDecoder::open`], trying `devices` in turn before the processor: so that the
    /// tests and measurements can try a device that [`Hardware::Auto`] would not reach.
    ///
    /// # Errors
    /// As [`VideoDecoder::open`].
    #[doc(hidden)]
    pub fn open_trying(path: &Path, devices: &[HardwarePath]) -> Result<Self, MediaError> {
        let mut fallbacks = Vec::new();
        for &device in devices {
            if device == HardwarePath::Software {
                break;
            }
            match Self::open_on(path, Some(device)) {
                Ok(mut decoder) => {
                    decoder.fallbacks = fallbacks;
                    return Ok(decoder);
                }
                Err(Attempt::Hardware(why)) => fallbacks.push((device, why)),
                Err(Attempt::File(error)) => return Err(error),
            }
        }
        let mut decoder = Self::open_on(path, None).map_err(Attempt::into_error)?;
        decoder.fallbacks = fallbacks;
        Ok(decoder)
    }

    /// Tell the decoder that its card failed where it could not see it: a [`Converter`] of the
    /// caller's own, on another thread, gave [`MediaError::Device`] for a frame of it. The next
    /// call opens the file again on the processor, where it was, and goes on there; ask it for
    /// the frame that failed again with [`VideoDecoder::decoded_at`]. Nothing is done if it
    /// decodes on the processor already.
    pub fn device_failed(&mut self, why: &str) {
        if self.path != HardwarePath::Software {
            self.failed_elsewhere = Some(why.to_owned());
        }
    }

    /// Make the decoder on the graphics card fail from now on, as a card that is lost does, so
    /// that the tests can see it go on on the processor. Nothing on the processor fails.
    #[doc(hidden)]
    pub fn fail_device(&mut self) {
        self.failing = true;
    }

    /// Open the picture of a file on a device, or on the processor if there is none.
    fn open_on(path: &Path, device: Option<HardwarePath>) -> Result<Self, Attempt> {
        let mut input = probe::open(path)?;
        let stream = probe::video_stream(&input)?.ok_or(MediaError::NoVideo)?;
        let (timing, start, mut context) = {
            let found = input.stream(stream).ok_or(MediaError::NoVideo)?;
            let base = probe::base(&found)?;
            let rate = probe::rate(&found).ok_or_else(|| {
                MediaError::Unsupported("its picture has no rate of frames".into())
            })?;
            let start = Some(found.start_time()).filter(|&start| start != ff::ffi::AV_NOPTS_VALUE);
            let timing = Timing {
                base,
                rate,
                origin: start.unwrap_or(0),
            };
            let context = ff::codec::Context::from_parameters(found.parameters())
                .map_err(|error| MediaError::Open(error.to_string()))?;
            (timing, start, context)
        };
        let codec = context.id();
        let unsupported = || {
            MediaError::Unsupported(format!(
                "this build of libav cannot decode {}",
                codec.name()
            ))
        };
        let mut decoder_of = ff::decoder::find(codec).ok_or_else(unsupported)?;
        let hardware = if let Some(device) = device {
            // libdav1d, which libav prefers for AV1, decodes on no card; its own decoder does.
            if codec == ff::codec::Id::AV1 {
                decoder_of = ff::decoder::find_by_name("av1").unwrap_or(decoder_of);
            }
            let made = Device::new(device.name()).map_err(Attempt::Hardware)?;
            let format = made.format_of(&decoder_of).ok_or_else(|| {
                Attempt::Hardware(format!("{} is not decoded on {device}", codec.name()))
            })?;
            made.attach(&mut context, format, EXTRA_FRAMES)
                .map_err(Attempt::Hardware)?;
            // A decoder on a card decodes one frame at a time; threads only wait on it.
            ffi::set_threads(&mut context, 1, false);
            Some((made, format))
        } else {
            ffi::set_threads(&mut context, 0, true);
            None
        };
        let opened = context
            .decoder()
            .open_as(decoder_of)
            .and_then(ff::decoder::Opened::video)
            .map_err(|error| {
                let why = format!("the decoder of {} cannot be opened: {error}", codec.name());
                if device.is_some() {
                    Attempt::Hardware(why)
                } else {
                    Attempt::File(MediaError::Decode(why))
                }
            })?;
        ffi::discard_others(&mut input, stream);
        let format_name = input.format().name().to_owned();
        let indexed = ["mov", "matroska", "avi"]
            .iter()
            .any(|name| format_name.split(',').any(|part| part == *name))
            && ffi::index_entries(&input, stream) > 1;
        let start_by_byte = ["mpegts", "mpeg"].contains(&format_name.as_str());
        let (device_kept, format) = hardware.unzip();
        let mut decoder = Self {
            source: path.to_owned(),
            input,
            stream,
            decoder: opened,
            _device: device_kept,
            path: device.unwrap_or(HardwarePath::Software),
            fallbacks: Vec::new(),
            timing,
            indexed,
            start_by_byte,
            first_frame: 0,
            given: None,
            shown: None,
            waiting: None,
            at_end: false,
            sent_end: false,
            last: None,
            key_frames: BTreeSet::new(),
            last_key: None,
            longest_group: None,
            key_shift: 0,
            checking: Some(BTreeSet::new()),
            previous: None,
            failures: 0,
            seeks: 0,
            failing: false,
            failed_elsewhere: None,
            converter: Converter::new(),
        };
        decoder.decode_first(format, start.is_none())?;
        Ok(decoder)
    }

    /// Decode the first frame, to be given first: it tells whether the device decodes the file,
    /// and, where the file does not say where its frames start (`unstarted`), where they do.
    /// `format` is that of the frames on the device, if there is one.
    fn decode_first(&mut self, format: Option<Pixel>, unstarted: bool) -> Result<(), Attempt> {
        let on_device = format.is_some();
        let first = match self.receive() {
            Ok(Some(first)) => first,
            Ok(None) => {
                let why = "it has no frame that can be decoded".to_owned();
                return Err(if on_device {
                    Attempt::Hardware(why)
                } else {
                    Attempt::File(MediaError::Decode(why))
                });
            }
            Err(error) if on_device => return Err(Attempt::Hardware(error.to_string())),
            Err(error) => return Err(Attempt::File(error)),
        };
        if let Some(format) = format {
            if first.raw.format() != format {
                return Err(Attempt::Hardware(format!(
                    "{} gave its first frame in memory, not on the card",
                    self.path
                )));
            }
            let mut copied = ff::frame::Video::empty();
            ffi::transfer(&mut copied, &first.raw).map_err(Attempt::Hardware)?;
        }
        let first = if unstarted {
            // Frame 0 is the first frame.
            self.timing.origin = first.raw.timestamp().or(first.raw.pts()).unwrap_or(0);
            self.key_frames.clear();
            self.last_key = None;
            if first.raw.is_key() {
                self.learn_key(0);
            }
            self.previous = Some(0);
            Decoded {
                frame: 0,
                raw: first.raw,
                repeat: false,
            }
        } else {
            first
        };
        self.first_frame = first.frame;
        self.waiting = Some(first);
        Ok(())
    }

    /// The frame that is shown at `frame` of the picture's own rate: the one numbered so, or,
    /// where none is (a file whose frames come at changing times), the last one before it.
    /// Before the first frame of a file that starts late, its first frame.
    ///
    /// Where frames come at changing times, two frames may be numbered alike, by rounding: this
    /// gives the first of them, and [`VideoDecoder::next`] after it the second, with the same
    /// number.
    ///
    /// # Errors
    /// `frame` is below 0 or after the last frame ([`MediaError::OutOfRange`]: a frame past the
    /// end is never given as the last one), or a frame cannot be decoded or copied. After an
    /// error, [`VideoDecoder::next`] gives nothing until a frame is asked for again.
    pub fn frame_at(&mut self, frame: i64) -> Result<Picture, MediaError> {
        self.or_software(false, |decoder| {
            let found = decoder.find(frame)?;
            decoder.converter.picture(&found)
        })
    }

    /// The frame after the last one given, without seeking; the first frame of the file if
    /// none was given. None at the end of the file.
    ///
    /// # Errors
    /// A frame cannot be decoded or copied.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<Option<Picture>, MediaError> {
        self.or_software(true, |decoder| match decoder.advance()? {
            Some(decoded) => decoder.converter.picture(&decoded).map(Some),
            None => Ok(None),
        })
    }

    /// What decodes, for the log: the device, or the processor where none was used or the one
    /// that was used failed.
    #[must_use]
    pub fn path_used(&self) -> HardwarePath {
        self.path
    }

    /// Each device that was tried before the one that is used, or that was used and failed, and
    /// why it was not used.
    #[must_use]
    pub fn fallbacks(&self) -> &[(HardwarePath, String)] {
        &self.fallbacks
    }

    /// The rate that the frames are numbered at.
    #[must_use]
    pub fn rate(&self) -> Rate {
        self.timing.rate
    }

    /// How many times the decoder has sought in the file since it was opened.
    #[must_use]
    pub fn seeks(&self) -> u64 {
        self.seeks
    }

    /// As [`VideoDecoder::frame_at`], but the frame as it was decoded, to be made a picture by
    /// a [`Converter`] of the caller's own. If that converter fails with [`MediaError::Device`],
    /// the decoder does not know: tell it with [`VideoDecoder::device_failed`], and ask for the
    /// frame again here, and it is decoded on the processor.
    ///
    /// # Errors
    /// As [`VideoDecoder::frame_at`].
    pub fn decoded_at(&mut self, frame: i64) -> Result<Decoded, MediaError> {
        self.or_software(false, |decoder| decoder.find(frame))
    }

    /// As [`VideoDecoder::next`], but the frame as it was decoded.
    ///
    /// # Errors
    /// As [`VideoDecoder::next`].
    pub fn next_decoded(&mut self) -> Result<Option<Decoded>, MediaError> {
        self.or_software(true, Self::advance)
    }

    /// Do `work`, and, if the card failed at it, open the file again on the processor and do it
    /// again there. Where `resume`, the decoder on the processor is first brought to where the
    /// one on the card was before `work`, so that the next frame is the one that failed.
    fn or_software<T>(
        &mut self,
        resume: bool,
        mut work: impl FnMut(&mut Self) -> Result<T, MediaError>,
    ) -> Result<T, MediaError> {
        let before = self.given;
        if let Some(why) = self.failed_elsewhere.take() {
            if self.path != HardwarePath::Software {
                self.fall_back(&MediaError::Device(why), resume, before)?;
            }
        }
        match work(self) {
            Err(error @ (MediaError::Device(_) | MediaError::Decode(_)))
                if self.path != HardwarePath::Software =>
            {
                self.fall_back(&error, resume, before)?;
                work(self)
            }
            done => done,
        }
    }

    /// Open the file again on the processor, in the place of this decoder on a card, which
    /// failed with `error`. Where `resume`, bring it to the frame after `given`, or to the first
    /// frame if that is none.
    fn fall_back(
        &mut self,
        error: &MediaError,
        resume: bool,
        given: Option<i64>,
    ) -> Result<(), MediaError> {
        let mut software = Self::open_on(&self.source, None).map_err(Attempt::into_error)?;
        let mut fallbacks = std::mem::take(&mut self.fallbacks);
        fallbacks.push((self.path, error.to_string()));
        software.fallbacks = fallbacks;
        software.seeks = self.seeks;
        software.last = self.last;
        *self = software;
        if let (true, Some(given)) = (resume, given) {
            self.find(given)?;
        }
        Ok(())
    }

    /// The frame shown at `frame`, from the last one given, by decoding on, or by seeking.
    fn find(&mut self, frame: i64) -> Result<Decoded, MediaError> {
        if frame < 0 || self.last.is_some_and(|last| frame > last) {
            return Err(MediaError::OutOfRange {
                frame,
                last: self.last,
            });
        }
        if let Some(shown) = &self.shown {
            // The frame given last is shown up to the next one: at its own number unless it is
            // the second of two numbered alike, and at the numbers after it that no frame has.
            let up_to = self.waiting.as_ref().map(|waiting| waiting.frame - 1);
            if self.given == Some(shown.frame)
                && ((shown.frame == frame && !shown.repeat)
                    || (shown.frame < frame && up_to >= Some(frame)))
            {
                return shown.again();
            }
        }
        if self.forward_to(frame) {
            let current = match self.waiting.take() {
                Some(waiting) => waiting,
                None => match self.receive()? {
                    Some(decoded) => decoded,
                    None => return Err(self.ended(frame, None)),
                },
            };
            if current.frame > frame {
                // No frame is numbered between the last one given and the next: the last one
                // given is shown at the frame.
                self.waiting = Some(current);
                if let Some(shown) = self
                    .shown
                    .as_ref()
                    .filter(|shown| self.given == Some(shown.frame))
                {
                    return shown.again();
                }
                return self.seek_to(frame);
            }
            return self.settle(frame, current);
        }
        self.seek_to(frame)
    }

    /// The next frame after the last one given, without seeking.
    fn advance(&mut self) -> Result<Option<Decoded>, MediaError> {
        let next = match self.waiting.take() {
            Some(waiting) => Some(waiting),
            None if self.at_end => None,
            None => self.receive()?,
        };
        if let Some(decoded) = next {
            return self.give(decoded).map(Some);
        }
        self.at_end = true;
        if let Some(given) = self.given {
            self.last = Some(given);
        }
        Ok(None)
    }

    /// Give a frame: it is the last one given, kept to be given again.
    fn give(&mut self, decoded: Decoded) -> Result<Decoded, MediaError> {
        self.given = Some(decoded.frame);
        self.shown = Some(decoded.again()?);
        Ok(decoded)
    }

    /// Whether `frame` is best reached by decoding on from where the decoder is: it is not
    /// before the next frame to come, and the key frame at or before it is not after that frame.
    fn forward_to(&self, frame: i64) -> bool {
        let next = match &self.waiting {
            Some(waiting) => waiting.frame,
            None if self.at_end => return false,
            None => match self.given {
                Some(given) => given + 1,
                None => return false,
            },
        };
        if frame < next {
            return false;
        }
        let learned = self.key_frames.range(..=frame).next_back().copied();
        let indexed = if self.indexed {
            // The last key frame shown at or before the frame is decoded at or before this.
            let decoded_by = self
                .timing
                .timestamp_of(frame)
                .saturating_sub(self.key_shift);
            ffi::key_frame_before(&self.input, self.stream, decoded_by).map(|timestamp| {
                self.timing
                    .frame_of(timestamp.saturating_add(self.key_shift))
            })
        } else {
            None
        };
        let key_before = learned.max(indexed).is_none_or(|key| key <= next);
        key_before && (self.indexed || frame - next <= FORWARD_UNINDEXED)
    }

    /// Seek to a key frame at or before `frame`, and decode on to it.
    ///
    /// It seeks from a key frame that was decoded where one is known within the longest group
    /// of frames before `frame`, else from `frame` itself; where the first frame that comes out
    /// is after `frame`, the key frame found was after it, and it seeks again from further
    /// back. From the first frame back, or after [`MOST_SEEKS`], it seeks to the start.
    fn seek_to(&mut self, frame: i64) -> Result<Decoded, MediaError> {
        let known = self
            .key_frames
            .range(..=frame)
            .next_back()
            .copied()
            .filter(|&key| self.longest_group.is_some_and(|group| frame - key < group));
        let mut from = known.unwrap_or(frame);
        let mut step = 0_i64;
        for _ in 0..MOST_SEEKS {
            if from <= self.first_frame {
                break;
            }
            // Seeking in a file whose index is not trusted goes by the time a frame is decoded.
            let shift = if self.indexed { 0 } else { self.key_shift };
            let at = self.timing.timestamp_of(from).saturating_sub(shift);
            self.seeks += 1;
            if ffi::seek(&mut self.input, self.stream, at, true).is_err() {
                break;
            }
            self.emptied();
            if let Some(first) = self.receive()? {
                if first.frame <= frame {
                    return self.settle(frame, first);
                }
            }
            step = step
                .saturating_mul(2)
                .max(8)
                .max(self.longest_group.map_or(0, |group| group + 1));
            from = from.saturating_sub(step);
        }
        self.seek_start(frame)
    }

    /// Seek to the start of the file, and decode on to `frame`: by its first byte, for MPEG-TS
    /// and MPEG-PS, whose seeking by time finds no key frame; else to the key frame at or
    /// before its first frame, or the first key frame after that.
    fn seek_start(&mut self, frame: i64) -> Result<Decoded, MediaError> {
        self.seeks += 1;
        let sought = if self.start_by_byte {
            ffi::seek_to_byte(&mut self.input, 0)
        } else {
            let at = self.timing.timestamp_of(self.first_frame);
            ffi::seek(&mut self.input, self.stream, at, true)
                .or_else(|_| ffi::seek(&mut self.input, self.stream, at, false))
        };
        sought.map_err(MediaError::Seek)?;
        self.emptied();
        let Some(first) = self.receive()? else {
            return Err(self.ended(frame, None));
        };
        if first.frame > frame.max(self.first_frame) {
            return Err(MediaError::Seek(format!(
                "seeking to the start of the file found frame {}, not {}",
                first.frame, self.first_frame
            )));
        }
        self.settle(frame, first)
    }

    /// Empty the decoder after a seek, and forget where it was.
    fn emptied(&mut self) {
        self.decoder.flush();
        self.waiting = None;
        self.given = None;
        self.at_end = false;
        self.sent_end = false;
        self.checking = Some(BTreeSet::new());
        self.previous = None;
        self.last_key = None;
        self.failures = 0;
    }

    /// Decode on from `current`, a frame at or before `frame` (or the first after a seek to the
    /// start), to the frame that is shown at `frame`, and give it.
    fn settle(&mut self, frame: i64, mut current: Decoded) -> Result<Decoded, MediaError> {
        while current.frame < frame {
            match self.receive()? {
                Some(next) if next.frame <= frame => current = next,
                Some(next) => {
                    self.waiting = Some(next);
                    break;
                }
                None => return Err(self.ended(frame, Some(current.frame))),
            }
        }
        self.give(current)
    }

    /// The error of a frame after the end of the file, which the decoder has reached; `last` is
    /// the last frame, if it came out since the decoder last sought.
    fn ended(&mut self, frame: i64, last: Option<i64>) -> MediaError {
        if let Some(last) = last.or(self.given) {
            self.last = Some(last);
        }
        self.at_end = true;
        self.given = None;
        self.waiting = None;
        MediaError::OutOfRange {
            frame,
            last: self.last,
        }
    }

    /// The next frame that comes out of the decoder, and its number; none at the end.
    fn receive(&mut self) -> Result<Option<Decoded>, MediaError> {
        loop {
            if self.failing && self.path != HardwarePath::Software {
                return Err(MediaError::Device(format!(
                    "{} was made to fail",
                    self.path
                )));
            }
            let mut raw = ff::frame::Video::empty();
            match self.decoder.receive_frame(&mut raw) {
                Ok(()) => {
                    if !self.belongs(&raw) {
                        continue;
                    }
                    self.failures = 0;
                    let frame = match raw.timestamp().or(raw.pts()) {
                        Some(timestamp) => self.timing.frame_of(timestamp),
                        None => self.previous.map_or(0, |previous| previous + 1),
                    };
                    let repeat = self.previous == Some(frame);
                    self.previous = Some(frame);
                    if raw.is_key() {
                        self.learn_key(frame);
                    }
                    return Ok(Some(Decoded { frame, raw, repeat }));
                }
                Err(ff::Error::Other { errno }) if errno == ff::util::error::EAGAIN => {
                    self.feed()?;
                }
                Err(ff::Error::Eof) => return Ok(None),
                Err(error) => self.failed(error)?,
            }
        }
    }

    /// Count a key frame that came out, and the group of frames that it ends.
    fn learn_key(&mut self, frame: i64) {
        self.key_frames.insert(frame);
        if let Some(last) = self.last_key.filter(|&last| last < frame) {
            let group = frame - last;
            self.longest_group = Some(self.longest_group.map_or(group, |most| most.max(group)));
        }
        self.last_key = Some(frame);
    }

    /// Pass over a frame or a packet that failed to decode, unless too many failed in a row.
    fn failed(&mut self, error: ff::Error) -> Result<(), MediaError> {
        self.failures += 1;
        if self.failures <= MOST_FAILURES {
            return Ok(());
        }
        let why = format!("{MOST_FAILURES} frames in a row failed, the last with: {error}");
        Err(if self.path == HardwarePath::Software {
            MediaError::Decode(why)
        } else {
            MediaError::Device(format!("{}: {why}", self.path))
        })
    }

    /// Whether a frame that came out may be given: the first after the decoder was emptied must
    /// be of a packet that was sent since, since a decoder on a card may give a frame from
    /// before a seek after it. Those after it are all given, those with the timestamp of
    /// another too, as a file may have.
    fn belongs(&mut self, raw: &ff::frame::Video) -> bool {
        if let (Some(sent), Some(pts)) = (&self.checking, raw.pts()) {
            if !sent.contains(&pts) {
                return false;
            }
        }
        self.checking = None;
        true
    }

    /// Send the decoder the next packet of the picture, or the end of the file.
    fn feed(&mut self) -> Result<(), MediaError> {
        if self.sent_end {
            return Err(MediaError::Decode(
                "the decoder wants more after the end of the file".into(),
            ));
        }
        loop {
            let mut packet = ff::Packet::empty();
            match packet.read(&mut self.input) {
                Ok(()) if packet.stream() == self.stream => {
                    if let (true, Some(pts), Some(dts)) =
                        (packet.is_key(), packet.pts(), packet.dts())
                    {
                        self.key_shift = self.key_shift.max(pts.saturating_sub(dts));
                    }
                    match packet.pts() {
                        Some(pts) => {
                            if let Some(sent) = &mut self.checking {
                                sent.insert(pts);
                            }
                        }
                        None => self.checking = None,
                    }
                    match self.decoder.send_packet(&packet) {
                        Ok(()) => return Ok(()),
                        // A packet that cannot be decoded is passed over, as ffmpeg does.
                        Err(error) => self.failed(error)?,
                    }
                }
                Ok(()) => {}
                Err(ff::Error::Eof) => {
                    self.sent_end = true;
                    return self
                        .decoder
                        .send_eof()
                        .map_err(|error| MediaError::Decode(error.to_string()));
                }
                Err(error) => {
                    return Err(MediaError::Read(format!(
                        "the file cannot be read on: {error}"
                    )))
                }
            }
        }
    }
}

/// Why a decoder could not be opened on a device: the device, so that the next is tried, or
/// the file, so that none is.
enum Attempt {
    Hardware(String),
    File(MediaError),
}

impl Attempt {
    /// The error of an attempt on the processor, where no device was tried.
    fn into_error(self) -> MediaError {
        match self {
            Self::Hardware(why) => MediaError::Decode(why),
            Self::File(error) => error,
        }
    }
}

impl From<MediaError> for Attempt {
    fn from(error: MediaError) -> Self {
        Self::File(error)
    }
}
