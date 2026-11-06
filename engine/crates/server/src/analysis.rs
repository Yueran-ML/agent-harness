//! Small pictures and native-sample audio peaks for the media pool and timeline. These read
//! a snapshot of media, never the project history. At most two thumbnail decoders and one
//! waveform decoder run in the background.

use std::collections::{HashSet, VecDeque};
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use columbina_media::{AudioDecoder, Converter, Hardware, MediaError, Rgba, VideoDecoder};
use columbina_model::{ChannelSource, Media, Time};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::Semaphore;

use crate::log::log;
use crate::proxy;
use crate::rpc::{self, Fault};
use crate::session::cannot_read;

const RATE: u32 = 100;
// Leave room for base64 and the JSON envelope within the protocol's line limit. This also
// bounds allocations when a hand-written project gives a source an implausible duration.
const MAX_PEAK_BYTES: usize = (rpc::MAX_LINE - 1024) / 4 * 3;
const WAVE_MAGIC: &[u8] = b"CWAV1";

#[cfg(test)]
mod mix_tests;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThumbnailRequest {
    pub media: String,
    pub frame: Option<i64>,
    pub height: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WaveformRequest {
    pub media: String,
    /// A stream of sound of the media, by its index among all the streams of the file, its
    /// channels as the stream has them; none for the sound of the media, its best stream.
    #[serde(default)]
    pub stream: Option<usize>,
    /// One output channel: the sample-wise mean of these native source channels.
    #[serde(default)]
    pub mix: Option<Vec<ChannelSource>>,
    /// The best stream converted to this many output channels, as automatic clip mapping.
    #[serde(default)]
    pub remix: Option<u16>,
}

/// Told that the waveform of a media, or of one of its streams of sound, is done.
pub type Notify =
    Box<dyn Fn(&Media, Option<usize>, Option<&[ChannelSource]>, Option<u16>) + Send + Sync>;

pub struct Analysis(Arc<Shared>);

struct Shared {
    project: PathBuf,
    thumb_slots: Arc<Semaphore>,
    wave_slots: Arc<Semaphore>,
    stopped: AtomicBool,
    jobs: Mutex<Jobs>,
    notify: Notify,
}

#[derive(Default)]
struct Jobs {
    pending: HashSet<String>,
    // Disk is the normal cache. Keep only two completed answers if writing it failed, or
    // two errors so the next request receives the failure instead of pending forever.
    recent: VecDeque<(String, Result<Wave, Fault>)>,
}

struct Wave {
    channels: u16,
    peaks: Vec<u8>,
}

impl Wave {
    fn value(&self) -> Value {
        json!({"rate": RATE, "channels": self.channels, "peaks": STANDARD.encode(&self.peaks)})
    }
}

/// A job is started after its caller has queued the pending response.
pub struct WaveJob {
    shared: Arc<Shared>,
    media: Media,
    stream: Option<usize>,
    mix: Option<Vec<ChannelSource>>,
    remix: Option<u16>,
    key: String,
    /// The size of the waveform; none for a stream whose file could not be opened, which the
    /// job opens again, to make its waveform or to say why it cannot.
    spec: Option<WaveSpec>,
}

impl Analysis {
    pub fn new(project: &Path, notify: Notify) -> Self {
        Self(Arc::new(Shared {
            project: project.to_owned(),
            thumb_slots: Arc::new(Semaphore::new(2)),
            wave_slots: Arc::new(Semaphore::new(1)),
            stopped: AtomicBool::new(false),
            jobs: Mutex::default(),
            notify,
        }))
    }

    pub fn stop(&self) {
        self.0.stopped.store(true, Ordering::Release);
        self.0.thumb_slots.close();
        self.0.wave_slots.close();
    }

