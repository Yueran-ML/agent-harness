//! Pictures and sound encoded and written into a file: the picture by an encoder of the graphics
//! card, or as `ProRes` where it has none, the sound as AAC or PCM, in MP4 or `QuickTime`
//! ([`Writer`]); or sound alone, as PCM in WAV or AAC in MPEG-4 audio (`.m4a`), with
//! [`SoundWriter`].

use std::fmt;
use std::path::Path;
use std::sync::{Mutex, PoisonError};

use columbina_model::Rate;
use ff::format::Pixel;
use ffmpeg_next as ff;

use crate::ffi::{self, Scaler};
use crate::{probe, MediaError, Picture};

/// What the picture is coded as.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Codec {
    /// H.264, of 8 bits.
    H264,
    /// HEVC (H.265), of 8 or 10 bits.
    Hevc,
    /// `ProRes` 422, of 10 bits, in `QuickTime` only.
    ProRes,
}

impl Codec {
    /// Its name, as libav names it (`h264`, `hevc`, `prores`), and as [`crate::probe`] gives it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::H264 => "h264",
            Self::Hevc => "hevc",
            Self::ProRes => "prores",
        }
    }
}

/// What encodes the picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EncoderPath {
    /// The graphics card of NVIDIA.
    Nvenc,
    /// The graphics of Intel (Quick Sync Video).
    Qsv,
    /// The graphics card of AMD.
    Amf,
    /// The graphics of a Mac.
    VideoToolbox,
    /// The processor, in `ProRes` (`prores_ks` of libav).
    ProResKs,
}

impl EncoderPath {
    /// Its name, for a log: `nvenc`, `qsv`, `amf`, `videotoolbox` or `prores_ks`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Nvenc => "nvenc",
            Self::Qsv => "qsv",
            Self::Amf => "amf",
            Self::VideoToolbox => "videotoolbox",
            Self::ProResKs => "prores_ks",
        }
    }

    /// The name of its encoder of `codec` in libav (`hevc_nvenc`), if it encodes `codec`.
    #[must_use]
    pub fn encoder_name(self, codec: Codec) -> Option<&'static str> {
        Some(match (self, codec) {
            (Self::Nvenc, Codec::H264) => "h264_nvenc",
            (Self::Nvenc, Codec::Hevc) => "hevc_nvenc",
            (Self::Qsv, Codec::H264) => "h264_qsv",
            (Self::Qsv, Codec::Hevc) => "hevc_qsv",
            (Self::Amf, Codec::H264) => "h264_amf",
            (Self::Amf, Codec::Hevc) => "hevc_amf",
            (Self::VideoToolbox, Codec::H264) => "h264_videotoolbox",
            (Self::VideoToolbox, Codec::Hevc) => "hevc_videotoolbox",
            (Self::ProResKs, Codec::ProRes) => "prores_ks",
            _ => return None,
        })
    }
}

impl fmt::Display for EncoderPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

/// How much of the picture is kept, against how large the file is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Quality {
    /// Near what was given: NVENC at a constant quality of 18, `ProRes` 422.
    High,
    /// NVENC at 23, `ProRes` 422 LT.
    Medium,
    /// NVENC at 28, `ProRes` 422 Proxy.
    Small,
}

impl Quality {
    /// The quantizer that the encoders of the graphics card aim at: 18, 23 or 28.
    fn quantizer(self) -> u32 {
        match self {
            Self::High => 18,
            Self::Medium => 23,
            Self::Small => 28,
        }
    }
}

/// The picture of a file to be written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoSettings {
    /// What it is coded as.
    pub codec: Codec,
    /// Its width in pixels: even.
    pub width: u32,
    /// Its height in pixels: even.
    pub height: u32,
    /// Its rate of frames.
    pub rate: Rate,
    /// The bits of each sample: 8, or 10 for HEVC. `ProRes` is written in 10 bits whatever this
    /// says.
    pub bits: u8,
    /// How much of the picture is kept.
    pub quality: Quality,
}

/// What a file says of the colours of its picture: H.273's numbers of its primaries, transfer and
/// matrix (as `ColorFacts` reads them), its range, and of HDR the display it was mastered on
/// (SMPTE ST 2086). By default BT.709 of limited range, as every file was written before.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorTags {
    pub primaries: u8,
    pub transfer: u8,
    pub matrix: u8,
    pub full_range: bool,
    pub mastering: Option<Mastering>,
}

impl Default for ColorTags {
    fn default() -> Self {
        ColorTags {
            primaries: 1,
            transfer: 1,
            matrix: 1,
            full_range: false,
            mastering: None,
        }
    }
}

/// The display that a picture of HDR was mastered on: the chromaticities (x, y) of its red,
/// green and blue and of its white, and its least and most luminance in nits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mastering {
    pub primaries: [[f64; 2]; 3],
    pub white: [f64; 2],
    pub min_nits: f64,
    pub max_nits: f64,
}

/// The light of the content of a picture of HDR (CTA-861.3): the brightest pixel of any frame
/// (`MaxCLL`) and the brightest mean of a frame (`MaxFALL`), in nits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ContentLight {
    pub max_cll: u32,
    pub max_fall: u32,
}

/// An `AVMasteringDisplayMetadata`: the primaries and white as fractions of 50000ths, the least
/// and most luminance as fractions of 10000ths of a nit, both said to be there (22 `int`s, as the
/// reader of `color.rs` reads them).
pub(crate) fn mastering_bytes_of(mastering: &Mastering) -> Vec<u8> {
    let mut words: Vec<i32> = Vec::with_capacity(22);
    #[allow(
        clippy::cast_possible_truncation,
        reason = "chromaticities and nits in their units"
    )]
    let mut fraction = |value: f64, unit: i32| {
        words.push((value * f64::from(unit)).round() as i32);
        words.push(unit);
    };
    for point in mastering.primaries.iter().chain([&mastering.white]) {
        fraction(point[0], 50_000);
        fraction(point[1], 50_000);
    }
    fraction(mastering.min_nits, 10_000);
    fraction(mastering.max_nits, 10_000);
    words.push(1);
    words.push(1);
    words.iter().flat_map(|word| word.to_ne_bytes()).collect()
}

