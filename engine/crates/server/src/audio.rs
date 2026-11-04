//! The sound of either monitor, supplied in the output device's format and played with cpal.
//!
//! The device is the default output of the system, opened in its own format (its mix format on
//! Windows, mostly 48 kHz in floats). Each start plays a **session**: a ring of the device's
//! frames (`columbina_audio::rt` rings, plan T5) that the device's own thread takes from as it
//! plays ([`columbina_audio::play::device::Player`]). A file is read into it by a **feeder**
//! thread, up to [`AHEAD_SECONDS`] ahead ([`Output::start`], the source monitor); the timeline is
//! mixed into it by the thread of the mix of `play/sound.rs` ([`Output::begin`]). The callback of
//! the device neither locks, takes memory nor waits: the sessions reach it through a ring, and
//! what it gave reaches the transport through a seqlock.
//!
//! The device is the clock of playing with sound: [`Output::played`] says how much of the sound
//! that was started has been heard, from the times that the device says each run it is given
//! will be played (its latency counted). Where the ring runs dry, the device plays silence,
//! which is not counted, so that the clock waits for the sound. After the end of the sound,
//! silence is counted, so that a picture that lasts longer than its sound goes on.
//!
//! The first run that a device asks for once a session starts is filled with silence, which is
//! not counted: WASAPI says that it is heard at once (a latency of 0) while it is in fact laid
//! ahead of what plays, some 20 ms, so that sound in it would be counted as heard that much too
//! early, and the clock and `sound_minus_system_ms` would be off by as much.
//!
//! A device that fails is never used again: [`Output::failed`] says so, and the transport opens
//! the default device anew the next time it plays, as it does when the default device of the
//! system changed ([`Output::current`]). A device that asks for no sound within [`FIRST_RUN`]
//! of being started (a Bluetooth headset waking takes some 300 ms), that stops asking for
//! [`SILENT_RUN`] once its sound has started to be heard, or that has let none of it be heard
//! [`NO_SOUND`] after sound was first waiting for it though it goes on asking, has failed too
//! ([`Output::failed`]): the picture then plays by the clock of the system.
//!
//! Besides the device, [`Kind`] has outputs of no device, for the tests and for machines with
//! no sound (`--sound null|stall|fail-after=<ms>|capture=<path>`): one that takes the sound in
//! runs of 10 ms timed by the clock of the system, as a device would, heard 20 ms later; one that
//! never asks for sound; one like the first that fails once it has played so long; and one like
//! the first that also writes every frame of sound that it plays into a file (T5, for the test
//! of T6 that what is heard is what is exported).

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use columbina_audio::play::device::{self, Feeding, Mark, Order, Player};
use columbina_audio::rt::{samples as ring, SampleConsumer, SampleProducer};
use columbina_media::{AudioDecoder, AudioFormat};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, OutputCallbackInfo, SampleFormat, SizedSample, StreamConfig};
use serde_json::json;

use crate::log::log;

/// How far the feeder decodes ahead of the device.
pub const AHEAD_SECONDS: f64 = 0.25;

/// How much the ring of a session holds: more than the feeder keeps ahead, and than the thread of
/// the mix keeps ahead (30 ms, or two runs of the device).
const RING_SECONDS: f64 = 0.5;

/// Interleaved samples in the output device's format. Opening and decoding happen on the
/// feeder thread. None marks the end; the output continues with silence after it.
pub trait Source: Send {
    fn next(&mut self) -> Result<Option<Vec<f32>>, String>;
}

/// A single-file source keeps the phase-0 source monitor's decoder and seek behaviour.
pub fn file(path: &Path, format: AudioFormat, from: i64) -> Box<dyn Source> {
    Box::new(File {
        path: path.to_owned(),
        format,
        from,
        decoder: None,
    })
}

struct File {
    path: PathBuf,
    format: AudioFormat,
    from: i64,
    decoder: Option<AudioDecoder>,
}

impl Source for File {
    fn next(&mut self) -> Result<Option<Vec<f32>>, String> {
        if self.decoder.is_none() {
            let mut decoder = AudioDecoder::open(&self.path, Some(self.format))
                .map_err(|error| error.to_string())?;
            decoder.seek(self.from).map_err(|error| error.to_string())?;
            self.decoder = Some(decoder);
        }
        self.decoder
            .as_mut()
            .expect("opened above")
            .next()
            .map(|samples| samples.map(|samples| samples.data))
            .map_err(|error| error.to_string())
    }
}