    pub async fn thumbnail(&self, media: Media, request: ThumbnailRequest) -> Result<Value, Fault> {
        let spec = ThumbSpec::new(&media, &request)?;
        let shared = Arc::clone(&self.0);
        let permit = Arc::clone(&shared.thumb_slots)
            .acquire_owned()
            .await
            .map_err(worker_error)?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let name = format!("{}-{}-{}.png", stem(&media), spec.frame, spec.height);
            let cache = cache_path(&shared.project, "thumbs", &name);
            let png = cache
                .as_ref()
                .and_then(|path| cached_png(path, &spec))
                .map_or_else(
                    || {
                        let rgba = picture(&media, &spec)?;
                        let png = encode_png(&rgba)
                            .map_err(|error| cannot_read(&media.path, "encode", &error))?;
                        if base64::encoded_len(png.len(), true)
                            .is_none_or(|length| length > rpc::MAX_LINE - 1024)
                        {
                            return Err(cannot_read(
                                &media.path,
                                "too_large",
                                "the thumbnail exceeds one protocol message",
                            ));
                        }
                        if let Some(cache) = &cache {
                            save(cache, &png);
                        }
                        Ok(png)
                    },
                    Ok,
                )?;
            Ok(json!({"width":spec.width, "height":spec.height, "png":STANDARD.encode(png)}))
        })
        .await
        .map_err(worker_error)?
    }

    pub async fn waveform(
        &self,
        media: Media,
        stream: Option<usize>,
        mix: Option<Vec<ChannelSource>>,
        remix: Option<u16>,
    ) -> Result<(Value, Option<WaveJob>), Fault> {
        if mix
            .as_ref()
            .is_some_and(|sources| sources.is_empty() || stream.is_some() || remix.is_some())
        {
            return Err(Fault::params(
                "mix must contain source channels and cannot be combined with stream or remix",
            ));
        }
        if remix.is_some_and(|channels| stream.is_some() || !(1..=24).contains(&channels)) {
            return Err(Fault::params(
                "remix takes 1 to 24 channels and cannot be combined with stream",
            ));
        }
        let shared = Arc::clone(&self.0);
        tokio::task::spawn_blocking(move || {
            let key = match &mix {
                Some(sources) => mix_key(&media, sources),
                None => match stream {
                    None => match remix {
                        Some(channels) => format!("{}-remix-{channels}.bin", stem(&media)),
                        None => format!("{}.bin", stem(&media)),
                    },
                    Some(stream) => format!("{}-s{stream}.bin", stem(&media)),
                },
            };
            let cache = cache_path(&shared.project, "waveforms", &key);
            let audio =
                media.facts.audio.as_ref().ok_or_else(|| {
                    cannot_read(&media.path, "no_audio", "the media has no sound")
                })?;
            let spec = match stream {
                None => Some(WaveSpec::new(
                    &media,
                    if mix.is_some() {
                        1
                    } else {
                        remix.unwrap_or(audio.channels)
                    },
                )?),
                Some(stream) => {
                    // The cache first, by the channels that its header says, so that the file
                    // is not opened for a waveform made before (nor needed: it may be offline).
                    if let Some(wave) = cache.as_deref().and_then(|path| {
                        let spec = WaveSpec::new(&media, cached_channels(path)?).ok()?;
                        cached_wave(path, &spec)
                    }) {
                        return Ok((wave.value(), None));
                    }
                    stream_spec(&media, stream)?
                }
            };
            if let (None, Some(spec), Some(path)) = (stream, &spec, &cache) {
                if let Some(wave) = cached_wave(path, spec) {
                    return Ok((wave.value(), None));
                }
            }
            // Without a cache, an absent source channel is refused now, before any job. A file
            // that cannot be opened is left to the job, which says why, as for the media.
            if let Some(sources) = &mix {
                if let Ok(sounds) = columbina_media::sounds(Path::new(&media.path)) {
                    mix_picks(&sounds, sources)?;
                }
            }
            let mut jobs = shared
                .jobs
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(index) = jobs.recent.iter().position(|(found, _)| found == &key) {
                let (_, result) = jobs.recent.remove(index).expect("the answer just found");
                drop(jobs);
                return result.map(|wave| (wave.value(), None));
            }
            let fresh = jobs.pending.insert(key.clone());
            drop(jobs);
            let job = fresh.then(|| WaveJob {
                shared,
                media,
                stream,
                mix,
                remix,
                key,
                spec,
            });
            Ok((json!({"pending":true}), job))
        })
        .await
        .map_err(worker_error)?
    }
}