/// An error of writing the file at `path`.
fn write_error(path: &Path) -> impl Fn(String) -> MediaError + '_ {
    move |why| MediaError::Write(format!("{} cannot be written: {why}", path.display()))
}

/// What the sound is coded as.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AudioCodec {
    /// AAC, by libav's own encoder, at `bitrate` bits a second; in MP4, `QuickTime` or `.m4a`.
    Aac {
        /// Bits a second, of all the channels.
        bitrate: u32,
    },
    /// PCM of 24-bit integers, little-endian: each sample clipped to −1…1 and rounded to the
    /// nearest step of 2⁻²³; in `QuickTime` or WAV.
    PcmS24,
    /// PCM of 32-bit floats, little-endian, as they are given (nothing clipped); in `QuickTime`
    /// or WAV.
    PcmF32,
}

impl AudioCodec {
    /// Its name, as libav names its encoder (`aac`, `pcm_s24le`, `pcm_f32le`), and as
    /// [`crate::probe`] gives it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Aac { .. } => "aac",
            Self::PcmS24 => "pcm_s24le",
            Self::PcmF32 => "pcm_f32le",
        }
    }

    /// Whether it is PCM.
    #[must_use]
    pub fn is_pcm(self) -> bool {
        !matches!(self, Self::Aac { .. })
    }

    /// The most channels that it is written with: 8 of AAC, 24 of PCM.
    #[must_use]
    pub fn most_channels(self) -> u16 {
        if self.is_pcm() {
            24
        } else {
            8
        }
    }
}

/// The sound of a file to be written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioSettings {
    /// Samples a second, of each channel.
    pub rate: u32,
    /// How many channels.
    pub channels: u16,
    /// The speakers of the channels, as a mask of libav's (`AV_CH_*`): one bit for each channel,
    /// the channels given in the order of their bits (as WAV and libav keep them); or 0 for the
    /// default layout of libav for that many channels (2 is left, right).
    pub mask: u64,
    /// What it is coded as.
    pub codec: AudioCodec,
}

impl Default for AudioSettings {
    /// 48 kHz in stereo, AAC at 192 kb/s.
    fn default() -> Self {
        Self {
            rate: 48000,
            channels: 2,
            mask: 0,
            codec: AudioCodec::Aac {
                bitrate: AUDIO_BIT_RATE,
            },
        }
    }
}

/// The bits a second of the sound of AAC, where none are asked for.
pub const AUDIO_BIT_RATE: u32 = 192_000;

/// The encoders of the graphics card that are tried, in order.
const TRIED: &[EncoderPath] = if cfg!(target_os = "macos") {
    &[EncoderPath::VideoToolbox]
} else {
    &[EncoderPath::Nvenc, EncoderPath::Qsv, EncoderPath::Amf]
};

/// The encoders of `codec` that open on this machine, in the order that they are tried: NVENC,
/// then QSV, then AMF (VideoToolbox on macOS) for H.264 and HEVC, and `prores_ks` for
/// `ProRes`. Each is opened once, for a picture of 720p in 8 bits, the first time that its codec
/// is asked for, and what opens is kept; one that opens may still refuse another size, or
/// 10 bits, which [`Writer::create`] goes past. Empty for H.264 and HEVC where the graphics
/// card has no encoder: then the picture is written as `ProRes`. That none opens is not kept:
/// they are tried again the next time (a driver may have been installed, or the card was busy),
/// and so are they all once none of them could write a file.
#[must_use]
pub fn encoders_for(codec: Codec) -> Vec<EncoderPath> {
    let mut found = FOUND[codec_index(codec)]
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if let Some(found) = found.as_ref() {
        return found.clone();
    }
    crate::init();
    let tried = VideoSettings {
        codec,
        width: 1280,
        height: 720,
        rate: Rate { num: 30, den: 1 },
        bits: 8,
        quality: Quality::Medium,
    };
    let opened: Vec<EncoderPath> = candidates(codec)
        .iter()
        .copied()
        .filter(|&encoder| open_video(encoder, &tried, &ColorTags::default()).is_ok())
        .collect();
    if !opened.is_empty() {
        *found = Some(opened.clone());
    }
    opened
}

/// The encoders of each codec that were found to open, once some were.
static FOUND: [Mutex<Option<Vec<EncoderPath>>>; 3] = [const { Mutex::new(None) }; 3];

fn codec_index(codec: Codec) -> usize {
    match codec {
        Codec::H264 => 0,
        Codec::Hevc => 1,
        Codec::ProRes => 2,
    }
}

/// Forget the encoders of `codec` that were found to open: none of them could write a file.
fn forget_encoders(codec: Codec) {
    *FOUND[codec_index(codec)]
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = None;
}

/// The encoders that may encode `codec`, in the order that they are tried.
fn candidates(codec: Codec) -> &'static [EncoderPath] {
    match codec {
        Codec::ProRes => &[EncoderPath::ProResKs],
        Codec::H264 | Codec::Hevc => TRIED,
    }
}

/// What a file is written as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Container {
    Mp4,
    Mov,
    /// WAV (RF64 where it outgrows 4 GiB), of sound alone.
    Wav,
    /// MPEG-4 audio, of sound alone.
    M4a,
}