/// How far the time of the sound is reckoned on past the last run given to the device.
const RUN_AHEAD: Duration = Duration::from_millis(100);

/// How long the feeder sleeps when it is far enough ahead.
const FEEDER_NAP: Duration = Duration::from_millis(5);

/// How long a device that was started may take to ask for its first run of sound.
pub const FIRST_RUN: Duration = Duration::from_millis(750);

/// How long a device whose sound has started to be heard may go without asking for more.
pub const SILENT_RUN: Duration = Duration::from_millis(500);

/// How long after sound was first waiting for it a device may go on asking for sound and let
/// none be heard.
pub const NO_SOUND: Duration = Duration::from_secs(2);

/// How long a feeder that is stopped is waited for.
const FEEDER_END: Duration = Duration::from_secs(1);

/// The run of the outputs of no device, and how much later it is heard.
const FAKE_RUN: Duration = Duration::from_millis(10);
const FAKE_LATENCY: Duration = Duration::from_millis(20);

/// The frames that a capture holds between the device and its file (5.5 s at 48 kHz).
const CAPTURE_FRAMES: usize = 1 << 18;

/// What the sound is played on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The default output device of the system; `mute` plays silence in the place of the sound
    /// (`--mute`), and the device is the clock all the same.
    Device { mute: bool },
    /// No device: the sound is taken in runs timed by the clock of the system (`--sound null`).
    Null,
    /// No device, and one that never asks for sound (`--sound stall`).
    Stall,
    /// As [`Kind::Null`], failing once it has played so long (`--sound fail-after=<ms>`).
    FailAfter(Duration),
    /// As [`Kind::Null`], every frame of sound that it plays (the frames counted as played, in
    /// order) also written into the file, as 32-bit floats, little-endian, two channels at
    /// 48 kHz interleaved (`--sound capture=<path>`).
    Capture(PathBuf),
}

impl Kind {
    /// The kind that `--sound` names: `device`, `null`, `stall`, `fail-after=<ms>` or
    /// `capture=<path>`.
    ///
    /// # Errors
    /// It names none.
    pub fn parse(text: &str) -> Result<Kind, String> {
        let wrong = || {
            format!(
                "--sound takes device, null, stall, fail-after=<ms> or capture=<path>, not {text}"
            )
        };
        match text {
            "device" => Ok(Kind::Device { mute: false }),
            "null" => Ok(Kind::Null),
            "stall" => Ok(Kind::Stall),
            _ => {
                if let Some(path) = text.strip_prefix("capture=") {
                    return if path.is_empty() {
                        Err(wrong())
                    } else {
                        Ok(Kind::Capture(PathBuf::from(path)))
                    };
                }
                text.strip_prefix("fail-after=")
                    .and_then(|millis| millis.parse().ok())
                    .map(|millis| Kind::FailAfter(Duration::from_millis(millis)))
                    .ok_or_else(wrong)
            }
        }
    }
}

/// An output of sound, opened, and what plays on it.
pub struct Output {
    backend: Backend,
    shared: Arc<Shared>,
    /// The side that tells the device's thread which session to play, and reads what it gave.
    control: device::Control,
    format: AudioFormat,
    /// The name of the device, for the log.
    name: String,
    /// The id of the device, to tell whether the default device of the system is another now.
    id: Option<String>,
    /// The instant that the times the device's thread tells are counted from.
    epoch: Instant,
    /// The session started last (from 1).
    generation: u64,
    /// Whether a session plays, so that the device is told to stop it once.
    session: bool,
    /// The runs short of sound before the session started last.
    short_before: u64,
    feeder: Option<Feeder>,
    /// When it was last started, while it plays.
    started: Option<Instant>,
    /// The most played that was told, so that what is told never goes back.
    told: Mutex<i64>,
    capture: Option<Capture>,
}

/// What plays the sound.
enum Backend {
    Device(cpal::Stream),
    Fake(Fake),
}