impl WaveJob {
    pub fn start(self) {
        tokio::spawn(async move {
            let Ok(permit) = Arc::clone(&self.shared.wave_slots).acquire_owned().await else {
                return;
            };
            let _ = tokio::task::spawn_blocking(move || {
                let _permit = permit;
                let result = match &self.mix {
                    Some(sources) => make_mix(&self.media, sources, &self.shared.stopped),
                    None => make_wave(
                        &self.media,
                        self.stream,
                        self.remix,
                        self.spec.as_ref(),
                        &self.shared.stopped,
                    ),
                };
                let saved = result.as_ref().is_ok_and(|wave| {
                    cache_path(&self.shared.project, "waveforms", &self.key).is_some_and(|path| {
                        let mut bytes = Vec::with_capacity(WAVE_MAGIC.len() + 2 + wave.peaks.len());
                        bytes.extend_from_slice(WAVE_MAGIC);
                        bytes.extend_from_slice(&wave.channels.to_le_bytes());
                        bytes.extend_from_slice(&wave.peaks);
                        save(&path, &bytes)
                    })
                });
                let mut jobs = self
                    .shared
                    .jobs
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                jobs.pending.remove(&self.key);
                if !saved {
                    jobs.recent.push_back((self.key, result));
                    while jobs.recent.len() > 2 {
                        jobs.recent.pop_front();
                    }
                }
                drop(jobs);
                if !self.shared.stopped.load(Ordering::Acquire) {
                    (self.shared.notify)(&self.media, self.stream, self.mix.as_deref(), self.remix);
                }
            })
            .await;
        });
    }
}

fn worker_error(error: impl std::fmt::Display) -> Fault {
    Fault::new(rpc::CANNOT_READ, format!("media analysis stopped: {error}"))
}

struct ThumbSpec {
    frame: i64,
    width: u32,
    height: u32,
    rotation: i16,
}

impl ThumbSpec {
    fn new(media: &Media, request: &ThumbnailRequest) -> Result<Self, Fault> {
        if !(1..=270).contains(&request.height) {
            return Err(Fault::params("thumbnail height must be between 1 and 270"));
        }
        let video = media
            .facts
            .video
            .as_ref()
            .ok_or_else(|| cannot_read(&media.path, "no_video", "the media has no picture"))?;
        let duration = media.facts.duration;
        let count = ceil(
            i128::from(duration.ticks) * i128::from(video.rate.num),
            i128::from(duration.scale) * i128::from(video.rate.den),
        );
        let frame = request
            .frame
            .unwrap_or(i64::try_from(count / 2).unwrap_or(i64::MAX));
        if frame < 0 || i128::from(frame) >= count {
            return Err(Fault::params("thumbnail frame is outside the media")
                .with_data(json!({"reason":"out_of_range", "frame":frame})));
        }
        let (w, h) = if video.rotation % 180 != 0 {
            (video.height, video.width)
        } else {
            (video.width, video.height)
        };
        let width =
            ((u64::from(w) * u64::from(request.height) + u64::from(h) / 2) / u64::from(h)).max(1);
        if width > 16384 {
            return Err(Fault::params(
                "the thumbnail would be wider than 16384 pixels",
            ));
        }
        Ok(Self {
            frame,
            width: u32::try_from(width).expect("thumbnail width checked above"),
            height: request.height,
            rotation: video.rotation,
        })
    }
}

fn picture(media: &Media, spec: &ThumbSpec) -> Result<Rgba, Fault> {
    let decode = |path: &Path, is_proxy| -> Result<Rgba, MediaError> {
        let mut decoder = if is_proxy {
            VideoDecoder::open_at_rate(
                path,
                Hardware::Off,
                media.facts.video.as_ref().expect("checked video").rate,
            )?
        } else {
            VideoDecoder::open(path, Hardware::Off)?
        };
        let frame = decoder.decoded_at(spec.frame)?;
        let (w, h) = if spec.rotation % 180 != 0 {
            (spec.height, spec.width)
        } else {
            (spec.width, spec.height)
        };
        let mut converter = Converter::new();
        converter
            .rgba_scaled(&frame, w, h)
            .and_then(|rgba| upright(rgba, spec.rotation))
    };
    if let Some(proxy) = proxy::usable(media) {
        if let Ok(picture) = decode(Path::new(&proxy.path), true) {
            return Ok(picture);
        }
    }
    decode(Path::new(&media.path), false)
        .map_err(|error| cannot_read(&media.path, error.reason(), &error.to_string()))
}

