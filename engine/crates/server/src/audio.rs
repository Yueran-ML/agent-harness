//! The sound of the transport: the sound of a media item, decoded from a place, converted to
//! what the output device of the system takes, and played on it with cpal.
//!
//! The device is the default output of the system, opened in its own format (its mix format on
//! Windows, mostly 48 kHz in floats), and the sound is converted to its rate and channels by
//! [`AudioDecoder`] (swresample: stereo is mixed down to mono, or laid on the front left and
//! right of more channels). A **feeder** thread decodes ahead of the device, up to
//! [`AHEAD_SECONDS`], into a queue that the device's own thread takes from as it plays.
//!
//! The device is the clock of playing with sound: [`Output::played`] says how much of the sound
//! that was started has been heard, from the times that the device says each run it is given
//! will be played (its latency counted). Where the feeder falls behind, the device plays
//! silence, which is not counted, so that the clock waits for the sound. After the end of the
//! sound, silence is counted, so that a picture that lasts longer than its sound goes on.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use columbina_media::{AudioDecoder, AudioFormat};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, OutputCallbackInfo, SampleFormat, SizedSample, StreamConfig};
use serde_json::json;

use crate::log::log;

/// How far the feeder decodes ahead of the device.
pub const AHEAD_SECONDS: f64 = 0.25;

/// How far the time of the sound is reckoned on past the last run given to the device.
const RUN_AHEAD: Duration = Duration::from_millis(100);

/// How long the feeder sleeps when it is far enough ahead.
const FEEDER_NAP: Duration = Duration::from_millis(5);

/// The output device, opened, and what plays on it.
pub struct Output {
    stream: cpal::Stream,
    shared: Arc<Shared>,
    format: AudioFormat,
    /// The name of the device, for the log.
    name: String,
    feeder: Option<Feeder>,
    /// The most played that was told, so that what is told never goes back.
    told: Mutex<u64>,
}

/// What the device's thread, the feeder and the transport share.
struct Shared {
    queue: Mutex<Queue>,
    /// What the device was last given.
    mark: Mutex<Option<Mark>>,
    /// Why the device failed, once it did.
    failed: Mutex<Option<String>>,
    channels: usize,
    /// Whether silence is played in the place of the sound (`--mute`, for the tests): the
    /// clock goes on as if it were heard.
    mute: bool,
}

/// The samples waiting to be played.
#[derive(Default)]
struct Queue {
    /// Interleaved, `channels` an instant.
    samples: VecDeque<f32>,
    /// Which start the samples are of: a feeder of an earlier one adds nothing.
    generation: u64,
    /// How many instants of sound the device was given since the start.
    played: u64,
}

/// What the device was given in its last run: the instants of sound before it, how many of its
/// first instants were sound (the rest was silence), whether all of them were, and when the
/// first is heard.
#[derive(Clone, Copy)]
struct Mark {
    before: u64,
    sound: u64,
    full: bool,
    heard: Instant,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The thread that decodes ahead of the device.
struct Feeder {
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

impl Output {
    /// Open the default output device of the system, in its own format, not playing yet.
    ///
    /// # Errors
    /// There is no output device, or it cannot be opened in a format of samples that this
    /// takes (floats, or integers of 8 to 32 bits).
    pub fn open(mute: bool) -> Result<Output, String> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or("there is no output device of sound")?;
        let name = device.to_string();
        let supported = device
            .default_output_config()
            .map_err(|error| format!("the output device {name} cannot be opened: {error}"))?;
        let config: StreamConfig = supported.config();
        let format = AudioFormat {
            rate: config.sample_rate,
            channels: config.channels,
        };
        if format.rate == 0 || format.channels == 0 {
            return Err(format!("the output device {name} has no format"));
        }
        let shared = Arc::new(Shared {
            queue: Mutex::new(Queue::default()),
            mark: Mutex::new(None),
            failed: Mutex::new(None),
            channels: usize::from(format.channels),
            mute,
        });
        let stream = match supported.sample_format() {
            SampleFormat::F32 => build::<f32>(&device, config, &shared),
            SampleFormat::F64 => build::<f64>(&device, config, &shared),
            SampleFormat::I16 => build::<i16>(&device, config, &shared),
            SampleFormat::I32 => build::<i32>(&device, config, &shared),
            SampleFormat::U16 => build::<u16>(&device, config, &shared),
            SampleFormat::I8 => build::<i8>(&device, config, &shared),
            SampleFormat::U8 => build::<u8>(&device, config, &shared),
            other => {
                return Err(format!(
                    "the output device {name} takes samples of {other}, which are not played"
                ))
            }
        }
        .map_err(|error| format!("the output device {name} cannot be opened: {error}"))?;
        Ok(Output {
            stream,
            shared,
            format,
            name,
            feeder: None,
            told: Mutex::new(0),
        })
    }

