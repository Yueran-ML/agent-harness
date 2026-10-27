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
//!
//! The first run that a device asks for once it is started is filled with silence, which is not
//! counted: WASAPI says that it is heard at once (a latency of 0) while it is in fact laid
//! ahead of what plays, some 20 ms, so that sound in it would be counted as heard that much too
//! early, and the clock and `sound_minus_system_ms` would be off by as much.
//!
//! A device that fails is never used again: [`Output::failed`] says so, and the transport opens
//! the default device anew the next time it plays, as it does when the default device of the
//! system changed ([`Output::current`]). A device that asks for no sound within [`FIRST_RUN`]
//! of being started (a Bluetooth headset waking takes some 300 ms), that stops asking for
//! [`SILENT_RUN`] once its sound has started to be heard, or that has let none of it be heard
//! [`NO_SOUND`] after it was started though it goes on asking, has failed too
//! ([`Output::failed`]): the picture then plays by the clock of the system.
//!
//! Besides the device, [`Kind`] has outputs of no device, for the tests and for machines with
//! no sound (`--sound null|stall|fail-after=<ms>`): one that takes the sound in runs of 10 ms
//! timed by the clock of the system, as a device would, heard 20 ms later; one that never asks
//! for sound; and one like the first that fails once it has played so long.

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

/// How long a device that was started may take to ask for its first run of sound.
pub const FIRST_RUN: Duration = Duration::from_millis(750);

/// How long a device whose sound has started to be heard may go without asking for more.
pub const SILENT_RUN: Duration = Duration::from_millis(500);

/// How long after it was started a device may go on asking for sound and let none be heard.
pub const NO_SOUND: Duration = Duration::from_secs(2);

/// How long a feeder that is stopped is waited for.
const FEEDER_END: Duration = Duration::from_secs(1);

/// The run of the outputs of no device, and how much later it is heard.
const FAKE_RUN: Duration = Duration::from_millis(10);
const FAKE_LATENCY: Duration = Duration::from_millis(20);

/// What the sound is played on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
}

impl Kind {
    /// The kind that `--sound` names: `device`, `null`, `stall` or `fail-after=<ms>`.
    ///
    /// # Errors
    /// It names none.
    pub fn parse(text: &str) -> Result<Kind, String> {
        match text {
            "device" => Ok(Kind::Device { mute: false }),
            "null" => Ok(Kind::Null),
            "stall" => Ok(Kind::Stall),
            _ => text
                .strip_prefix("fail-after=")
                .and_then(|millis| millis.parse().ok())
                .map(|millis| Kind::FailAfter(Duration::from_millis(millis)))
                .ok_or_else(|| {
                    format!("--sound takes device, null, stall or fail-after=<ms>, not {text}")
                }),
        }
    }
}

/// An output of sound, opened, and what plays on it.
pub struct Output {
    backend: Backend,
    shared: Arc<Shared>,
    format: AudioFormat,
    /// The name of the device, for the log.
    name: String,
    /// The id of the device, to tell whether the default device of the system is another now.
    id: Option<String>,
    feeder: Option<Feeder>,
    /// When it was last started, while it plays.
    started: Option<Instant>,
    /// The most played that was told, so that what is told never goes back.
    told: Mutex<i64>,
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

/// What the device's thread, the feeder and the transport share.
struct Shared {
    queue: Mutex<Queue>,
    /// What the device was last given.
    mark: Mutex<Option<Mark>>,
    /// Why the device failed, once it did.
    failed: Mutex<Option<String>>,
    channels: usize,
    rate: u32,
    /// Whether silence is played in the place of the sound.
    mute: bool,
    /// Whether the first run after each start is silence, not counted (a device of WASAPI).
    prime: bool,
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
    /// How many runs since the start had less sound than they asked for, after the first
    /// sound: the feeder fell behind.
    short: u64,
    /// Whether the first run since the start was asked for.
    primed: bool,
}

/// What the device was given in its last run: the instants of sound before it, how many of its
/// first instants were sound (the rest was silence), whether all of them were, when the first
/// is heard, and when it was asked for.
#[derive(Clone, Copy)]
struct Mark {
    before: u64,
    sound: u64,
    full: bool,
    heard: Instant,
    asked: Instant,
    /// How long the run lasts.
    run: Duration,
}

impl Mark {
    /// How many instants of sound have been heard by `now`, at `rate` instants a second, by
    /// this run: those before it, plus (or less) the time from when its first is heard to
    /// `now`, up to its sound and, where it was sound and nothing but, [`RUN_AHEAD`] more.
    fn played(&self, rate: u32, now: Instant) -> i128 {
        let rate = i128::from(rate);
        let since = match now.checked_duration_since(self.heard) {
            Some(after) => i128::try_from(after.as_nanos()).unwrap_or(i128::MAX),
            None => -i128::try_from(self.heard.duration_since(now).as_nanos()).unwrap_or(0),
        };
        let into = since.saturating_mul(rate) / 1_000_000_000;
        let most = if self.full {
            i128::from(self.sound)
                + i128::try_from(RUN_AHEAD.as_nanos()).unwrap_or(0) * rate / 1_000_000_000
        } else {
            i128::from(self.sound)
        };
        i128::from(self.before) + into.min(most)
    }
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
    /// Open an output of `kind`, in its own format, not playing yet.
    ///
    /// # Errors
    /// There is no output device, or it cannot be opened in a format of samples that this
    /// takes (floats, or integers of 8 to 32 bits).
    pub fn open(kind: Kind) -> Result<Output, String> {
        match kind {
            Kind::Device { mute } => Self::open_device(mute),
            Kind::Null | Kind::Stall | Kind::FailAfter(_) => Ok(Self::open_fake(kind)),
        }
    }