/// The thread of an output of no device.
struct Fake {
    playing: Arc<AtomicBool>,
    quit: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

/// The thread that writes what a capture plays into its file.
struct Capture {
    quit: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

/// Why the device failed, as its thread or the device says, for the transport.
struct Shared {
    failed: Mutex<Option<String>>,
    /// Whether it failed, read by the thread of an output of no device without a lock.
    failing: AtomicBool,
}

impl Shared {
    fn fail(&self, message: String) {
        *lock(&self.failed) = Some(message);
        self.failing.store(true, Ordering::Release);
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The nanoseconds from `epoch` to `at` (fewer than none before it).
fn nanos(epoch: Instant, at: Instant) -> i64 {
    match at.checked_duration_since(epoch) {
        Some(after) => i64::try_from(after.as_nanos()).unwrap_or(i64::MAX),
        None => -i64::try_from(epoch.duration_since(at).as_nanos()).unwrap_or(i64::MAX),
    }
}

/// How many frames of sound have been heard by `now` (nanoseconds from the output's epoch), at
/// `rate` frames a second, by the run of `mark`: those before it, plus (or less) the time from
/// when its first is heard to `now`, up to its sound and, where it was sound and nothing but,
/// [`RUN_AHEAD`] more.
fn played_by(mark: &Mark, rate: u32, now: i64) -> i128 {
    let rate = i128::from(rate);
    let since = i128::from(now) - i128::from(mark.heard);
    let into = since.saturating_mul(rate) / 1_000_000_000;
    let most = if mark.full {
        i128::from(mark.sound)
            + i128::try_from(RUN_AHEAD.as_nanos()).unwrap_or(0) * rate / 1_000_000_000
    } else {
        i128::from(mark.sound)
    };
    i128::from(mark.before) + into.min(most)
}

/// Why a device that was started at `started` and gave `mark` (of the session that plays, if it
/// gave any yet) has failed by `now`, if it has: see [`Output::failed`]. Times are nanoseconds
/// from the output's epoch.
fn failure(mark: Option<&Mark>, started: i64, now: i64, rate: u32, name: &str) -> Option<String> {
    let since = |at: i64| Duration::from_nanos(u64::try_from(now - at).unwrap_or(0));
    let moving = mark.is_some_and(|mark| mark.before > 0 || mark.sound > 0);
    let heard = mark.is_some_and(|mark| played_by(mark, rate, now) > 0);
    match mark {
        None if since(started) > FIRST_RUN => Some(format!(
            "the output device {name} asked for no sound within {} ms of being started",
            FIRST_RUN.as_millis()
        )),
        Some(mark) if moving && since(mark.asked) > SILENT_RUN => Some(format!(
            "the output device {name} asked for no sound for {} ms",
            SILENT_RUN.as_millis()
        )),
        Some(mark) if !heard && mark.fed.is_some_and(|fed| since(fed) > NO_SOUND) => Some(format!(
            "the output device {name} let no sound be heard within {} ms of being given it",
            NO_SOUND.as_millis()
        )),
        _ => None,
    }
}

/// The thread that decodes ahead of the device.
struct Feeder {
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

impl Output {
    /// Open an output of `kind`, in its own format, not playing yet.
    ///
    /// # Errors
    /// There is no output device, or it cannot be opened in a format of samples that this
    /// takes (floats, or integers of 8 to 32 bits); or the file of a capture cannot be made.
    pub fn open(kind: &Kind) -> Result<Output, String> {
        match kind {
            Kind::Device { mute } => Self::open_device(*mute),
            Kind::Null | Kind::Stall | Kind::FailAfter(_) | Kind::Capture(_) => {
                Self::open_fake(kind)
            }
        }
    }

    fn shared() -> Arc<Shared> {
        Arc::new(Shared {
            failed: Mutex::new(None),
            failing: AtomicBool::new(false),
        })
    }

    fn open_device(mute: bool) -> Result<Output, String> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or("there is no output device of sound")?;
        let name = device.to_string();
        let id = device.id().ok().map(|id| id.to_string());
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
        let shared = Self::shared();
        let epoch = Instant::now();
        let (player, control) = device::player(
            (usize::from(format.channels), format.rate),
            true,
            mute,
            None,
        );
        let stream = match supported.sample_format() {
            SampleFormat::F32 => build::<f32>(&device, config, player, &shared, epoch),
            SampleFormat::F64 => build::<f64>(&device, config, player, &shared, epoch),
            SampleFormat::I16 => build::<i16>(&device, config, player, &shared, epoch),
            SampleFormat::I32 => build::<i32>(&device, config, player, &shared, epoch),
            SampleFormat::U16 => build::<u16>(&device, config, player, &shared, epoch),
            SampleFormat::I8 => build::<i8>(&device, config, player, &shared, epoch),
            SampleFormat::U8 => build::<u8>(&device, config, player, &shared, epoch),
            other => {
                return Err(format!(
                    "the output device {name} takes samples of {other}, which are not played"
                ))
            }
        }
        .map_err(|error| format!("the output device {name} cannot be opened: {error}"))?;
        Ok(Output {
            backend: Backend::Device(stream),
            shared,
            control,
            format,
            name,
            id,
            epoch,
            generation: 0,
            session: false,
            short_before: 0,
            feeder: None,
            started: None,
            told: Mutex::new(i64::MIN),
            capture: None,
        })
    }

    fn open_fake(kind: &Kind) -> Result<Output, String> {
        let format = AudioFormat {
            rate: 48000,
            channels: 2,
        };
        let shared = Self::shared();
        let epoch = Instant::now();
        let (capture, producer) = match kind {
            Kind::Capture(path) => {
                let (producer, consumer) = ring(CAPTURE_FRAMES, usize::from(format.channels));
                (Some(Capture::start(path, consumer)?), Some(producer))
            }
            _ => (None, None),
        };
        let (player, control) = device::player(
            (usize::from(format.channels), format.rate),
            false,
            false,
            producer,
        );
        let playing = Arc::new(AtomicBool::new(false));
        let quit = Arc::new(AtomicBool::new(false));
        let thread = if *kind == Kind::Stall {
            None
        } else {
            let fail_after = match kind {
                Kind::FailAfter(after) => Some(*after),
                _ => None,
            };
            let shared = Arc::clone(&shared);
            let playing = Arc::clone(&playing);
            let quit = Arc::clone(&quit);
            std::thread::Builder::new()
                .name("columbina-sound-null".into())
                .spawn(move || fake(player, &shared, (&playing, &quit), fail_after, epoch))
                .ok()
        };
        let name = match kind {
            Kind::Stall => "stall".to_owned(),
            Kind::FailAfter(after) => format!("fail-after={}", after.as_millis()),
            Kind::Capture(path) => format!("capture={}", path.display()),
            _ => "null".to_owned(),
        };
        Ok(Output {
            backend: Backend::Fake(Fake {
                playing,
                quit,
                thread,
            }),
            shared,
            control,
            format,
            name,
            id: None,
            epoch,
            generation: 0,
            session: false,
            short_before: 0,
            feeder: None,
            started: None,
            told: Mutex::new(i64::MIN),
            capture,
        })
    }

    /// The session started last: a new one each time the sound starts (again).
    #[cfg(test)]
    pub fn session(&self) -> u64 {
        self.generation
    }

    /// The rate and channels that the device plays.
    pub fn format(&self) -> AudioFormat {
        self.format
    }

    /// The name of the device.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// What the device's thread counts: the frames of its last run (the period that the thread
    /// of the mix keeps two of ahead).
    pub fn device(&self) -> Arc<device::Shared> {
        Arc::clone(&self.control.shared)
    }

    /// Whether the device is the default output device of the system still. An output of no
    /// device always is.
    pub fn current(&self) -> bool {
        match &self.backend {
            Backend::Fake(_) => true,
            Backend::Device(_) => {
                let now = cpal::default_host()
                    .default_output_device()
                    .and_then(|device| device.id().ok())
                    .map(|id| id.to_string());
                now.is_some() && now == self.id
            }
        }
    }

    /// Play a source in this device's format in place of what played before.
    /// It returns at once: the feeder opens the source, and the device plays silence, which is
    /// not counted as played, until the first samples come.
    ///
    /// # Errors
    /// The device cannot be started, or a thread cannot be started; then nothing plays.
    pub fn start(&mut self, source: Box<dyn Source>) -> Result<(), String> {
        let producer = self.begin()?;
        let stop = Arc::new(AtomicBool::new(false));
        let thread = std::thread::Builder::new()
            .name("columbina-sound".into())
            .spawn({
                let stop = Arc::clone(&stop);
                let format = self.format;
                move || feed(producer, &stop, source, format)
            })
            .map_err(|error| format!("the feeder of the sound cannot be started: {error}"));
        match thread {
            Ok(thread) => {
                self.feeder = Some(Feeder { stop, thread });
                Ok(())
            }
            Err(error) => {
                self.stop();
                Err(error)
            }
        }
    }

    /// Start a session in place of what played before, and give the end of its ring that writes:
    /// the device plays what is written into it, silence (not counted) until the first frames
    /// come. It returns at once.
    ///
    /// # Errors
    /// The device cannot be started, or does not take a new session; then nothing plays.
    pub fn begin(&mut self) -> Result<SampleProducer, String> {
        self.stop();
        while self.control.back.pop().is_some() {}
        self.generation += 1;
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "half a second at a rate of samples"
        )]
        let frames = (f64::from(self.format.rate) * RING_SECONDS) as usize;
        let (producer, consumer) = ring(frames, usize::from(self.format.channels));
        self.control
            .orders
            .push(Order::Play(Feeding {
                generation: self.generation,
                ring: consumer,
            }))
            .map_err(|_| format!("the output device {} takes no new sound", self.name))?;
        self.session = true;
        self.short_before = self.control.shared.short();
        *lock(&self.told) = i64::MIN;
        self.started = Some(Instant::now());
        let played = match &self.backend {
            Backend::Device(stream) => stream
                .play()
                .map_err(|error| format!("the output device {} cannot play: {error}", self.name)),
            Backend::Fake(fake) => {
                fake.playing.store(true, Ordering::Release);
                Ok(())
            }
        };
        match played {
            Ok(()) => Ok(producer),
            Err(error) => {
                self.stop();
                Err(error)
            }
        }
    }