    /// The rate and channels that the device plays.
    pub fn format(&self) -> AudioFormat {
        self.format
    }

    /// The name of the device.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Play the sound of the file at `path` from sample `from` (at the device's rate, from time
    /// 0 of the file, as [`AudioDecoder`] numbers them), in the place of what played before.
    /// It returns at once: the feeder opens the file, and the device plays silence, which is
    /// not counted as played, until the first samples come.
    ///
    /// # Errors
    /// The device cannot be started, or a thread cannot be started.
    pub fn start(&mut self, path: &Path, from: i64) -> Result<(), String> {
        self.stop();
        let generation = {
            let mut queue = lock(&self.shared.queue);
            queue.generation += 1;
            queue.samples.clear();
            queue.played = 0;
            queue.generation
        };
        *lock(&self.shared.mark) = None;
        *lock(&self.told) = 0;
        let stop = Arc::new(AtomicBool::new(false));
        let thread = std::thread::Builder::new()
            .name("columbina-sound".into())
            .spawn({
                let shared = Arc::clone(&self.shared);
                let stop = Arc::clone(&stop);
                let path = path.to_owned();
                let format = self.format;
                move || feed(&shared, &stop, generation, &path, format, from)
            })
            .map_err(|error| format!("the feeder of the sound cannot be started: {error}"))?;
        self.feeder = Some(Feeder { stop, thread });
        self.stream
            .play()
            .map_err(|error| format!("the output device {} cannot play: {error}", self.name))
    }

    /// Stop playing at once, and let what was waiting to be played go.
    pub fn stop(&mut self) {
        let _ = self.stream.pause();
        if let Some(feeder) = self.feeder.take() {
            feeder.stop.store(true, Ordering::Relaxed);
            // It ends within a decode of a run of sound; a thread that hangs in libav is left.
            drop(feeder.thread);
        }
        let mut queue = lock(&self.shared.queue);
        queue.generation += 1;
        queue.samples.clear();
    }

    /// How many instants of the sound started last have been heard by `now`, at the device's
    /// rate; never fewer than told before. None until the first of them is handed to the
    /// device.
    ///
    /// It is reckoned from the last run that the device was given: the instants before it, and
    /// those of it heard since its first was. While the device is given sound and nothing but,
    /// the time goes on past that run as it plays on, up to [`RUN_AHEAD`] (its next run is
    /// asked for only once it needs it); where the run ended in silence, the feeder having
    /// fallen behind, the time stops at its end until more sound comes.
    pub fn played(&self, now: Instant) -> Option<u64> {
        let mark = (*lock(&self.shared.mark))?;
        if mark.before == 0 && mark.sound == 0 {
            return None;
        }
        let since = now.saturating_duration_since(mark.heard);
        let rate = u128::from(self.format.rate);
        let into = u64::try_from(since.as_nanos() * rate / 1_000_000_000).unwrap_or(u64::MAX);
        let most = if mark.full {
            mark.sound + u64::try_from(RUN_AHEAD.as_nanos() * rate / 1_000_000_000).unwrap_or(0)
        } else {
            mark.sound
        };
        let played = mark.before + into.min(most);
        let mut told = lock(&self.told);
        *told = (*told).max(played);
        Some(*told)
    }