impl Container {
    /// The container that a path names by its extension, `.mp4`, `.mov`, `.wav` or `.m4a`,
    /// passing over a last `.part`: `cut.mp4.part` is written as MP4.
    fn of(path: &Path) -> Result<Self, MediaError> {
        let mut named = path.extension();
        if named.is_some_and(|extension| extension.eq_ignore_ascii_case("part")) {
            named = path.file_stem().map(Path::new).and_then(Path::extension);
        }
        match named.and_then(|extension| extension.to_str()) {
            Some(extension) if extension.eq_ignore_ascii_case("mp4") => Ok(Self::Mp4),
            Some(extension) if extension.eq_ignore_ascii_case("mov") => Ok(Self::Mov),
            Some(extension) if extension.eq_ignore_ascii_case("wav") => Ok(Self::Wav),
            Some(extension) if extension.eq_ignore_ascii_case("m4a") => Ok(Self::M4a),
            _ => Err(MediaError::Unsupported(
                "a file is written as .mp4, .mov, .wav or .m4a".into(),
            )),
        }
    }

    /// The name of its muxer in libav.
    fn muxer(self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Mov => "mov",
            Self::Wav => "wav",
            Self::M4a => "ipod",
        }
    }

    /// Whether it holds a picture.
    fn pictured(self) -> bool {
        matches!(self, Self::Mp4 | Self::Mov)
    }
}

/// Refuse sound that cannot be written so into `container`, before anything is opened: AAC in
/// WAV, PCM in MP4 or `.m4a`, a rate of 0, too many channels for the codec, or a mask that does
/// not name as many speakers as there are channels.
fn check_audio(settings: &AudioSettings, container: Container) -> Result<(), MediaError> {
    let refuse = |why: String| Err(MediaError::Unsupported(why));
    let codec = settings.codec;
    match (codec.is_pcm(), container) {
        (true, Container::Mp4 | Container::M4a) => {
            return refuse("PCM is written in .mov or .wav".into())
        }
        (false, Container::Wav) => return refuse("AAC is written in .mp4, .mov or .m4a".into()),
        _ => {}
    }
    if settings.rate == 0 || i32::try_from(settings.rate).is_err() {
        return refuse("a rate of sound too high or 0".into());
    }
    if settings.channels == 0 || settings.channels > codec.most_channels() {
        return refuse(format!(
            "{} is written with 1 to {} channels",
            codec.name(),
            codec.most_channels()
        ));
    }
    if settings.mask != 0 && settings.mask.count_ones() != u32::from(settings.channels) {
        return refuse(format!(
            "a mask of {} speakers for {} channels",
            settings.mask.count_ones(),
            settings.channels
        ));
    }
    Ok(())
}

/// Refuse a picture that cannot be written so, before anything is opened.
fn check(settings: &VideoSettings, container: Container) -> Result<(), MediaError> {
    let refuse = |why: &str| Err(MediaError::Unsupported(why.to_owned()));
    if settings.width == 0
        || settings.height == 0
        || !settings.width.is_multiple_of(2)
        || !settings.height.is_multiple_of(2)
    {
        return refuse("a picture is written with an even width and height");
    }
    if settings.width > 16384 || settings.height > 16384 {
        return refuse("a picture is written at most 16384 pixels wide and high");
    }
    let Rate { num, den } = settings.rate;
    if num == 0 || den == 0 || i32::try_from(num).is_err() || i32::try_from(den).is_err() {
        return refuse("a rate of frames is written with numbers from 1 to 2147483647");
    }
    match (settings.codec, settings.bits) {
        (Codec::H264, 8) | (Codec::Hevc | Codec::ProRes, 8 | 10) => {}
        (Codec::H264, _) => return refuse("H.264 is written in 8 bits"),
        _ => return refuse("a picture is written in 8 or 10 bits"),
    }
    if settings.codec == Codec::ProRes && container == Container::Mp4 {
        return refuse("ProRes is written in .mov");
    }
    if !container.pictured() {
        return refuse("a picture is written in .mp4 or .mov");
    }
    Ok(())
}

/// The format that the encoder of a picture is given its frames in.
fn input_format(settings: &VideoSettings) -> Pixel {
    match (settings.codec, settings.bits) {
        (Codec::ProRes, _) => Pixel::YUV422P10LE,
        (_, 10) => Pixel::P010LE,
        _ => Pixel::NV12,
    }
}