    /// Stop playing at once, let what was waiting to be played go, and end the feeder.
    pub fn stop(&mut self) {
        match &self.backend {
            Backend::Device(stream) => {
                let _ = stream.pause();
            }
            Backend::Fake(fake) => fake.playing.store(false, Ordering::Release),
        }
        self.started = None;
        if let Some(feeder) = self.feeder.take() {
            feeder.stop.store(true, Ordering::Release);
            // It ends within a decode of a run of sound; one that hangs in libav (a file on a
            // drive that does not answer) is left after a second.
            let deadline = Instant::now() + FEEDER_END;
            while !feeder.thread.is_finished() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(1));
            }
            if feeder.thread.is_finished() {
                let _ = feeder.thread.join();
            } else {
                log(
                    "sound_failed",
                    &json!({ "message": "the feeder of the sound did not end; it is left" }),
                );
            }
        }
        if std::mem::take(&mut self.session) {
            // The device lets the session go when it next runs; were the ring of orders full,
            // the next session takes its place all the same.
            let _ = self.control.orders.push(Order::Stop);
        }
        while self.control.back.pop().is_some() {}
    }

    /// What the device gave in its last run of the session started last, if it gave any.
    fn mark(&self) -> Option<Mark> {
        let mark = self.control.mark.read();
        (mark.generation == self.generation && self.generation > 0).then_some(mark)
    }