    /// Why the device failed while it played, if it did: then it plays no more.
    pub fn failed(&self) -> Option<String> {
        lock(&self.shared.failed).clone()
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Make a stream of the device that plays samples of `T`, taken from the queue.
fn build<T>(
    device: &cpal::Device,
    config: StreamConfig,
    shared: &Arc<Shared>,
) -> Result<cpal::Stream, cpal::Error>
where
    T: SizedSample + FromSample<f32> + Send + 'static,
{
    let playing = Arc::clone(shared);
    let failing = Arc::clone(shared);
    device.build_output_stream::<T, _, _>(
        config,
        move |data: &mut [T], info: &OutputCallbackInfo| playing.fill(data, info),
        move |error| {
            let message = error.to_string();
            log("sound_failed", &json!({ "message": message }));
            *lock(&failing.failed) = Some(message);
        },
        None,
    )
}

impl Shared {
    /// Fill a run of the device with what waits in the queue, and silence after it; mark how
    /// much of it is sound, and when it is heard.
    fn fill<T: SizedSample + FromSample<f32>>(&self, data: &mut [T], info: &OutputCallbackInfo) {
        let stamp = info.timestamp();
        let latency = stamp
            .playback
            .checked_duration_since(stamp.callback)
            .unwrap_or_default();
        let heard = Instant::now() + latency;
        let mut queue = lock(&self.queue);
        let instants = data.len() / self.channels;
        let available = queue.samples.len() / self.channels;
        let sound = instants.min(available);
        let floats = sound * self.channels;
        for (slot, sample) in data.iter_mut().zip(queue.samples.drain(..floats)) {
            *slot = T::from_sample(if self.mute { 0.0 } else { sample });
        }
        for slot in &mut data[floats..] {
            *slot = T::EQUILIBRIUM;
        }
        let before = queue.played;
        let sound = u64::try_from(sound).unwrap_or(u64::MAX);
        queue.played += sound;
        drop(queue);
        *lock(&self.mark) = Some(Mark {
            before,
            sound,
            full: sound == u64::try_from(instants).unwrap_or(u64::MAX),
            heard,
        });
    }
}

/// Decode the sound of a file from `from` into the queue, ahead of the device, until it is
/// stopped; after its end, or if it cannot be decoded, silence.
fn feed(
    shared: &Shared,
    stop: &AtomicBool,
    generation: u64,
    path: &Path,
    format: AudioFormat,
    from: i64,
) {
    let mut decoder = match AudioDecoder::open(path, Some(format))
        .and_then(|mut decoder| decoder.seek(from).map(|()| decoder))
    {
        Ok(decoder) => Some(decoder),
        Err(error) => {
            log(
                "sound_failed",
                &json!({ "message": format!("the sound cannot be decoded: {error}") }),
            );
            None
        }
    };
    let channels = usize::from(format.channels);
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a quarter of a second at a rate of samples"
    )]
    let ahead = (f64::from(format.rate) * AHEAD_SECONDS) as usize * channels;
    // 20 ms of silence at a time, after the end.
    let silence = vec![0.0; (format.rate as usize / 50).max(1) * channels];
    while !stop.load(Ordering::Relaxed) {
        if lock(&shared.queue).samples.len() >= ahead {
            std::thread::sleep(FEEDER_NAP);
            continue;
        }
        let run = match decoder.as_mut().map(AudioDecoder::next) {
            Some(Ok(Some(samples))) => samples.data,
            Some(Ok(None)) | None => silence.clone(),
            Some(Err(error)) => {
                log(
                    "sound_failed",
                    &json!({ "message": format!("the sound cannot be decoded on: {error}") }),
                );
                decoder = None;
                silence.clone()
            }
        };
        let mut queue = lock(&shared.queue);
        if queue.generation != generation {
            return;
        }
        queue.samples.extend(run);
    }
}