/// Open the encoder of a picture, for a file whose header holds what the decoder needs (as MP4
/// and `QuickTime` have it); the reason it cannot be opened, if it cannot.
fn open_video(
    encoder: EncoderPath,
    settings: &VideoSettings,
    tags: &ColorTags,
) -> Result<ff::encoder::Video, String> {
    let codec = settings.codec;
    let name = encoder
        .encoder_name(codec)
        .ok_or_else(|| format!("{encoder} does not encode {}", codec.name()))?;
    let found = ff::encoder::find_by_name(name)
        .ok_or_else(|| format!("this build of libav has no {name}"))?;
    let mut context = ff::codec::Context::new_with_codec(found)
        .encoder()
        .video()
        .map_err(|error| format!("{name} cannot be set up: {error}"))?;
    let Rate { num, den } = settings.rate;
    let (num, den) = (
        i32::try_from(num).map_err(|_| "a rate too high")?,
        i32::try_from(den).map_err(|_| "a rate too high")?,
    );
    context.set_width(settings.width);
    context.set_height(settings.height);
    context.set_format(input_format(settings));
    context.set_time_base((den, num));
    context.set_frame_rate(Some((num, den)));
    // A key frame every 2 s or so, so that the file is sought in quickly.
    let every = (2 * u64::from(settings.rate.num)).div_ceil(u64::from(settings.rate.den));
    context.set_gop(u32::try_from(every).unwrap_or(u32::MAX).max(1));
    // Rate is set by quality, not by bits a second (whose default is 200 kb/s).
    context.set_bit_rate(0);
    ffi::tag(&mut context, tags);
    let mut flags = ff::codec::Flags::GLOBAL_HEADER;
    let quantizer = settings.quality.quantizer();
    let mut options = ff::Dictionary::new();
    // H.264 in its High profile, which some encoders do not choose by themselves; HEVC in Main
    // or Main 10, as the bits of its frames say.
    if codec == Codec::H264 && encoder != EncoderPath::VideoToolbox {
        options.set("profile", "high");
    }
    // HEVC of 10 bits is Main 10, as HDR10 and HLG are written, asked for by name of NVENC.
    if codec == Codec::Hevc && settings.bits > 8 && encoder == EncoderPath::Nvenc {
        options.set("profile", "main10");
    }
    match encoder {
        EncoderPath::Nvenc => {
            options.set("preset", "p5");
            options.set("tune", "hq");
            options.set("rc", "vbr");
            options.set("cq", &quantizer.to_string());
        }
        EncoderPath::Qsv => {
            // A global quality without a highest rate is ICQ, its constant quality.
            context.set_global_quality(i32::try_from(quantizer).unwrap_or(23));
        }
        EncoderPath::Amf => {
            options.set("rc", "cqp");
            options.set("quality", "quality");
            for frame in ["qp_i", "qp_p", "qp_b"] {
                options.set(frame, &quantizer.to_string());
            }
        }
        EncoderPath::VideoToolbox => {
            // A constant quality of 0 to 100, in units of lambda.
            let quality = match settings.quality {
                Quality::High => 75,
                Quality::Medium => 60,
                Quality::Small => 45,
            };
            flags |= ff::codec::Flags::QSCALE;
            context.set_global_quality(quality * ff::ffi::FF_QP2LAMBDA);
        }
        EncoderPath::ProResKs => {
            let profile = match settings.quality {
                Quality::High => "standard",
                Quality::Medium => "lt",
                Quality::Small => "proxy",
            };
            options.set("profile", profile);
            options.set("vendor", "apl0");
            // ProRes is coded on the processor, a frame on each thread.
            ffi::set_threads(&mut context, 0, true);
        }
    }
    context.set_flags(flags);
    context
        .open_as_with(found, options)
        .map_err(|error| format!("{name} cannot be opened: {error}"))
}

/// The layout of the channels of `settings`: its mask, in libav's order of the bits, or libav's
/// default for that many channels.
fn channel_layout(settings: &AudioSettings) -> ff::ChannelLayout {
    if settings.mask == 0 {
        return ff::ChannelLayout::default(i32::from(settings.channels));
    }
    ff::ChannelLayout(ff::ffi::AVChannelLayout {
        order: ff::ffi::AVChannelOrder::AV_CHANNEL_ORDER_NATIVE,
        nb_channels: i32::from(settings.channels),
        u: ff::ffi::AVChannelLayout__bindgen_ty_1 {
            mask: settings.mask,
        },
        opaque: std::ptr::null_mut(),
    })
}

/// Open the encoder of the sound: libav's AAC, of 32-bit floats a channel at a time; or its PCM,
/// of 32-bit integers (of which 24 bits are written) or floats, the channels interleaved.
fn open_audio(settings: AudioSettings) -> Result<ff::encoder::Audio, MediaError> {
    use ff::format::sample::Type;
    let name = settings.codec.name();
    let failed = |error: ff::Error| MediaError::Encode(format!("{name} cannot be opened: {error}"));
    let found = ff::encoder::find_by_name(name)
        .ok_or_else(|| MediaError::Encode(format!("this build of libav has no {name}")))?;
    let rate = i32::try_from(settings.rate)
        .ok()
        .filter(|&rate| rate > 0)
        .ok_or_else(|| MediaError::Unsupported("a rate of sound too high or 0".into()))?;
    let most = settings.codec.most_channels();
    if settings.channels == 0 || settings.channels > most {
        return Err(MediaError::Unsupported(format!(
            "{name} is written with 1 to {most} channels"
        )));
    }
    let mut context = ff::codec::Context::new_with_codec(found)
        .encoder()
        .audio()
        .map_err(failed)?;
    context.set_rate(rate);
    context.set_format(match settings.codec {
        AudioCodec::Aac { .. } => ff::format::Sample::F32(Type::Planar),
        AudioCodec::PcmS24 => ff::format::Sample::I32(Type::Packed),
        AudioCodec::PcmF32 => ff::format::Sample::F32(Type::Packed),
    });
    context.set_channel_layout(channel_layout(&settings));
    if let AudioCodec::Aac { bitrate } = settings.codec {
        context.set_bit_rate(usize::try_from(bitrate).unwrap_or(usize::MAX));
    }
    context.set_time_base((1, rate));
    context.set_flags(ff::codec::Flags::GLOBAL_HEADER);
    context.open_as(found).map_err(failed)
}

/// A stream of the file: its index, and the time bases of its encoder and of the file.
#[derive(Clone, Copy)]
struct Track {
    index: usize,
    encoder: ff::Rational,
    file: ff::Rational,
}

/// The sound of a file being written: what was given and not yet encoded, and the next sample.
struct Sound {
    encoder: ff::encoder::Audio,
    track: Track,
    frame: ff::frame::Audio,
    codec: AudioCodec,
    channels: usize,
    /// Samples in each frame of AAC, of each channel.
    frame_size: usize,
    /// The samples given and not yet encoded, interleaved, from `taken` on.
    pending: Vec<f32>,
    taken: usize,
    /// The number of the next sample to be encoded.
    next: i64,
}