/// A picture turned clockwise by `rotation`, a quarter turn or more, to be shown upright. The
/// model holds no other rotation; one that is not a quarter turn is refused, not drawn.
fn upright(source: Rgba, rotation: i16) -> Result<Rgba, MediaError> {
    if rotation == 0 {
        return Ok(source);
    }
    if !matches!(rotation, 90 | 180 | 270) {
        return Err(MediaError::Unsupported(format!(
            "a picture turned by {rotation} degrees, not a quarter turn"
        )));
    }
    let (width, height) = if rotation % 180 != 0 {
        (source.height, source.width)
    } else {
        (source.width, source.height)
    };
    let mut result = Rgba {
        width,
        height,
        data: vec![0; source.data.len()],
    };
    for y in 0..source.height {
        for x in 0..source.width {
            let (dx, dy) = match rotation {
                90 => (source.height - 1 - y, x),
                180 => (source.width - 1 - x, source.height - 1 - y),
                _ => (y, source.width - 1 - x),
            };
            let from = ((y * source.width + x) * 4) as usize;
            let to = ((dy * width + dx) * 4) as usize;
            result.data[to..to + 4].copy_from_slice(&source.data[from..from + 4]);
        }
    }
    Ok(result)
}

fn encode_png(rgba: &Rgba) -> Result<Vec<u8>, String> {
    let mut data = Vec::new();
    let mut encoder = png::Encoder::new(&mut data, rgba.width, rgba.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .and_then(|mut writer| writer.write_image_data(&rgba.data))
        .map_err(|error| error.to_string())?;
    Ok(data)
}

fn cached_png(path: &Path, spec: &ThumbSpec) -> Option<Vec<u8>> {
    let bytes = read_cache(path, MAX_PEAK_BYTES)?;
    let mut decoder = png::Decoder::new(Cursor::new(&bytes));
    decoder.set_limits(png::Limits {
        bytes: 64 * 1024 * 1024,
    });
    let mut reader = decoder.read_info().ok()?;
    let info = reader.info();
    if (info.width, info.height, info.color_type, info.bit_depth)
        != (
            spec.width,
            spec.height,
            png::ColorType::Rgba,
            png::BitDepth::Eight,
        )
    {
        return None;
    }
    let mut pixels = vec![0; reader.output_buffer_size()?];
    reader.next_frame(&mut pixels).ok()?;
    Some(bytes)
}

struct WaveSpec {
    channels: u16,
    duration: Time,
    bytes: usize,
}

/// The size of the waveform of stream `stream` of the media, from the channels that its file
/// says the stream has. A stream that the file does not have as sound is refused; where the file
/// cannot be opened, none, so that the job says why, as it does for the sound of the media.
fn stream_spec(media: &Media, stream: usize) -> Result<Option<WaveSpec>, Fault> {
    let Ok(sounds) = columbina_media::sounds(Path::new(&media.path)) else {
        return Ok(None);
    };
    let channels = sounds
        .streams
        .iter()
        .find(|found| found.index == stream)
        .map(|found| found.channels)
        .ok_or_else(|| {
            Fault::params(format!("the media has no stream of sound {stream}"))
                .with_data(json!({"reason":"stream", "stream":stream}))
        })?;
    WaveSpec::new(media, channels).map(Some)
}

impl WaveSpec {
    /// The waveform of `channels` channels of the sound of the media, as long as the media.
    fn new(media: &Media, channels: u16) -> Result<Self, Fault> {
        let duration = media.facts.duration;
        let bins = ceil(
            i128::from(duration.ticks) * i128::from(RATE),
            i128::from(duration.scale),
        );
        let bytes = bins * i128::from(channels) * 2;
        if !(0..=MAX_PEAK_BYTES as i128).contains(&bytes) {
            return Err(cannot_read(
                &media.path,
                "too_large",
                "the waveform exceeds one protocol message",
            ));
        }
        Ok(Self {
            channels,
            duration,
            bytes: usize::try_from(bytes).expect("waveform size checked above"),
        })
    }
}

fn make_wave(
    media: &Media,
    stream: Option<usize>,
    remix: Option<u16>,
    spec: Option<&WaveSpec>,
    stopped: &AtomicBool,
) -> Result<Wave, Fault> {
    let path = Path::new(&media.path);
    let cannot = |error: MediaError| cannot_read(&media.path, error.reason(), &error.to_string());
    let decoder = match stream {
        None => AudioDecoder::open(
            path,
            remix.map(|channels| columbina_media::AudioFormat {
                channels,
                rate: media.facts.audio.as_ref().expect("checked audio").rate,
            }),
        ),
        Some(stream) => AudioDecoder::open_unmixed(path, stream, None),
    };
    let decoder = match decoder {
        Ok(decoder) => decoder,
        // Opened now, the file has no such stream of sound.
        Err(MediaError::NoStreams) if stream.is_some() => {
            return Err(cannot_read(
                &media.path,
                "no_audio",
                "the media has no such stream of sound",
            ))
        }
        Err(error) => return Err(cannot(error)),
    };
    let made;
    let spec = if let Some(spec) = spec {
        spec
    } else {
        made = WaveSpec::new(media, decoder.format().channels)?;
        &made
    };
    let read = |mut decoder: AudioDecoder| -> Result<Wave, MediaError> {
        let format = decoder.format();
        if format.channels != spec.channels {
            return Err(MediaError::Unsupported(
                "the number of audio channels changed since import".to_owned(),
            ));
        }
        // Every stream of sound is placed by the start of the best one, as its clips are.
        let origin = if media.facts.video.is_some() {
            columbina_media::video_start(path)?
        } else if stream.is_some() {
            columbina_media::sound_start(path)?
        } else {
            decoder.start()
        };
        let origin = i64::try_from(
            (i128::from(origin.ticks) * i128::from(format.rate))
                .div_euclid(i128::from(origin.scale)),
        )
        .map_err(|_| {
            MediaError::Unsupported("the audio start is outside sample time".to_owned())
        })?;
        // A waveform reads the whole stream once. Keep the decoder's continuous sample
        // clock: seeking directly to a video start in Matroska's millisecond time base can
        // shift PCM by a fraction of a packet, because packet timestamps are rounded.
        let mut peaks = Peaks::new(spec, format.rate);
        while !stopped.load(Ordering::Acquire) {
            let Some(samples) = decoder.next()? else {
                break;
            };
            peaks.add(
                i128::from(samples.start) - i128::from(origin),
                &samples.data,
            );
            if peaks.next >= peaks.frames {
                break;
            }
        }
        if stopped.load(Ordering::Acquire) {
            return Err(MediaError::Unsupported("analysis cancelled".to_owned()));
        }
        Ok(Wave {
            channels: spec.channels,
            peaks: peaks.finish(),
        })
    };
    read(decoder).map_err(cannot)
}

/// The longest source list written into a cache name as it is. A longer one is hashed, so
/// that the name stays far below the 260 characters of a Windows path.
const MOST_MIX_NAME: usize = 64;

/// The job and cache name of a mix: its exact ordered source list, or a hash of a long one.
/// A hashed name starts with `h`, which a written list (digits, `.` and `-`) never does.
fn mix_key(media: &Media, sources: &[ChannelSource]) -> String {
    use sha2::{Digest as _, Sha256};
    let list = sources
        .iter()
        .map(|source| format!("{}.{}", source.stream, source.channel))
        .collect::<Vec<_>>()
        .join("-");
    if list.len() <= MOST_MIX_NAME {
        return format!("{}-mix-{list}.bin", stem(media));
    }
    let hash = Sha256::digest(list.as_bytes());
    let short = hash[..16].iter().fold(String::new(), |mut short, b| {
        use std::fmt::Write as _;
        let _ = write!(short, "{b:02x}");
        short
    });
    format!("{}-mix-h{short}.bin", stem(media))
}

/// The native channels of a mix, and the highest sample rate among their streams; a source
/// that is not a channel of a stream of sound of the file is refused as `-32602`.
fn mix_picks(
    sounds: &columbina_media::Sounds,
    sources: &[ChannelSource],
) -> Result<(Vec<crate::play::mix::reader::Pick>, u32), Fault> {
    let mut rate = 0;
    let mut picks = Vec::with_capacity(sources.len());
    for source in sources {
        let sound = sounds
            .streams
            .iter()
            .find(|sound| u32::try_from(sound.index) == Ok(source.stream))
            .ok_or_else(|| {
                Fault::params("mix names a stream that is not a stream of sound of the media")
                    .with_data(json!({"reason":"stream", "stream":source.stream}))
            })?;
        let channel = u16::try_from(source.channel)
            .ok()
            .filter(|channel| *channel < sound.channels)
            .ok_or_else(|| {
                Fault::params("mix names a channel absent from the source").with_data(
                    json!({"reason":"channel", "stream":source.stream, "channel":source.channel}),
                )
            })?;
        rate = rate.max(sound.rate);
        picks.push(crate::play::mix::reader::Pick {
            stream: sound.index,
            channel,
        });
    }
    Ok((picks, rate))
}

/// Mix actual samples before reducing them: peaks cannot recover phase cancellation.
/// The playback reader keeps bounded PCM windows and aligns all streams to the video origin.
fn make_mix(media: &Media, sources: &[ChannelSource], stopped: &AtomicBool) -> Result<Wave, Fault> {
    use crate::play::mix::reader::{Lane, Layout, Reader};
    let cannot = |message: &str| cannot_read(&media.path, "decode", message);
    let sounds = columbina_media::sounds(Path::new(&media.path))
        .map_err(|error| cannot_read(&media.path, error.reason(), &error.to_string()))?;
    let (picks, rate) = mix_picks(&sounds, sources)?;
    if rate == 0 {
        return Err(cannot("the mix has no sample rate"));
    }
    let spec = WaveSpec::new(media, 1)?;
    let mut peaks = Peaks::new(&spec, rate);
    let mut reader = Reader::new(
        media,
        rate,
        &Layout::Mapped(vec![Lane::mean(picks, true, false, 0.0)]),
    )
    .map_err(|message| cannot(&message))?;
    while peaks.next < peaks.frames {
        if stopped.load(Ordering::Acquire) {
            return Err(cannot("analysis cancelled"));
        }
        let from = i64::try_from(peaks.next).map_err(|_| cannot("mix exceeds sample time"))?;
        let to = i64::try_from((peaks.next + 4096).min(peaks.frames))
            .map_err(|_| cannot("mix exceeds sample time"))?;
        let samples = reader.read(from..to).map_err(|message| cannot(&message))?;
        peaks.add(i128::from(from), &samples);
    }
    Ok(Wave {
        channels: 1,
        peaks: peaks.finish(),
    })
}

/// Reduce every native sample, without downsampling away narrow peaks. Missing samples are
/// silence; overlapping decoder packets do not count twice. Bins start at media time zero.
struct Peaks {
    rate: u32,
    channels: usize,
    frames: u64,
    next: u64,
    values: Vec<i8>,
}

impl Peaks {
    fn new(spec: &WaveSpec, rate: u32) -> Self {
        let frames = u64::try_from(ceil(
            i128::from(spec.duration.ticks) * i128::from(rate),
            i128::from(spec.duration.scale),
        ))
        .expect("bounded duration times u32 sample rate fits u64");
        let mut values = vec![0; spec.bytes];
        for pair in values.as_chunks_mut::<2>().0.iter_mut() {
            pair.copy_from_slice(&[127, -128]);
        }
        Self {
            rate,
            channels: usize::from(spec.channels),
            frames,
            next: 0,
            values,
        }
    }

    fn bin(&self, sample: u64) -> usize {
        usize::try_from(sample * u64::from(RATE) / u64::from(self.rate))
            .expect("bin is within the allocated peaks")
    }

    fn silence_until(&mut self, sample: u64) {
        let end = sample.min(self.frames);
        if self.next < end {
            let from = self.bin(self.next) * self.channels * 2;
            let to = (self.bin(end - 1) + 1) * self.channels * 2;
            for pair in self.values[from..to].as_chunks_mut::<2>().0.iter_mut() {
                pair[0] = pair[0].min(0);
                pair[1] = pair[1].max(0);
            }
            self.next = end;
        }
    }

    fn add(&mut self, start: i128, samples: &[f32]) {
        if start > i128::from(self.next) {
            self.silence_until(u64::try_from(start).unwrap_or(u64::MAX));
        }
        let skip = usize::try_from((i128::from(self.next) - start).max(0)).unwrap_or(usize::MAX);
        for frame in samples.chunks_exact(self.channels).skip(skip) {
            if self.next >= self.frames {
                break;
            }
            let bin = self.bin(self.next) * self.channels * 2;
            for (channel, sample) in frame.iter().enumerate() {
                #[allow(
                    clippy::cast_possible_truncation,
                    reason = "rounded to an integer in -127..=127"
                )]
                let value = if sample.is_finite() {
                    (sample.clamp(-1.0, 1.0) * 127.0).round() as i8
                } else {
                    0
                };
                let pair = &mut self.values[bin + channel * 2..bin + channel * 2 + 2];
                pair[0] = pair[0].min(value);
                pair[1] = pair[1].max(value);
            }
            self.next += 1;
        }
    }

    fn finish(mut self) -> Vec<u8> {
        self.silence_until(self.frames);
        // At rates below 100 Hz some bins have no native samples.
        for pair in self.values.as_chunks_mut::<2>().0.iter_mut() {
            if pair[0] > pair[1] {
                pair.copy_from_slice(&[0, 0]);
            }
        }
        self.values.into_iter().map(i8::cast_unsigned).collect()
    }
}