    fn shared(format: AudioFormat, mute: bool, prime: bool) -> Arc<Shared> {
        Arc::new(Shared {
            queue: Mutex::new(Queue::default()),
            mark: Mutex::new(None),
            failed: Mutex::new(None),
            channels: usize::from(format.channels),
            rate: format.rate,
            mute,
            prime,
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
        let shared = Self::shared(format, mute, true);
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
            backend: Backend::Device(stream),
            shared,
            format,
            name,
            id,
            feeder: None,
            started: None,
            told: Mutex::new(i64::MIN),
        })
    }

    fn open_fake(kind: Kind) -> Output {
        let format = AudioFormat {
            rate: 48000,
            channels: 2,
        };
        let shared = Self::shared(format, false, false);
        let playing = Arc::new(AtomicBool::new(false));
        let quit = Arc::new(AtomicBool::new(false));
        let thread = if kind == Kind::Stall {
            None
        } else {
            let fail_after = match kind {
                Kind::FailAfter(after) => Some(after),
                _ => None,
            };
            let shared = Arc::clone(&shared);
            let playing = Arc::clone(&playing);
            let quit = Arc::clone(&quit);
            std::thread::Builder::new()
                .name("columbina-sound-null".into())
                .spawn(move || fake(&shared, &playing, &quit, fail_after))
                .ok()
        };
        let name = match kind {
            Kind::Stall => "stall".to_owned(),
            Kind::FailAfter(after) => format!("fail-after={}", after.as_millis()),
            _ => "null".to_owned(),
        };
        Output {
            backend: Backend::Fake(Fake {
                playing,
                quit,
                thread,
            }),
            shared,
            format,
            name,
            id: None,
            feeder: None,
            started: None,
            told: Mutex::new(i64::MIN),
        }
    }

    /// The rate and channels that the device plays.
    pub fn format(&self) -> AudioFormat {
        self.format
    }