impl Sound {
    /// Encode a frame of `count` samples from the pending ones, and write what comes out.
    fn encode(
        &mut self,
        count: usize,
        output: &mut ff::format::context::Output,
    ) -> Result<(), MediaError> {
        // Made writable at its full size, so that its memory holds a whole frame.
        self.frame.set_samples(self.frame_size);
        ffi::writable(&mut self.frame).map_err(MediaError::Encode)?;
        self.frame.set_samples(count);
        let from = &self.pending[self.taken..self.taken + count * self.channels];
        match self.codec {
            AudioCodec::Aac { .. } => {
                for channel in 0..self.channels {
                    let plane = self.frame.plane_mut::<f32>(channel);
                    for (into, sample) in plane
                        .iter_mut()
                        .zip(from.iter().skip(channel).step_by(self.channels))
                    {
                        *into = *sample;
                    }
                }
            }
            AudioCodec::PcmF32 => {
                let bytes = self.frame.data_mut(0);
                for (into, sample) in bytes.as_chunks_mut::<4>().0.iter_mut().zip(from) {
                    *into = sample.to_ne_bytes();
                }
            }
            AudioCodec::PcmS24 => {
                let bytes = self.frame.data_mut(0);
                for (into, sample) in bytes.as_chunks_mut::<4>().0.iter_mut().zip(from) {
                    *into = s24(*sample).to_ne_bytes();
                }
            }
        }
        self.taken += count * self.channels;
        self.frame.set_pts(Some(self.next));
        self.next += i64::try_from(count).unwrap_or(i64::MAX);
        send(&mut self.encoder, Some(&self.frame), output, self.track)
    }

    /// Take the next samples, interleaved, and encode the whole frames that they make.
    fn give(
        &mut self,
        samples: &[f32],
        output: &mut ff::format::context::Output,
    ) -> Result<(), MediaError> {
        if !samples.len().is_multiple_of(self.channels) {
            return Err(MediaError::Encode(format!(
                "{} samples are not whole instants of {} channels",
                samples.len(),
                self.channels
            )));
        }
        self.pending.extend_from_slice(samples);
        let whole = self.frame_size * self.channels;
        while self.pending.len() - self.taken >= whole {
            self.encode(self.frame_size, output)?;
        }
        self.pending.drain(..self.taken);
        self.taken = 0;
        Ok(())
    }

    /// Encode what is left, in a shorter frame, and the end.
    fn end(&mut self, output: &mut ff::format::context::Output) -> Result<(), MediaError> {
        let left = (self.pending.len() - self.taken) / self.channels;
        if left > 0 {
            self.encode(left, output)?;
        }
        send(&mut self.encoder, None, output, self.track)
    }
}

/// A file of sound alone being written: PCM in WAV (RF64 once it outgrows the 4 GiB of WAV), or
/// AAC in MPEG-4 audio (`.m4a`, its index moved before its data), as its path names (`.wav`,
/// `.m4a`, either with `.part` after it). Sound is given in runs of any length by
/// [`SoundWriter::audio`], each after the last, from time 0. A writer that is dropped without
/// [`SoundWriter::finish`] leaves a file that cannot be played.
pub struct SoundWriter {
    output: ff::format::context::Output,
    sound: Sound,
}

impl SoundWriter {
    /// Make a file of sound at `path` and write its header.
    ///
    /// # Errors
    /// The path is not UTF-8 ([`MediaError::NotUtf8`]); it names no container of `.wav` or
    /// `.m4a`, or the sound cannot be written into it ([`MediaError::Unsupported`]: PCM in
    /// `.m4a`, AAC in WAV, too many channels); the encoder does not open
    /// ([`MediaError::Encode`]); or the file cannot be made ([`MediaError::Write`]).
    pub fn create(path: &Path, settings: AudioSettings) -> Result<Self, MediaError> {
        crate::init();
        let container = Container::of(path)?;
        if container.pictured() {
            return Err(MediaError::Unsupported(
                "sound alone is written as .wav or .m4a".into(),
            ));
        }
        check_audio(&settings, container)?;
        probe::checked(path)?;
        let encoder = open_audio(settings)?;
        let full = std::path::absolute(path)?;
        let url = format!("file:{}", probe::checked(&full)?);
        let write = |error: ff::Error| {
            MediaError::Write(format!("{} cannot be written: {error}", path.display()))
        };
        let mut output = ff::format::output_as(&url, container.muxer()).map_err(write)?;
        let index = {
            let mut stream = output.add_stream_with(&encoder).map_err(write)?;
            stream.set_time_base(encoder.time_base());
            stream.index()
        };
        let mut options = ff::Dictionary::new();
        match container {
            Container::Wav => options.set("rf64", "auto"),
            _ => options.set("movflags", "+faststart"),
        }
        output.write_header_with(options).map_err(write)?;
        let sound = sound_of(encoder, settings, index, &output);
        Ok(Self { output, sound })
    }

    /// The name of its encoder in libav: `pcm_s24le`, `pcm_f32le` or `aac`.
    #[must_use]
    pub fn encoder(&self) -> &'static str {
        self.sound.codec.name()
    }

    /// Encode the next samples: 32-bit floats, interleaved, as many channels an instant as the
    /// file has (in the order of the bits of its mask). Those that do not fill a frame of the
    /// encoder are kept for the next run, or for [`SoundWriter::finish`].
    ///
    /// # Errors
    /// The samples are not whole instants ([`MediaError::Encode`]); or they cannot be encoded,
    /// or the file cannot be written.
    pub fn audio(&mut self, samples: &[f32]) -> Result<(), MediaError> {
        self.sound.give(samples, &mut self.output)
    }

    /// Encode what is left and write the end of the file.
    ///
    /// # Errors
    /// What is left cannot be encoded, or the file cannot be written.
    pub fn finish(mut self) -> Result<(), MediaError> {
        self.sound.end(&mut self.output)?;
        self.output.write_trailer().map_err(|error| {
            MediaError::Write(format!("the end of the file cannot be written: {error}"))
        })
    }
}