    /// How many instants of the sound started last have been heard by `now`, at the device's
    /// rate: fewer than none before the first is heard; never fewer than told before. None until
    /// the first run of sound is handed to the device.
    ///
    /// It is reckoned from the last run that the device was given: the instants before it, plus
    /// (or less) the time from when its first instant is heard to `now`. While the device is
    /// given sound and nothing but, the time goes on past that run as it plays on, up to
    /// [`RUN_AHEAD`] (its next run is asked for only once it needs it); where the run ended in
    /// silence, the ring having run dry, the time stops at its end until more sound comes.
    pub fn played(&self, now: Instant) -> Option<i64> {
        let mark = self.mark()?;
        if mark.before == 0 && mark.sound == 0 {
            return None;
        }
        let played = i64::try_from(played_by(&mark, self.format.rate, nanos(self.epoch, now)))
            .unwrap_or(i64::MAX);
        let mut told = lock(&self.told);
        *told = (*told).max(played);
        Some(*told)
    }

    /// The latency of the device (from when it asks for a run to when the run is heard) and
    /// how long its runs last, as its last run had them.
    pub fn latency(&self) -> Option<(Duration, Duration)> {
        self.mark().map(|mark| {
            (
                Duration::from_nanos(u64::try_from(mark.heard - mark.asked).unwrap_or(0)),
                Duration::from_nanos(mark.run),
            )
        })
    }

    /// How many runs of the sound started last had less sound than the device asked for: the
    /// feeder or the mix fell behind, and the device played silence.
    pub fn short_runs(&self) -> u64 {
        self.control.shared.short() - self.short_before
    }

    /// Why the device failed, if it did: it said so, or, while it plays, it asked for no sound
    /// within [`FIRST_RUN`] of being started, asked for none for [`SILENT_RUN`] once its sound
    /// had started to be heard, or let none be heard within [`NO_SOUND`] of being given its first
    /// sound. Then it plays no more, and is to be opened anew.
    pub fn failed(&self) -> Option<String> {
        let mut failed = lock(&self.shared.failed);
        if failed.is_some() {
            return failed.clone();
        }
        let started = self.started?;
        let why = failure(
            self.mark().as_ref(),
            nanos(self.epoch, started),
            nanos(self.epoch, Instant::now()),
            self.format.rate,
            &self.name,
        )?;
        log("sound_failed", &json!({ "message": why }));
        *failed = Some(why);
        self.shared.failing.store(true, Ordering::Release);
        failed.clone()
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        self.stop();
        if let Backend::Fake(fake) = &mut self.backend {
            fake.quit.store(true, Ordering::Release);
            if let Some(thread) = fake.thread.take() {
                let _ = thread.join();
            }
        }
        if let Some(capture) = &mut self.capture {
            capture.quit.store(true, Ordering::Release);
            if let Some(thread) = capture.thread.take() {
                let _ = thread.join();
            }
        }
    }
}