/// The channels of a waveform in a cache, from its header.
fn cached_channels(path: &Path) -> Option<u16> {
    let header = WAVE_MAGIC.len() + 2;
    let mut bytes = vec![0; header];
    std::fs::File::open(path)
        .ok()?
        .read_exact(&mut bytes)
        .ok()?;
    bytes
        .starts_with(WAVE_MAGIC)
        .then(|| u16::from_le_bytes([bytes[header - 2], bytes[header - 1]]))
}

fn cached_wave(path: &Path, spec: &WaveSpec) -> Option<Wave> {
    let header = WAVE_MAGIC.len() + 2;
    let bytes = read_cache(path, header + spec.bytes)?;
    if bytes.len() != header + spec.bytes || !bytes.starts_with(WAVE_MAGIC) {
        return None;
    }
    let channels = u16::from_le_bytes(bytes[WAVE_MAGIC.len()..header].try_into().ok()?);
    if channels != spec.channels
        || bytes[header..]
            .as_chunks::<2>()
            .0
            .iter()
            .any(|pair| pair[0].cast_signed() > pair[1].cast_signed())
    {
        return None;
    }
    Some(Wave {
        channels,
        peaks: bytes[header..].to_vec(),
    })
}

/// Reuse the identity recorded at import. Encoding the id keeps paths inside the cache and
/// distinguishes ids containing punctuation, without introducing another content digest.
fn stem(media: &Media) -> String {
    let mut id = String::new();
    for byte in media.id.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_') {
            id.push(char::from(byte));
        } else {
            use std::fmt::Write as _;
            let _ = write!(id, "%{byte:02X}");
        }
    }
    // The model refuses a digest that is not 64 lowercase hex digits; should one ever reach
    // here, a digest of it still names one file without a panic or a path out of the cache.
    let digest = &media.identity.digest;
    match digest.get(..12) {
        Some(short)
            if short
                .bytes()
                .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) =>
        {
            format!("{id}-{short}")
        }
        _ => {
            use sha2::{Digest as _, Sha256};
            let hash = Sha256::digest(digest.as_bytes());
            let short = hash[..6].iter().fold(String::new(), |mut short, b| {
                use std::fmt::Write as _;
                let _ = write!(short, "{b:02x}");
                short
            });
            format!("{id}-{short}")
        }
    }
}