/// A sample as PCM of 24 bits, in the top 24 bits of an `i32`, as libav's `pcm_s24le` takes it:
/// clipped to −1…1 and rounded to the nearest of the steps of 2⁻²³, 1 written as the largest,
/// 1 − 2⁻²³ (and what is not a number as 0).
#[allow(
    clippy::cast_possible_truncation,
    reason = "rounded and clamped to the range of 24 bits"
)]
fn s24(sample: f32) -> i32 {
    let scaled = (f64::from(sample).clamp(-1.0, 1.0) * 8_388_608.0).round();
    let code = scaled.clamp(-8_388_608.0, 8_388_607.0) as i32;
    code << 8
}

/// The sound of a file of `settings` written into `output` as stream `index`: its encoder, and
/// a frame to give it.
fn sound_of(
    encoder: ff::encoder::Audio,
    settings: AudioSettings,
    index: usize,
    output: &ff::format::context::Output,
) -> Sound {
    let track = Track {
        index,
        encoder: encoder.time_base(),
        file: output
            .stream(index)
            .map_or(encoder.time_base(), |stream| stream.time_base()),
    };
    let frame_size = usize::try_from(encoder.frame_size())
        .ok()
        .filter(|&size| size > 0)
        .unwrap_or(1024);
    let mut frame = ff::frame::Audio::new(encoder.format(), frame_size, encoder.channel_layout());
    frame.set_rate(settings.rate);
    Sound {
        encoder,
        track,
        frame,
        codec: settings.codec,
        channels: usize::from(settings.channels),
        frame_size,
        pending: Vec::new(),
        taken: 0,
        next: 0,
    }
}

/// A file being written: one picture, encoded on the graphics card or as `ProRes`, and sound in
/// AAC or PCM if it was asked for, in MP4 or `QuickTime` as its path names (`.mp4`, `.mov`, either with
/// `.part` after it). Its index is moved before its data when it is finished, so that it plays
/// while it is downloaded (`faststart`).
///
/// Frames are given one after another by [`Writer::video`], each the frame after the last, so
/// that the first is at time 0 and frame `n` at `n` frames of the rate. Sound is given in runs
/// of any length by [`Writer::audio`], each after the last, from time 0. A writer that is
/// dropped without [`Writer::finish`] leaves a file that cannot be played.
pub struct Writer {
    output: ff::format::context::Output,
    used: EncoderPath,
    fallbacks: Vec<(EncoderPath, String)>,
    width: u32,
    height: u32,
    encoder: ff::encoder::Video,
    track: Track,
    /// The frame that a picture is copied or converted into, to be encoded.
    frame: ff::frame::Video,
    /// What converts pictures that are not in the format of the encoder.
    scaler: Option<Scaler>,
    /// How many frames were given.
    frames: i64,
    sound: Option<Sound>,
}

impl Writer {
    /// Make a file at `path` and write the header of its picture and sound, with the first
    /// encoder of [`encoders_for`] the codec that opens for these settings; those tried before
    /// it, and why they did not open, are in [`Writer::fallbacks`].
    ///
    /// # Errors
    /// The path is not UTF-8 ([`MediaError::NotUtf8`]); it names no container of `.mp4` or
    /// `.mov`, or the settings cannot be written ([`MediaError::Unsupported`]: an odd size, H.264
    /// of 10 bits, `ProRes` in MP4); no encoder of the codec opens
    /// ([`MediaError::NoEncoder`]); the sound cannot be encoded ([`MediaError::Encode`]); or
    /// the file cannot be made ([`MediaError::Write`]).
    pub fn create(
        path: &Path,
        video: VideoSettings,
        audio: Option<AudioSettings>,
    ) -> Result<Self, MediaError> {
        Self::create_tagged(path, video, audio, &ColorTags::default())
    }

    /// As [`Writer::create`], the file saying that its picture's colours are `tags`: in the
    /// parameters of its encoder and stream (and so in the `colr` of MP4 and `QuickTime`), and of
    /// HDR, its mastering display in the stream's side data (`mdcv`), and in the encoder's where
    /// it writes it into the stream (SEI). Its light ([`Writer::set_content_light`]) is said
    /// when it is finished.
    ///
    /// # Errors
    /// As [`Writer::create`].
    pub fn create_tagged(
        path: &Path,
        video: VideoSettings,
        audio: Option<AudioSettings>,
        tags: &ColorTags,
    ) -> Result<Self, MediaError> {
        let container = Container::of(path)?;
        check(&video, container)?;
        // The sound first: an encoder of sound that does not open is not one of the picture
        // that fails, to be passed for the next.
        if let Some(settings) = audio {
            check_audio(&settings, container)?;
            drop(open_audio(settings)?);
        }
        let mut fallbacks = Vec::new();
        for encoder in encoders_for(video.codec) {
            match Self::create_tagged_on(path, encoder, video, audio, tags) {
                Ok(mut writer) => {
                    writer.fallbacks = fallbacks;
                    return Ok(writer);
                }
                Err(MediaError::Encode(why)) => fallbacks.push((encoder, why)),
                Err(error) => return Err(error),
            }
        }
        // Found to open, and none writes: they are looked for again the next time.
        forget_encoders(video.codec);
        let why = if fallbacks.is_empty() {
            format!("{} opens on this machine", video.codec.name())
        } else {
            let tried: Vec<String> = fallbacks
                .iter()
                .map(|(encoder, why)| format!("{encoder}: {why}"))
                .collect();
            format!("{} opens: {}", video.codec.name(), tried.join("; "))
        };
        Err(MediaError::NoEncoder(why))
    }