    /// The name of the device.
    pub fn name(&self) -> &str {
        &self.name
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

    /// Play the sound of the file at `path` from sample `from` (at the device's rate, from time
    /// 0 of the file, as [`AudioDecoder`] numbers them), in the place of what played before.
    /// It returns at once: the feeder opens the file, and the device plays silence, which is
    /// not counted as played, until the first samples come.
    ///
    /// # Errors
    /// The device cannot be started, or a thread cannot be started; then nothing plays.
    pub fn start(&mut self, path: &Path, from: i64) -> Result<(), String> {
        self.stop();
        let generation = {
            let mut queue = lock(&self.shared.queue);
            queue.generation += 1;
            queue.samples.clear();
            queue.played = 0;
            queue.short = 0;
            queue.primed = false;
            queue.generation
        };
        *lock(&self.shared.mark) = None;
        *lock(&self.told) = i64::MIN;
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
        if played.is_err() {
            self.stop();
        }
        played
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
        let mut queue = lock(&self.shared.queue);
        queue.generation += 1;
        queue.samples.clear();
    }

    /// How many instants of the sound started last have been heard by `now`, at the device's
    /// rate: fewer than none before the first is heard; never fewer than told before. None until
    /// the first run of sound is handed to the device.
    ///
    /// It is reckoned from the last run that the device was given: the instants before it, plus
    /// (or less) the time from when its first instant is heard to `now`. While the device is
    /// given sound and nothing but, the time goes on past that run as it plays on, up to
    /// [`RUN_AHEAD`] (its next run is asked for only once it needs it); where the run ended in
    /// silence, the feeder having fallen behind, the time stops at its end until more sound
    /// comes.
    pub fn played(&self, now: Instant) -> Option<i64> {
        let mark = (*lock(&self.shared.mark))?;
        if mark.before == 0 && mark.sound == 0 {
            return None;
        }
        let played = i64::try_from(mark.played(self.format.rate, now)).unwrap_or(i64::MAX);
        let mut told = lock(&self.told);
        *told = (*told).max(played);
        Some(*told)
    }

    /// The latency of the device (from when it asks for a run to when the run is heard) and
    /// how long its runs last, as its last run had them.
    pub fn latency(&self) -> Option<(Duration, Duration)> {
        lock(&self.shared.mark)
            .map(|mark| (mark.heard.saturating_duration_since(mark.asked), mark.run))
    }

    /// How many runs of the sound started last had less sound than the device asked for: the
    /// feeder fell behind, and the device played silence.
    pub fn short_runs(&self) -> u64 {
        lock(&self.shared.queue).short
    }

    /// Why the device failed, if it did: it said so, or, while it plays, it asked for no sound
    /// within [`FIRST_RUN`] of being started, asked for none for [`SILENT_RUN`] once its sound
    /// had started to be heard, or let none be heard within [`NO_SOUND`] of being started. Then
    /// it plays no more, and is to be opened anew.
    pub fn failed(&self) -> Option<String> {
        let mut failed = lock(&self.shared.failed);
        if failed.is_some() {
            return failed.clone();
        }
        let started = self.started?;
        let now = Instant::now();
        let mark = *lock(&self.shared.mark);
        let moving = mark.is_some_and(|mark| mark.before > 0 || mark.sound > 0);
        let heard = mark.is_some_and(|mark| mark.played(self.format.rate, now) > 0);
        let since_start = now.duration_since(started);
        let why = match mark {
            None if since_start > FIRST_RUN => format!(
                "the output device {} asked for no sound within {} ms of being started",
                self.name,
                FIRST_RUN.as_millis()
            ),
            Some(mark) if moving && now.saturating_duration_since(mark.asked) > SILENT_RUN => {
                format!(
                    "the output device {} asked for no sound for {} ms",
                    self.name,
                    SILENT_RUN.as_millis()
                )
            }
            Some(_) if !heard && since_start > NO_SOUND => format!(
                "the output device {} let no sound be heard within {} ms of being started",
                self.name,
                NO_SOUND.as_millis()
            ),
            _ => return None,
        };
        log("sound_failed", &json!({ "message": why }));
        *failed = Some(why);
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
        move |data: &mut [T], info: &OutputCallbackInfo| {
            let stamp = info.timestamp();
            let latency = stamp
                .playback
                .checked_duration_since(stamp.callback)
                .unwrap_or_default();
            let asked = Instant::now();
            playing.fill(data, asked, asked + latency);
        },
        move |error| {
            let message = error.to_string();
            log("sound_failed", &json!({ "message": message }));
            *lock(&failing.failed) = Some(message);
        },
        None,
    )
}

impl Shared {
    /// Fill a run of the device, asked for at `asked` and heard from `heard` on, with what waits
    /// in the queue, and silence after it; mark how much of it is sound, and when it is heard.
    fn fill<T: SizedSample + FromSample<f32>>(
        &self,
        data: &mut [T],
        asked: Instant,
        heard: Instant,
    ) {
        let mut queue = lock(&self.queue);
        let instants = data.len() / self.channels;
        let run = Duration::from_nanos(
            u64::try_from(instants).unwrap_or(0) * 1_000_000_000 / u64::from(self.rate.max(1)),
        );
        if self.prime && !queue.primed {
            // The first run: silence, not counted, since it is told as heard too early.
            queue.primed = true;
            drop(queue);
            for slot in data.iter_mut() {
                *slot = T::EQUILIBRIUM;
            }
            *lock(&self.mark) = Some(Mark {
                before: 0,
                sound: 0,
                full: false,
                heard,
                asked,
                run,
            });
            return;
        }
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
        if before > 0 && sound < u64::try_from(instants).unwrap_or(u64::MAX) {
            queue.short += 1;
        }
        queue.played += sound;
        drop(queue);
        *lock(&self.mark) = Some(Mark {
            before,
            sound,
            full: sound == u64::try_from(instants).unwrap_or(u64::MAX),
            heard,
            asked,
            run,
        });
    }
}

/// The thread of an output of no device: while it plays, it takes a run of [`FAKE_RUN`] at
/// each tick of it, as a device does, heard [`FAKE_LATENCY`] later; one that is to fail stops
/// taking sound and says it failed once it has played `fail_after`.
fn fake(shared: &Shared, playing: &AtomicBool, quit: &AtomicBool, fail_after: Option<Duration>) {
    let instants = usize::try_from(u128::from(shared.rate) * FAKE_RUN.as_nanos() / 1_000_000_000)
        .unwrap_or(1)
        .max(1);
    let mut run = vec![0.0_f32; instants * shared.channels];
    let mut next = Instant::now();
    let mut played = Duration::ZERO;
    while !quit.load(Ordering::Acquire) {
        if !playing.load(Ordering::Acquire) || lock(&shared.failed).is_some() {
            std::thread::sleep(Duration::from_millis(2));
            next = Instant::now();
            continue;
        }
        if fail_after.is_some_and(|after| played >= after) {
            let message = "the fake output was made to fail".to_owned();
            log("sound_failed", &json!({ "message": message }));
            *lock(&shared.failed) = Some(message);
            continue;
        }
        let now = Instant::now();
        if now < next {
            std::thread::sleep(next - now);
            continue;
        }
        shared.fill(&mut run, now, now + FAKE_LATENCY);
        played += FAKE_RUN;
        next += FAKE_RUN;
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
    while !stop.load(Ordering::Acquire) {
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
        for wrong in ["", "nul", "fail-after=", "fail-after=x", "fail-after=-1"] {
            assert!(Kind::parse(wrong).is_err(), "{wrong}");
        }
    }

    #[test]
    fn the_null_output_plays_as_fast_as_the_clock_and_is_heard_after_its_latency() {
        let mut output = Output::open(Kind::Null).unwrap();
        assert!(output.current());
        // A file that is not there plays silence, as a sound that cannot be decoded does.
        output.start(Path::new("not there.wav"), 0).unwrap();
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
        output.stop();
        assert!(output.failed().is_none());
    }

    #[test]
    fn an_output_that_stalls_or_fails_has_failed() {
        let mut stalled = Output::open(Kind::Stall).unwrap();
        stalled.start(Path::new("not there.wav"), 0).unwrap();
        assert!(stalled.failed().is_none());
        std::thread::sleep(FIRST_RUN + Duration::from_millis(50));
        assert!(stalled.failed().unwrap().contains("asked for no sound"));
        assert!(stalled.played(Instant::now()).is_none());

        let mut failing = Output::open(Kind::FailAfter(Duration::from_millis(100))).unwrap();
        failing.start(Path::new("not there.wav"), 0).unwrap();
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
        let format = AudioFormat {
            rate: 1000,
            channels: 1,
        };
        let shared = Output::shared(format, false, true);
        lock(&shared.queue).samples.extend([0.5_f32; 30]);
        let now = Instant::now();
        let mut run = [1.0_f32; 10];
        shared.fill(&mut run, now, now);
        assert_eq!(run, [0.0; 10]);
        let mark = lock(&shared.mark).unwrap();
        assert_eq!((mark.before, mark.sound), (0, 0));
        assert_eq!(lock(&shared.queue).samples.len(), 30);
        shared.fill(&mut run, now, now);
        assert_eq!(run, [0.5; 10]);
        let mark = lock(&shared.mark).unwrap();
        assert_eq!((mark.before, mark.sound, mark.full), (0, 10, true));
        // An output of no device has no such first run.
        let fake = Output::shared(format, false, false);
        lock(&fake.queue).samples.extend([0.5_f32; 10]);
        fake.fill(&mut run, now, now);
        assert_eq!(lock(&fake.mark).unwrap().sound, 10);
    }

    #[test]
    fn a_device_that_stops_once_heard_or_lets_nothing_be_heard_has_failed() {
        let now = Instant::now();
        let mark = |sound: u64, asked: Duration| Mark {
            before: 0,
            sound,
            full: sound > 0,
            heard: now.checked_sub(asked).unwrap(),
            asked: now.checked_sub(asked).unwrap(),
            run: Duration::from_millis(10),
        };
        let failed = |mark: Mark, started: Duration| {
            let mut output = Output::open(Kind::Stall).unwrap();
            output.started = now.checked_sub(started);
            *lock(&output.shared.mark) = Some(mark);
            output.failed()
        };
        // Its sound heard, then no run asked for 600 ms.
        let why = failed(mark(10, Duration::from_millis(600)), Duration::from_secs(1));
        assert!(why.unwrap().contains("asked for no sound for 500 ms"));
        // 300 ms is not long enough.
        assert!(failed(mark(10, Duration::from_millis(300)), Duration::from_secs(1)).is_none());
        // Before any sound was heard, a pause in its runs is not counted...
        assert!(failed(mark(0, Duration::from_millis(600)), Duration::from_secs(1)).is_none());
        // ...but nothing heard 2 s after it was started is.
        let why = failed(
            mark(0, Duration::from_millis(5)),
            Duration::from_millis(2100),
        );
        assert!(why
            .unwrap()
            .contains("let no sound be heard within 2000 ms"));
    }
}
