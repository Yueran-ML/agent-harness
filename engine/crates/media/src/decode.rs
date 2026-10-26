//! The pictures of a file: any frame of it, and the frames after it one at a time, decoded on
//! the graphics card where it can.

use std::collections::BTreeSet;
use std::fmt;
use std::path::Path;

use columbina_model::Rate;
use ff::format::context::Input;
use ff::format::Pixel;
use ffmpeg_next as ff;

use crate::ffi::{self, Device, Scaler};
use crate::probe;
use crate::timing::Timing;
use crate::MediaError;

/// Whether a decoder may use the graphics card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hardware {
    /// The graphics card where it can decode the file, else the processor. On Windows CUDA is
    /// tried first (an NVIDIA card), then Direct3D 11; on macOS VideoToolbox; on Linux CUDA.
    /// Direct3D 12 is never tried: it cannot decode HEVC Main 10 at 4K on the drivers of
    /// NVIDIA of 2026.
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
/// given, and up to [`VideoDecoder::MOST_HELD`] held by whoever was given them.
const EXTRA_FRAMES: i32 = 6;

/// How many frames on from the next one a decoder decodes its way to, rather than seek, in a
/// file whose key frames were not all indexed when it was opened, where it has decoded no key
/// frame between them: one that it has not decoded may lie between.
const FORWARD_UNINDEXED: i64 = 30;

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

/// A frame as the decoder gave it, on the graphics card or in memory, before it is made a
/// [`Picture`] by a [`Converter`]. It may be sent to another thread to be made one there while
/// the decoder decodes the next: copying a frame of 4K from the card takes 5 to 12 ms.
pub struct Decoded {
    frame: i64,
    raw: ff::frame::Video,
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
}

/// What makes [`Decoded`] frames into [`Picture`]s: it copies them from the graphics card, and
/// converts those of other formats (4:2:2, 4:4:4, RGB, 12 bits) with swscale, to P010 if they
/// have more than 8 bits and NV12 if not. It keeps what it made for the last frame, so that the
/// next one of the same shape is made without starting again; one is used by one thread at a
/// time, and each thread that makes pictures has its own.
#[derive(Default)]
pub struct Converter {
    copied: Option<ff::frame::Video>,
    converted: Option<ff::frame::Video>,
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
    /// It cannot be copied from the card, or converted.
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
    /// As [`Converter::picture`]; `into` is then as it was, or written over in part.
    pub fn picture_into(
        &mut self,
        decoded: &Decoded,
        into: &mut Picture,
    ) -> Result<(), MediaError> {
        let mut source = &decoded.raw;
        if ffi::on_device(source.format()) {
            let copied = self.copied.get_or_insert_with(ff::frame::Video::empty);
            ffi::transfer(copied, source).map_err(MediaError::Decode)?;
            source = &*copied;
        }
        let format = source.format();
        if matches!(format, Pixel::NV12 | Pixel::P010LE) {
            write_picture(decoded.frame, source, into);
            return Ok(());
        }
        let target = if ffi::depth(format).unwrap_or(8) > 8 {
            Pixel::P010LE
        } else {
            Pixel::NV12
        };
        let (width, height) = (source.width(), source.height());
        let scaler = match &mut self.scaler {
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
                    target,
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
                self.converted = None;
                slot.insert(Scaler(made))
            }
        };
        let converted = self.converted.get_or_insert_with(ff::frame::Video::empty);
        scaler.0.run(source, converted).map_err(|error| {
            MediaError::Decode(format!("the frame cannot be converted: {error}"))
        })?;
        write_picture(decoded.frame, converted, into);
        Ok(())
    }
}

/// Write a frame of NV12 or P010 into a picture.
fn write_picture(frame: i64, source: &ff::frame::Video, into: &mut Picture) {
    into.frame = frame;
    into.width = source.width();
    into.height = source.height();
    into.bits = if source.format() == Pixel::P010LE {
        10
    } else {
        8
    };
    into.luma.clear();
    into.luma.extend_from_slice(source.data(0));
    into.chroma.clear();
    into.chroma.extend_from_slice(source.data(1));
    into.luma_stride = source.stride(0);
    into.chroma_stride = source.stride(1);
}