    /// As [`Writer::create`], with `encoder` only.
    ///
    /// # Errors
    /// As [`Writer::create`]; where the encoder does not open, [`MediaError::Encode`].
    pub fn create_on(
        path: &Path,
        encoder: EncoderPath,
        video: VideoSettings,
        audio: Option<AudioSettings>,
    ) -> Result<Self, MediaError> {
        Self::create_tagged_on(path, encoder, video, audio, &ColorTags::default())
    }

    /// As [`Writer::create_on`], tagged as [`Writer::create_tagged`] says.
    ///
    /// # Errors
    /// As [`Writer::create_on`].
    pub fn create_tagged_on(
        path: &Path,
        encoder: EncoderPath,
        video: VideoSettings,
        audio: Option<AudioSettings>,
        tags: &ColorTags,
    ) -> Result<Self, MediaError> {
        crate::init();
        let container = Container::of(path)?;
        check(&video, container)?;
        if let Some(settings) = &audio {
            check_audio(settings, container)?;
        }
        if encoder.encoder_name(video.codec).is_none() {
            return Err(MediaError::Unsupported(format!(
                "{encoder} does not encode {}",
                video.codec.name()
            )));
        }
        probe::checked(path)?;
        let opened = open_video(encoder, &video, tags).map_err(MediaError::Encode)?;
        let sound = audio.map(open_audio).transpose()?;

        // The path is given as a URL of the file protocol, as a file is read (`crate::probe`).
        let full = std::path::absolute(path)?;
        let url = format!("file:{}", probe::checked(&full)?);
        let write = |error: ff::Error| {
            MediaError::Write(format!("{} cannot be written: {error}", path.display()))
        };
        let mut output = ff::format::output_as(&url, container.muxer()).map_err(write)?;
        let rate = (video.rate.num, video.rate.den);
        #[allow(clippy::cast_possible_wrap)]
        let rate = ff::Rational::new(rate.0 as i32, rate.1 as i32);
        let video_index = {
            let mut stream = output.add_stream_with(&opened).map_err(write)?;
            stream.set_time_base(opened.time_base());
            stream.set_rate(rate);
            stream.set_avg_frame_rate(rate);
            stream.index()
        };
        if video.codec == Codec::Hevc {
            ffi::set_codec_tag(&mut output, video_index, *b"hvc1");
        }
        if let Some(mastering) = &tags.mastering {
            ffi::add_stream_side_data(
                &mut output,
                video_index,
                ffi::SideData::Mastering(mastering_bytes_of(mastering)),
            )
            .map_err(write_error(path))?;
        }
        let audio_index = match &sound {
            Some(encoder) => {
                let mut stream = output.add_stream_with(encoder).map_err(write)?;
                stream.set_time_base(encoder.time_base());
                Some(stream.index())
            }
            None => None,
        };
        let mut options = ff::Dictionary::new();
        options.set("movflags", "+faststart");
        output.write_header_with(options).map_err(write)?;
        let track = |index: usize, encoder: ff::Rational| Track {
            index,
            encoder,
            file: output
                .stream(index)
                .map_or(encoder, |stream| stream.time_base()),
        };
        let video_track = track(video_index, opened.time_base());
        let sound = match (sound, audio_index, audio) {
            (Some(encoder), Some(index), Some(settings)) => {
                Some(sound_of(encoder, settings, index, &output))
            }
            _ => None,
        };
        let frame = ff::frame::Video::new(input_format(&video), video.width, video.height);
        Ok(Self {
            output,
            used: encoder,
            fallbacks: Vec::new(),
            width: video.width,
            height: video.height,
            encoder: opened,
            track: video_track,
            frame,
            scaler: None,
            frames: 0,
            sound,
        })
    }

    /// The encoder that encodes the picture.
    #[must_use]
    pub fn encoder(&self) -> EncoderPath {
        self.used
    }

    /// Each encoder that was tried before the one that is used, and why it did not open.
    #[must_use]
    pub fn fallbacks(&self) -> &[(EncoderPath, String)] {
        &self.fallbacks
    }

    /// An error of the encoder of the picture, with its name.
    fn named(&self, error: MediaError) -> MediaError {
        match error {
            MediaError::Encode(why) => MediaError::Encode(format!("{}: {why}", self.used)),
            other => other,
        }
    }

    /// How many frames were given.
    #[must_use]
    pub fn frames(&self) -> i64 {
        self.frames
    }