/// Make a stream of the device that plays samples of `T`, given by `player`.
fn build<T>(
    device: &cpal::Device,
    config: StreamConfig,
    mut player: Player,
    shared: &Arc<Shared>,
    epoch: Instant,
) -> Result<cpal::Stream, cpal::Error>
where
    T: SizedSample + FromSample<f32> + Send + 'static,
{
    let failing = Arc::clone(shared);
    device.build_output_stream::<T, _, _>(
        config,
        move |data: &mut [T], info: &OutputCallbackInfo| {
            let stamp = info.timestamp();
            let latency = stamp
                .playback
                .checked_duration_since(stamp.callback)
                .unwrap_or_default();
            let asked = nanos(epoch, Instant::now());
            let heard = asked.saturating_add(i64::try_from(latency.as_nanos()).unwrap_or(0));
            player.fill(data, (asked, heard), T::EQUILIBRIUM, |sample| {
                T::from_sample(sample)
            });
        },
        move |error| {
            let message = error.to_string();
            log("sound_failed", &json!({ "message": message }));
            failing.fail(message);
        },
        None,
    )
}

/// The thread of an output of no device: while it plays, it gives `player` a run of
/// [`FAKE_RUN`] at each tick of it, as a device does, heard [`FAKE_LATENCY`] later; one that is
/// to fail stops taking sound and says it failed once it has played `fail_after`.
fn fake(
    mut player: Player,
    shared: &Shared,
    (playing, quit): (&AtomicBool, &AtomicBool),
    fail_after: Option<Duration>,
    epoch: Instant,
) {
    let instants = usize::try_from(48_000 * FAKE_RUN.as_nanos() / 1_000_000_000)
        .unwrap_or(1)
        .max(1);
    let mut run = vec![0.0_f32; instants * 2];
    let mut next = Instant::now();
    let mut given = Duration::ZERO;
    let latency = i64::try_from(FAKE_LATENCY.as_nanos()).unwrap_or(0);
    while !quit.load(Ordering::Acquire) {
        if !playing.load(Ordering::Acquire) || shared.failing.load(Ordering::Acquire) {
            std::thread::sleep(Duration::from_millis(2));
            next = Instant::now();
            continue;
        }
        if fail_after.is_some_and(|after| given >= after) {
            let message = "the fake output was made to fail".to_owned();
            log("sound_failed", &json!({ "message": message }));
            shared.fail(message);
            continue;
        }
        let now = Instant::now();
        if now < next {
            std::thread::sleep(next - now);
            continue;
        }
        let asked = nanos(epoch, now);
        player.fill(&mut run, (asked, asked + latency), 0.0, |sample| sample);
        given += FAKE_RUN;
        next += FAKE_RUN;
    }
}

impl Capture {
    /// Writes what `ring` gets into a new file at `path` on a thread of its own, until it is
    /// told to quit and the ring is empty.
    fn start(path: &Path, mut ring: SampleConsumer) -> Result<Capture, String> {
        let file = std::fs::File::create(path)
            .map_err(|error| format!("the capture {} cannot be made: {error}", path.display()))?;
        let quit = Arc::new(AtomicBool::new(false));
        let thread = std::thread::Builder::new()
            .name("columbina-sound-capture".into())
            .spawn({
                let quit = Arc::clone(&quit);
                move || {
                    let mut file = std::io::BufWriter::new(file);
                    let mut samples = vec![0.0_f32; 4096 * ring.channels()];
                    let mut bytes = Vec::with_capacity(samples.len() * 4);
                    loop {
                        let frames = ring.pop(&mut samples);
                        if frames == 0 {
                            let _ = file.flush();
                            if quit.load(Ordering::Acquire) {
                                return;
                            }
                            std::thread::sleep(Duration::from_millis(5));
                            continue;
                        }
                        bytes.clear();
                        for sample in &samples[..frames * ring.channels()] {
                            bytes.extend_from_slice(&sample.to_le_bytes());
                        }
                        if file.write_all(&bytes).is_err() {
                            return;
                        }
                    }
                }
            })
            .map_err(|error| format!("the capture cannot be started: {error}"))?;
        Ok(Capture {
            quit,
            thread: Some(thread),
        })
    }
}