/// Never follow a cache directory that a link or junction redirects outside the project.
/// A read-only cache is optional: the computed answer is still returned.
fn cache_path(project: &Path, kind: &str, name: &str) -> Option<PathBuf> {
    let root = project.canonicalize().ok()?;
    let mut dir = root.clone();
    for part in ["cache", kind] {
        dir.push(part);
        match std::fs::create_dir(&dir) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => return None,
        }
        if dir.canonicalize().ok()? != dir {
            return None;
        }
    }
    Some(dir.join(name))
}

fn read_cache(path: &Path, limit: usize) -> Option<Vec<u8>> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > limit as u64 {
        return None;
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() <= limit).then_some(bytes)
}

fn save(path: &Path, bytes: &[u8]) -> bool {
    let write = || -> std::io::Result<()> {
        let mut file = tempfile::NamedTempFile::new_in(path.parent().expect("cache directory"))?;
        file.write_all(bytes)?;
        file.persist(path).map_err(|error| error.error)?;
        Ok(())
    };
    if let Err(error) = write() {
        log(
            "analysis_cache",
            &json!({"path":path, "message":error.to_string()}),
        );
        false
    } else {
        true
    }
}

/// Positive project time, rounded up without floating-point conversion.
fn ceil(over: i128, under: i128) -> i128 {
    (over + under - 1) / under
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(channels: u16, ticks: i64, scale: u32) -> WaveSpec {
        WaveSpec {
            channels,
            duration: Time { ticks, scale },
            bytes: usize::try_from(ceil(i128::from(ticks) * 100, i128::from(scale))).unwrap()
                * usize::from(channels)
                * 2,
        }
    }

    #[test]
    fn peaks_keep_single_sample_transients_and_channels_at_exact_bin_edges() {
        let mut peaks = Peaks::new(&spec(2, 21, 1000), 44_100);
        let mut samples = [0.25, -0.5].repeat(927);
        samples[440 * 2] = 1.0; // Last native sample of the first 10 ms.
        samples[441 * 2 + 1] = -1.0; // First native sample of the next 10 ms.
        peaks.add(0, &samples[..400]);
        peaks.add(200, &samples[400..]); // Packet boundaries do not reset the bins.
        let got: Vec<i8> = peaks.finish().into_iter().map(u8::cast_signed).collect();
        assert_eq!(
            got,
            [32, 127, -64, -64, 32, 32, -127, -64, 32, 32, -64, -64]
        );
    }

    #[test]
    fn peaks_trim_preroll_and_padding_and_fill_missing_sound_with_silence() {
        let mut peaks = Peaks::new(&spec(1, 25, 1000), 1000);
        peaks.add(-10, &[1.0; 15]); // Only the first 5 ms are in the media.
        peaks.add(15, &[-0.5; 30]); // Gap 5..15 ms; padding after 25 ms is ignored.
        let got: Vec<i8> = peaks.finish().into_iter().map(u8::cast_signed).collect();
        assert_eq!(got, [0, 127, -64, 0, -64, -64]);
        let mut tail = Peaks::new(&spec(1, 25, 1000), 1000);
        tail.add(0, &[1.0; 15]);
        assert_eq!(tail.finish(), [127, 127, 0, 127, 0, 0]);
    }

    #[test]
    fn quarter_turns_put_pixels_in_their_display_positions() {
        let source = Rgba {
            width: 3,
            height: 2,
            data: (1..=6).flat_map(|x| [x, x, x, 255]).collect(),
        };
        for (turn, shape, expected) in [
            (90, (2, 3), vec![4, 1, 5, 2, 6, 3]),
            (180, (3, 2), vec![6, 5, 4, 3, 2, 1]),
            (270, (2, 3), vec![3, 6, 2, 5, 1, 4]),
        ] {
            let result = upright(source.clone(), turn).unwrap();
            assert_eq!((result.width, result.height), shape);
            assert_eq!(
                result
                    .data
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|pixel| pixel[0])
                    .collect::<Vec<_>>(),
                expected
            );
        }
    }

    #[test]
    fn a_turn_that_is_not_a_quarter_turn_is_refused_not_drawn() {
        let source = Rgba {
            width: 3,
            height: 2,
            data: vec![0; 24],
        };
        for turn in [45, -90, 360] {
            let error = upright(source.clone(), turn).unwrap_err();
            assert!(matches!(error, MediaError::Unsupported(_)), "{error}");
        }
    }

    #[test]
    fn a_cache_name_takes_12_digits_of_the_digest_and_never_panics_on_a_bad_one() {
        let mut media = columbina_model::Project::sample().media[0].clone();
        media.id = "m/1".into();
        media.identity.digest = "0123456789abcdef".repeat(4);
        assert_eq!(stem(&media), "m%2F1-0123456789ab");
        // Shorter than 12 bytes, cut inside a character, or not hex: still one file name.
        for digest in ["", "abc", "ééééééééééé", "ABCDEFABCDEF00"] {
            media.identity.digest = digest.into();
            let name = stem(&media);
            assert!(name.starts_with("m%2F1-") && name.len() == 18, "{name}");
            assert!(name[6..].bytes().all(|b| b.is_ascii_hexdigit()), "{name}");
        }
    }

    #[test]
    fn cache_rejects_truncated_wrong_channel_and_reversed_peak_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = cache_path(dir.path(), "waveforms", "m1.bin").unwrap();
        let spec = spec(1, 10, 1000);
        for bytes in [
            b"CWAV1\x01\x00\x7f".as_slice(),
            b"CWAV1\x02\x00\x00\x7f",
            b"CWAV1\x01\x00\x7f\x00",
        ] {
            assert!(save(&path, bytes));
            assert!(cached_wave(&path, &spec).is_none());
        }
        assert!(save(&path, b"CWAV1\x01\x00\x81\x7f"));
        assert_eq!(cached_wave(&path, &spec).unwrap().peaks, [129, 127]);
    }

    #[cfg(unix)]
    #[test]
    fn cache_does_not_read_or_write_through_directory_or_file_symlinks() {
        let project = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), project.path().join("cache")).unwrap();
        assert!(cache_path(project.path(), "thumbs", "m1.png").is_none());
        assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
        let original = outside.path().join("data");
        std::fs::write(&original, b"source").unwrap();
        let linked = project.path().join("link");
        std::os::unix::fs::symlink(&original, &linked).unwrap();
        assert!(read_cache(&linked, 100).is_none());
        assert!(save(&linked, b"cache"));
        assert_eq!(std::fs::read(&original).unwrap(), b"source");
    }
}