    /// Encode the next frame: a picture of the size of the file, in NV12 or P010, whatever the
    /// bits of the file (a picture of other bits is converted). Its number is not looked at:
    /// it is the frame after the last one given.
    ///
    /// # Errors
    /// The picture is of another size than the file's, of other bits than 8 or 10, or its
    /// planes are shorter than its size; or it cannot be encoded ([`MediaError::Encode`]); or
    /// the file cannot be written ([`MediaError::Write`]).
    pub fn video(&mut self, picture: &Picture) -> Result<(), MediaError> {
        if (picture.width, picture.height) != (self.width, self.height) {
            return Err(MediaError::Encode(format!(
                "a picture of {}×{} is given to a file of {}×{}",
                picture.width, picture.height, self.width, self.height
            )));
        }
        if !matches!(picture.bits, 8 | 10) {
            return Err(MediaError::Encode(format!(
                "a picture of {} bits is given",
                picture.bits
            )));
        }
        ffi::writable(&mut self.frame).map_err(MediaError::Encode)?;
        let format = ffi::format_of(picture);
        if format == self.frame.format() {
            copy_into(picture, &mut self.frame)?;
        } else {
            let (width, height) = (self.width, self.height);
            let target = self.frame.format();
            let scaler = match &mut self.scaler {
                Some(scaler) if scaler.0.input().format == format => scaler,
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
                        MediaError::Encode(format!("pictures cannot be converted: {error}"))
                    })?;
                    slot.insert(Scaler(made))
                }
            };
            ffi::scale_from(scaler, picture, &mut self.frame).map_err(MediaError::Encode)?;
        }
        self.frame.set_pts(Some(self.frames));
        send(
            &mut self.encoder,
            Some(&self.frame),
            &mut self.output,
            self.track,
        )
        .map_err(|error| self.named(error))?;
        self.frames += 1;
        Ok(())
    }

    /// Encode the next samples of the sound: 32-bit floats from -1 to 1, interleaved, as many
    /// channels an instant as the file has. Those that do not fill a frame of AAC are kept for
    /// the next run, or for [`Writer::finish`].
    ///
    /// # Errors
    /// The file has no sound, or the samples are not whole instants ([`MediaError::Encode`]);
    /// or they cannot be encoded, or the file cannot be written.
    pub fn audio(&mut self, samples: &[f32]) -> Result<(), MediaError> {
        let Some(sound) = &mut self.sound else {
            return Err(MediaError::Encode(
                "sound is given to a file without sound".into(),
            ));
        };
        sound.give(samples, &mut self.output)
    }

    /// Say the light of the content of the picture (`MaxCLL` and `MaxFALL`), measured of its
    /// frames, in the stream's side data (`clli` of MP4 and `QuickTime`): given before
    /// [`Writer::finish`], which writes the index of the file and its description of the stream
    /// in it.
    ///
    /// # Errors
    /// libav cannot keep it.
    pub fn set_content_light(&mut self, light: ContentLight) -> Result<(), MediaError> {
        let mut bytes = Vec::with_capacity(8);
        bytes.extend_from_slice(&light.max_cll.to_ne_bytes());
        bytes.extend_from_slice(&light.max_fall.to_ne_bytes());
        ffi::add_stream_side_data(
            &mut self.output,
            self.track.index,
            ffi::SideData::ContentLight(bytes),
        )
        .map_err(MediaError::Write)
    }

    /// Encode what is left, the last samples of sound in a shorter frame, and write the end
    /// of the file, with its index moved before its data.
    ///
    /// # Errors
    /// What is left cannot be encoded, or the file cannot be written.
    pub fn finish(mut self) -> Result<(), MediaError> {
        if let Some(sound) = &mut self.sound {
            sound.end(&mut self.output)?;
        }
        send(&mut self.encoder, None, &mut self.output, self.track)
            .map_err(|error| self.named(error))?;
        self.output.write_trailer().map_err(|error| {
            MediaError::Write(format!("the end of the file cannot be written: {error}"))
        })
    }
}

/// Copy a picture into a frame of its format and size, a row at a time.
fn copy_into(picture: &Picture, frame: &mut ff::frame::Video) -> Result<(), MediaError> {
    let bytes = if picture.bits > 8 { 2 } else { 1 };
    let row = (picture.width as usize).div_ceil(2) * 2 * bytes;
    let rows = picture.height as usize;
    for (plane, from, stride, count) in [
        (0, &picture.luma, picture.luma_stride, rows),
        (1, &picture.chroma, picture.chroma_stride, rows.div_ceil(2)),
    ] {
        if stride < row || from.len() < stride * count {
            return Err(MediaError::Encode(
                "the planes of the picture are shorter than its size".into(),
            ));
        }
        let into_stride = frame.stride(plane);
        let into = frame.data_mut(plane);
        for y in 0..count {
            into[y * into_stride..y * into_stride + row]
                .copy_from_slice(&from[y * stride..y * stride + row]);
        }
    }
    Ok(())
}

/// Send an encoder a frame, or the end where there is none, and write the packets that come out
/// into the file.
fn send(
    encoder: &mut ff::codec::encoder::Encoder,
    frame: Option<&ff::Frame>,
    output: &mut ff::format::context::Output,
    track: Track,
) -> Result<(), MediaError> {
    let sent = match frame {
        Some(frame) => encoder.send_frame(frame),
        None => encoder.send_eof(),
    };
    match sent {
        Ok(()) => {}
        // The encoder has packets to be taken first; then it takes the frame.
        Err(ff::Error::Other { errno }) if errno == ff::util::error::EAGAIN => {
            drain(encoder, output, track)?;
            match frame {
                Some(frame) => encoder.send_frame(frame),
                None => encoder.send_eof(),
            }
            .map_err(|error| {
                MediaError::Encode(format!("a frame cannot be encoded: {}", described(error)))
            })?;
        }
        Err(ff::Error::Eof) if frame.is_none() => {}
        Err(error) => {
            return Err(MediaError::Encode(format!(
                "a frame cannot be encoded: {}",
                described(error)
            )))
        }
    }
    drain(encoder, output, track)
}

/// Write the packets that an encoder has ready into the file.
fn drain(
    encoder: &mut ff::codec::encoder::Encoder,
    output: &mut ff::format::context::Output,
    track: Track,
) -> Result<(), MediaError> {
    let mut packet = ff::Packet::empty();
    loop {
        match encoder.receive_packet(&mut packet) {
            Ok(()) => {
                packet.set_stream(track.index);
                packet.rescale_ts(track.encoder, track.file);
                packet.write_interleaved(output).map_err(|error| {
                    MediaError::Write(format!("the file cannot be written: {error}"))
                })?;
            }
            Err(ff::Error::Other { errno }) if errno == ff::util::error::EAGAIN => return Ok(()),
            Err(ff::Error::Eof) => return Ok(()),
            Err(error) => {
                return Err(MediaError::Encode(format!(
                    "a frame cannot be encoded: {}",
                    described(error)
                )))
            }
        }
    }
}

/// The text of an error of libav, or its code where libav has no text for it.
fn described(error: ff::Error) -> String {
    let text = error.to_string();
    if text.is_empty() {
        format!("{error:?}")
    } else {
        text
    }
}