/// Read a source into the ring of its session, ahead of the device, until it is stopped or the
/// device lets the session go; after its end, or if it cannot be decoded, silence.
fn feed(mut ring: SampleProducer, stop: &AtomicBool, source: Box<dyn Source>, format: AudioFormat) {
    let mut source = Some(source);
    let channels = usize::from(format.channels);
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a quarter of a second at a rate of samples"
    )]
    let ahead = (f64::from(format.rate) * AHEAD_SECONDS) as usize;
    // 20 ms of silence at a time, after the end.
    let silence = vec![0.0; (format.rate as usize / 50).max(1) * channels];
    let mut pending: Vec<f32> = Vec::new();
    let mut at = 0;
    while !stop.load(Ordering::Acquire) && !ring.is_abandoned() {
        if ring.capacity() - ring.free() >= ahead {
            std::thread::sleep(FEEDER_NAP);
            continue;
        }
        if at >= pending.len() {
            pending = match source.as_mut().map(|source| source.next()) {
                Some(Ok(Some(samples))) => samples,
                Some(Ok(None)) | None => silence.clone(),
                Some(Err(error)) => {
                    log(
                        "sound_failed",
                        &json!({ "message": format!("the sound cannot be decoded on: {error}") }),
                    );
                    source = None;
                    silence.clone()
                }
            };
            at = 0;
        }
        let pushed = ring.push(&pending[at..]);
        at += pushed * channels;
        if pushed == 0 {
            std::thread::sleep(FEEDER_NAP);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_kinds_of_output_are_read_as_sound_names_them() {
        assert_eq!(Kind::parse("device"), Ok(Kind::Device { mute: false }));
        assert_eq!(Kind::parse("null"), Ok(Kind::Null));
        assert_eq!(Kind::parse("stall"), Ok(Kind::Stall));
        assert_eq!(
            Kind::parse("fail-after=300"),
            Ok(Kind::FailAfter(Duration::from_millis(300)))
        );
        assert_eq!(
            Kind::parse("capture=C:/a b/heard.f32"),
            Ok(Kind::Capture(PathBuf::from("C:/a b/heard.f32")))
        );
        for wrong in [
            "",
            "nul",
            "fail-after=",
            "fail-after=x",
            "fail-after=-1",
            "capture=",
        ] {
            assert!(Kind::parse(wrong).is_err(), "{wrong}");
        }
    }

    #[test]
    fn the_null_output_plays_as_fast_as_the_clock_and_is_heard_after_its_latency() {
        let mut output = Output::open(&Kind::Null).unwrap();
        assert!(output.current());
        // A file that is not there plays silence, as a sound that cannot be decoded does.
        output
            .start(file(Path::new("not there.wav"), output.format(), 0))
            .unwrap();
        let started = Instant::now();
        std::thread::sleep(Duration::from_millis(300));
        let played = output.played(Instant::now()).unwrap();
        let elapsed = started.elapsed();
        // 48 kHz, 20 ms late, within a run of 10 ms and the scheduling of the test.
        #[allow(clippy::cast_possible_truncation)]
        let wanted = (elapsed.as_secs_f64() * 48000.0) as i64 - 960;
        assert!(
            (played - wanted).abs() < 1500,
            "{played} played, {wanted} wanted"
        );
        assert!(output.failed().is_none());
        let (latency, run) = output.latency().unwrap();
        assert_eq!((latency, run), (FAKE_LATENCY, FAKE_RUN));
        assert_eq!(output.device().period(), 480);
        output.stop();
        assert!(output.failed().is_none());
    }

    #[test]
    fn an_output_that_stalls_or_fails_has_failed() {
        let mut stalled = Output::open(&Kind::Stall).unwrap();
        stalled
            .start(file(Path::new("not there.wav"), stalled.format(), 0))
            .unwrap();
        assert!(stalled.failed().is_none());
        std::thread::sleep(FIRST_RUN + Duration::from_millis(50));
        assert!(stalled.failed().unwrap().contains("asked for no sound"));
        assert!(stalled.played(Instant::now()).is_none());

        let mut failing = Output::open(&Kind::FailAfter(Duration::from_millis(100))).unwrap();
        failing
            .start(file(Path::new("not there.wav"), failing.format(), 0))
            .unwrap();
        std::thread::sleep(Duration::from_millis(250));
        assert_eq!(
            failing.failed().as_deref(),
            Some("the fake output was made to fail")
        );
        // Stopped, it stays failed.
        failing.stop();
        assert!(failing.failed().is_some());
    }

    #[test]
    fn the_first_run_of_a_device_is_silence_that_is_not_counted() {
        let (mut player, mut control) = device::player((1, 1000), true, false, None);
        let (mut producer, consumer) = ring(64, 1);
        producer.push(&[0.5_f32; 30]);
        control
            .orders
            .push(Order::Play(Feeding {
                generation: 1,
                ring: consumer,
            }))
            .unwrap();
        let mut run = [1.0_f32; 10];
        player.fill(&mut run, (0, 0), 0.0, |x| x);
        assert_eq!(run, [0.0; 10]);
        let mark = control.mark.read();
        assert_eq!((mark.generation, mark.before, mark.sound), (1, 0, 0));
        player.fill(&mut run, (0, 0), 0.0, |x| x);
        assert_eq!(run, [0.5; 10]);
        let mark = control.mark.read();
        assert_eq!((mark.before, mark.sound, mark.full), (0, 10, true));
        // An output of no device has no such first run.
        let (mut fake, mut told) = device::player((1, 1000), false, false, None);
        let (mut producer, consumer) = ring(64, 1);
        producer.push(&[0.5_f32; 10]);
        told.orders
            .push(Order::Play(Feeding {
                generation: 1,
                ring: consumer,
            }))
            .unwrap();
        fake.fill(&mut run, (0, 0), 0.0, |x| x);
        assert_eq!(told.mark.read().sound, 10);
    }

    #[test]
    fn a_device_that_stops_once_heard_or_lets_nothing_be_heard_has_failed() {
        const MS: i64 = 1_000_000;
        let now = 10_000 * MS;
        let mark = |sound: u64, asked: i64, fed: Option<i64>| Mark {
            generation: 1,
            before: 0,
            sound,
            full: sound > 0,
            heard: now - asked * MS,
            asked: now - asked * MS,
            run: 10 * MS as u64,
            fed: fed.map(|ago| now - ago * MS),
        };
        let failed = |mark: Option<Mark>, started: i64| {
            failure(mark.as_ref(), now - started * MS, now, 48000, "test")
        };
        // Its sound heard, then no run asked for 600 ms.
        let why = failed(Some(mark(10, 600, Some(1000))), 1000);
        assert!(why.unwrap().contains("asked for no sound for 500 ms"));
        // 300 ms is not long enough.
        assert!(failed(Some(mark(10, 300, Some(1000))), 1000).is_none());
        // Before any sound was heard, a pause in its runs is not counted...
        assert!(failed(Some(mark(0, 600, None)), 1000).is_none());
        // ...but nothing heard 2 s after its sound first waited for it is.
        let why = failed(Some(mark(0, 5, Some(2100))), 2100);
        assert!(why
            .unwrap()
            .contains("let no sound be heard within 2000 ms"));
        // A device given no sound yet (a graph still being built) has not failed for that.
        assert!(failed(Some(mark(0, 5, None)), 5000).is_none());
        assert!(failed(Some(mark(0, 5, Some(1000))), 5000).is_none());
        // Nor has one asked for nothing within 750 ms of being started; one that has, has.
        assert!(failed(None, 700).is_none());
        assert!(failed(None, 800).unwrap().contains("within 750 ms"));
    }

    #[test]
    fn a_capture_writes_every_frame_of_sound_it_plays_into_its_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("heard.f32");
        let mut output = Output::open(&Kind::Capture(path.clone())).unwrap();
        let mut ring = output.begin().unwrap();
        let frames: Vec<f32> = (0..9600)
            .flat_map(|n| {
                #[allow(clippy::cast_precision_loss, reason = "a test signal")]
                let value = n as f32 / 9600.0;
                [value, -value]
            })
            .collect();
        assert_eq!(ring.push(&frames), 9600);
        let deadline = Instant::now() + Duration::from_secs(5);
        while output.played(Instant::now()).unwrap_or(0) < 9600 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        drop(output);
        let bytes = std::fs::read(&path).unwrap();
        let written: Vec<f32> = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|word| f32::from_le_bytes(*word))
            .collect();
        // Only what was counted as played, in order: the silence after the end of the ring is
        // not sound.
        assert_eq!(written, frames);
    }
}