/// The decoder of the picture of a file.
///
/// It gives the frame at any number ([`VideoDecoder::frame_at`]), and the frames after the last
/// one it gave ([`VideoDecoder::next`]). To give a frame further on it decodes on from where it
/// is, where the key frame at or before that frame is not after the frame it is at, since
/// decoding from there is what seeking would do; else it seeks to that key frame and decodes on
/// from it. It knows key frames from the index of the file where it has one, and from those it
/// has decoded.
///
/// What it gives is decoded: it is a [`Decoded`] from [`VideoDecoder::decoded_at`] and
/// [`VideoDecoder::next_decoded`], and a [`Picture`] from the others, which make it one with a
/// [`Converter`] of the decoder. Making it one on another thread while the decoder goes on is
/// left to whoever plays the file.
pub struct VideoDecoder {
    input: Input,
    stream: usize,
    decoder: ff::decoder::Video,
    /// The device that the decoder decodes on, kept as long as the decoder is.
    _device: Option<Device>,
    path: HardwarePath,
    fallbacks: Vec<(HardwarePath, String)>,
    timing: Timing,
    /// The last frame that was given.
    given: Option<i64>,
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
    /// Whether the file indexed its frames when it was opened, as MP4 and Matroska do, so that
    /// the index knows every key frame.
    indexed: bool,
    /// How much later a key frame is shown than it is decoded, in ticks, as its packets say: an
    /// index of MP4 holds the time a frame is decoded, a few frames before it is shown where
    /// frames are reordered.
    key_shift: i64,
    /// The timestamps of the packets that were sent to the decoder since it was last emptied
    /// and whose frames have not come out yet, so that a frame that is none of them, left from
    /// before, is dropped. None after a packet without one.
    sent: Option<BTreeSet<i64>>,
    /// The number of the last frame that came out, for a frame without a timestamp.
    previous: Option<i64>,
    seeks: u64,
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
        match Self::open_on(path, None) {
            Ok(mut decoder) => {
                decoder.fallbacks = fallbacks;
                Ok(decoder)
            }
            Err(Attempt::File(error)) => Err(error),
            Err(Attempt::Hardware(why)) => Err(MediaError::Decode(why)),
        }
    }

    /// Open the picture of a file on a device, or on the processor if there is none.
    fn open_on(path: &Path, device: Option<HardwarePath>) -> Result<Self, Attempt> {
        let mut input = probe::open(path)?;
        let stream = probe::video_stream(&input).ok_or(MediaError::NoVideo)?;
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
        let decoder_of = ff::decoder::find(codec).ok_or_else(|| {
            MediaError::Unsupported(format!(
                "this build of libav cannot decode {}",
                codec.name()
            ))
        })?;
        let hardware = if let Some(device) = device {
            let made = Device::new(device.name()).map_err(Attempt::Hardware)?;
            let format = made.format_of(codec).ok_or_else(|| {
                Attempt::Hardware(format!("{} is not decoded on {device}", codec.name()))
            })?;
            made.attach(&mut context, format, EXTRA_FRAMES);
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
        let indexed = ffi::index_entries(&input, stream) > 1;
        let (device_kept, format) = hardware.unzip();
        let mut decoder = Self {
            input,
            stream,
            decoder: opened,
            _device: device_kept,
            path: device.unwrap_or(HardwarePath::Software),
            fallbacks: Vec::new(),
            timing,
            given: None,
            waiting: None,
            at_end: false,
            sent_end: false,
            last: None,
            key_frames: BTreeSet::new(),
            indexed,
            key_shift: 0,
            sent: Some(BTreeSet::new()),
            previous: None,
            seeks: 0,
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
            if first.raw.is_key() {
                self.key_frames.insert(0);
            }
            self.previous = Some(0);
            Decoded {
                frame: 0,
                raw: first.raw,
            }
        } else {
            first
        };
        self.waiting = Some(first);
        Ok(())
    }

    /// The frame that is shown at `frame` of the picture's own rate: the one numbered so, or,
    /// where none is (a file whose frames come at changing times), the last one before it.
    /// Before the first frame of a file that starts late, its first frame.
    ///
    /// # Errors
    /// `frame` is below 0 or after the last frame ([`MediaError::OutOfRange`]: a frame past the
    /// end is never given as the last one), or a frame cannot be decoded or copied. After an
    /// error, [`VideoDecoder::next`] gives nothing until a frame is asked for again.
    pub fn frame_at(&mut self, frame: i64) -> Result<Picture, MediaError> {
        let decoded = self.decoded_at(frame)?;
        self.converter.picture(&decoded)
    }

    /// The frame after the last one given, without seeking; the first frame of the file if
    /// none was given. None at the end of the file.
    ///
    /// # Errors
    /// A frame cannot be decoded or copied.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<Option<Picture>, MediaError> {
        match self.next_decoded()? {
            Some(decoded) => self.converter.picture(&decoded).map(Some),
            None => Ok(None),
        }
    }

    /// What decodes, for the log.
    #[must_use]
    pub fn path_used(&self) -> HardwarePath {
        self.path
    }

    /// Each device that was tried before the one that is used, and why it was not used.
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

    /// As [`VideoDecoder::frame_at`], but the frame as it was decoded.
    ///
    /// # Errors
    /// As [`VideoDecoder::frame_at`].
    pub fn decoded_at(&mut self, frame: i64) -> Result<Decoded, MediaError> {
        if frame < 0 || self.last.is_some_and(|last| frame > last) {
            return Err(MediaError::OutOfRange {
                frame,
                last: self.last,
            });
        }
        if self.forward_to(frame) {
            let current = match self.waiting.take() {
                Some(waiting) => waiting,
                None => match self.receive()? {
                    Some(decoded) => decoded,
                    None => return Err(self.ended(frame, None)),
                },
            };
            return self.settle(frame, current);
        }
        self.seek_to(frame)
    }

    /// As [`VideoDecoder::next`], but the frame as it was decoded.
    ///
    /// # Errors
    /// As [`VideoDecoder::next`].
    pub fn next_decoded(&mut self) -> Result<Option<Decoded>, MediaError> {
        let next = match self.waiting.take() {
            Some(waiting) => Some(waiting),
            None if self.at_end => None,
            None => self.receive()?,
        };
        if let Some(decoded) = &next {
            self.given = Some(decoded.frame);
        } else {
            self.at_end = true;
            if let Some(given) = self.given {
                self.last = Some(given);
            }
        }
        Ok(next)
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
        // The last key frame shown at or before the frame is decoded at or before this.
        let decoded_by = self
            .timing
            .timestamp_of(frame)
            .saturating_sub(self.key_shift);
        let indexed =
            ffi::key_frame_before(&self.input, self.stream, decoded_by).map(|timestamp| {
                self.timing
                    .frame_of(timestamp.saturating_add(self.key_shift))
            });
        let key_before = learned.max(indexed).is_none_or(|key| key <= next);
        key_before && (self.indexed || frame - next <= FORWARD_UNINDEXED)
    }

    /// Seek to the key frame at or before `frame`, and decode on to it.
    fn seek_to(&mut self, frame: i64) -> Result<Decoded, MediaError> {
        let mut back = 0_i64;
        loop {
            let from = frame.saturating_sub(back);
            let from_start = from <= 0;
            self.seek(self.timing.timestamp_of(from.max(0)), from_start)?;
            let Some(first) = self.receive()? else {
                if from_start {
                    return Err(self.ended(frame, None));
                }
                back = if back == 0 { 8 } else { back.saturating_mul(2) };
                continue;
            };
            // The index may hold the time a key frame is decoded rather than shown: then the
            // key frame found may be shown after the frame. Seek again from further back.
            if first.frame > frame && !from_start {
                back = if back == 0 { 8 } else { back.saturating_mul(2) };
                continue;
            }
            return self.settle(frame, first);
        }
    }

    /// Seek to the key frame at or before `timestamp`, and empty the decoder.
    fn seek(&mut self, timestamp: i64, from_start: bool) -> Result<(), MediaError> {
        self.seeks += 1;
        let mut sought = ffi::seek(&mut self.input, self.stream, timestamp, true);
        if sought.is_err() && from_start {
            // Nothing is indexed at or before the start: take the first key frame after it.
            sought = ffi::seek(&mut self.input, self.stream, timestamp, false);
        }
        sought.map_err(MediaError::Decode)?;
        self.decoder.flush();
        self.waiting = None;
        self.given = None;
        self.at_end = false;
        self.sent_end = false;
        self.sent = Some(BTreeSet::new());
        self.previous = None;
        Ok(())
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
        self.given = Some(current.frame);
        Ok(current)
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
            let mut raw = ff::frame::Video::empty();
            match self.decoder.receive_frame(&mut raw) {
                Ok(()) => {
                    if !self.belongs(&raw) {
                        continue;
                    }
                    let frame = match raw.timestamp().or(raw.pts()) {
                        Some(timestamp) => self.timing.frame_of(timestamp),
                        None => self.previous.map_or(0, |previous| previous + 1),
                    };
                    self.previous = Some(frame);
                    if raw.is_key() {
                        self.key_frames.insert(frame);
                    }
                    return Ok(Some(Decoded { frame, raw }));
                }
                Err(ff::Error::Other { errno }) if errno == ff::util::error::EAGAIN => {
                    self.feed()?;
                }
                Err(ff::Error::Eof) => return Ok(None),
                Err(error) => return Err(MediaError::Decode(error.to_string())),
            }
        }
    }

    /// Whether a frame that came out was sent since the decoder was last emptied: a decoder on
    /// a card may give a frame from before a seek after it. Forgets the timestamps up to it,
    /// since frames come out in the order they are shown.
    fn belongs(&mut self, raw: &ff::frame::Video) -> bool {
        let (Some(sent), Some(pts)) = (&mut self.sent, raw.pts()) else {
            return true;
        };
        if !sent.contains(&pts) {
            return false;
        }
        *sent = sent.split_off(&pts.saturating_add(1));
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
                            if let Some(sent) = &mut self.sent {
                                sent.insert(pts);
                            }
                        }
                        None => self.sent = None,
                    }
                    match self.decoder.send_packet(&packet) {
                        Ok(()) => return Ok(()),
                        // A broken packet is passed over, as ffmpeg does.
                        Err(ff::Error::InvalidData) => {}
                        Err(error) => return Err(MediaError::Decode(error.to_string())),
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
                    return Err(MediaError::Decode(format!(
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

impl From<MediaError> for Attempt {
    fn from(error: MediaError) -> Self {
        Self::File(error)
    }
}
