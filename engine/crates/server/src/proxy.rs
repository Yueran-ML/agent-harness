//! Proxies: for media that is slow to seek in, a copy of its picture whose every frame is a key
//! frame, so that any frame is decoded alone, at once: scrubbing shows the frame itself and not
//! a key frame before it, and playing backward shows every frame.
//!
//! # Which media
//!
//! Media whose picture has long groups of frames (a key frame, then frames that are decoded
//! from it) and any of: more than 1080 rows, more than 8 bits, HEVC or AV1, or B-frames
//! ([`needs_proxy`]). Media whose every frame is a key frame (`ProRes`, `DNxHR`, MJPEG, H.264 or
//! HEVC of intra frames only) seeks quickly already, and so does H.264 of 8 bits up to 1080p
//! without B-frames. The first [`GOP_PACKETS`] packets of the picture are read to tell
//! ([`columbina_media::gop`]).
//!
//! # How
//!
//! A proxy is made by the `ffmpeg` of the LGPL build that comes with the engine ([`find_ffmpeg`];
//! never one found on `PATH`, which may be of the GPL), one at a time, at a low priority of the
//! system, from the media taken in by `media.import` and, when the engine starts, from the
//! media of the project that has none (or one made from another file, or whose file is gone).
//! The picture is made at most 1920 pixels on its long edge, its shape kept, in HEVC of intra
//! frames only by NVENC (decoded and scaled on the card where CUDA can, else on the processor),
//! or in `ProRes` 422 Proxy where NVENC is not there or fails. Its frames are those of the media,
//! with their timestamps (`-fps_mode passthrough`, the time base of the media), so that frame `n`
//! of the proxy is frame `n` of the media, numbered at the media's rate; it has no sound, which is
//! read from the media. A proxy is checked before it is used: its first and last frames are
//! numbered as the media's ([`columbina_media::frame_numbers`]), and its frames are all key
//! frames. It is written to `<project>/cache/proxies/<media>-<12 digits of the digest>.mov`, as a
//! `.part` first, and set on the media by a change of the engine that is no step of the history to
//! undo ([`columbina_model::History::apply_unrecorded`]). Media whose file is that of other media
//! (the same digest and the same facts) is given that media's proxy ([`twin_of`]).
//!
//! What it is at is told to every client in `media.proxy`: `queued`, `running` (with how far it
//! is, at most once a second), `done` or `failed`. A proxy being made of media that is removed
//! is stopped and its `.part` deleted, and so is one being made when the engine ends; so is one
//! whose `ffmpeg` makes no progress for [`WATCHDOG`]. A proxy that cannot be made of a file by any
//! encoder (but for a full disk or a directory that may not be written, which may pass) is
//! remembered as such in `cache/proxies/failed.json`, by the digest of the file, and
//! not tried again until a client asks for it (`media.make_proxy`). As the engine starts, the files
//! of `cache/proxies` that no media of the project names are deleted.

use std::collections::{BTreeMap, VecDeque};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use columbina_media::{Gop, Time, VideoFacts};
use columbina_model::{Media, Project, Proxy};
use serde::Serialize;
use serde_json::json;

use crate::log::log;
use crate::transport::clock;

/// How many packets of the picture are read to tell whether media needs a proxy.
pub const GOP_PACKETS: u64 = 64;

/// The longest edge of a proxy, in pixels.
pub const LONG_EDGE: u32 = 1920;

/// How often how far a proxy is is told, at most.
pub const TOLD_EVERY: Duration = Duration::from_secs(1);

/// How long an `ffmpeg` may go without writing a frame before it is taken for stuck, and killed.
pub const WATCHDOG: Duration = Duration::from_secs(60);

/// How many frames at the start and at the end of a proxy are checked to be numbered as those of
/// its media.
const CHECKED_FRAMES: usize = 8;

/// The file of the proxies that could not be made, by the digest of their media.
const FAILED: &str = "failed.json";

/// How long the engine waits, as it ends, for the proxy being made to stop.
const STOPPING: Duration = Duration::from_secs(3);

/// What the engine was told of proxies on its command line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Setup {
    /// Whether proxies are made: not in a headless engine, nor with `--no-proxies`.
    pub make: bool,
    /// The `ffmpeg` to make them with (`--proxy-ffmpeg`, an absolute path), before
    /// `COLUMBINA_PROXY_FFMPEG`.
    pub ffmpeg: Option<PathBuf>,
    /// For the tests (`--proxy-readrate <R>`): the media is read no faster than R times as it
    /// plays (`-readrate`), so that a proxy takes long to make, or makes no progress.
    pub readrate: Option<String>,
    /// For the tests (`--proxy-encoder nvenc|prores`): only the recipes of this encoder.
    pub encoder: Option<Encoder>,
    /// How long an `ffmpeg` may go without writing a frame (`--proxy-watchdog <S>`, for the
    /// tests; [`WATCHDOG`] unless given).
    pub watchdog: Duration,
}

impl Default for Setup {
    fn default() -> Setup {
        Setup {
            make: true,
            ffmpeg: None,
            readrate: None,
            encoder: None,
            watchdog: WATCHDOG,
        }
    }
}

/// The encoders that a proxy is made by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoder {
    /// HEVC by NVENC.
    Nvenc,
    /// `ProRes` 422 Proxy.
    Prores,
}

impl Encoder {
    /// The encoder that `--proxy-encoder` names: `nvenc` or `prores`.
    ///
    /// # Errors
    /// It names none.
    pub fn parse(text: &str) -> Result<Encoder, String> {
        match text {
            "nvenc" => Ok(Encoder::Nvenc),
            "prores" => Ok(Encoder::Prores),
            _ => Err(format!("--proxy-encoder takes nvenc or prores, not {text}")),
        }
    }
}

/// Whether media needs a proxy: its picture has long groups of frames, and more than 1080 rows,
/// more than 8 bits, a codec slow to decode (HEVC, AV1), or B-frames.
pub fn needs_proxy(video: &VideoFacts, gop: &Gop) -> bool {
    !gop.all_intra()
        && (video.height > 1080
            || video.bits > 8
            || matches!(video.codec.as_str(), "hevc" | "av1")
            || gop.b_frames)
}

/// The size of the proxy of a picture of `width` × `height`: at most [`LONG_EDGE`] on its long
/// edge, its shape kept, in even numbers of pixels (as 4:2:0 needs).
pub fn proxy_size(width: u32, height: u32) -> (u32, u32) {
    let long = width.max(height).max(1);
    let even = |pixels: u64| u32::try_from((pixels / 2).max(1) * 2).unwrap_or(u32::MAX);
    if long <= LONG_EDGE {
        return (even(width.into()), even(height.into()));
    }
    // Rounded to the nearest even number.
    let scaled = |pixels: u32| {
        let over = u64::from(pixels) * u64::from(LONG_EDGE);
        let under = u64::from(long);
        even((over + under) / under)
    };
    (scaled(width), scaled(height))
}

/// The name of the file of the proxy of media: its id, with any character that a file name may
/// not hold put as `_`, and the first 12 digits of the digest of its identity.
pub fn file_name(media: &str, digest: &str) -> String {
    let id: String = media
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let digest: String = digest
        .chars()
        .filter(char::is_ascii_hexdigit)
        .take(12)
        .collect();
    format!("{id}-{digest}.mov")
}

/// The directory of the proxies of the project in `project_dir`.
pub fn proxy_dir(project_dir: &Path) -> PathBuf {
    project_dir.join("cache").join("proxies")
}

/// Whether a file name is that of a proxy: `<id>-<12 hex digits>.mov`, the id of the media
/// that it was made for (of the letters, digits, `-` and `_` that [`file_name`] keeps), which
/// may be other media of the same file.
pub fn proxy_name(name: &str) -> bool {
    let Some(stem) = name.strip_suffix(".mov") else {
        return false;
    };
    let Some((id, digest)) = stem.rsplit_once('-') else {
        return false;
    };
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        && digest.len() == 12
        && digest
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
}

/// Whether `path` is in `dir` itself (`<project>/cache/proxies`), as the system finds them, so
/// that no `..`, link or junction leads out of it: an absolute path without `.` or `..`, whose
/// directory is `dir`, which is `cache/proxies` of the project itself, not a link or a junction
/// to somewhere else.
fn inside(dir: &Path, path: &Path) -> bool {
    use std::path::Component;
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
    {
        return false;
    }
    let canonical = |path: &Path| path.canonicalize().ok();
    let project = dir.parent().and_then(Path::parent).and_then(canonical);
    let (Some(parent), Some(dir), Some(project)) =
        (path.parent().and_then(canonical), canonical(dir), project)
    else {
        return false;
    };
    dir == project.join("cache").join("proxies") && parent == dir
}

/// Whether `path` is one that the engine keeps a proxy at: a file named as a proxy is
/// ([`proxy_name`]) directly in `dir`, the directory of the proxies of the project. A proxy that
/// a client sets, or that a project that is read names, must be so; and nothing else is ever
/// deleted as a proxy.
pub fn owned(dir: &Path, path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(proxy_name)
        && inside(dir, path)
}

/// Delete the file of a proxy, or a `.part` of one, at `path` in `dir`: only a file of the
/// engine's own ([`owned`]), and only a file, not a link. Whether it was deleted.
pub fn delete(dir: &Path, path: &Path) -> bool {
    let named = path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            proxy_name(name)
                || name
                    .strip_suffix(".part")
                    .is_some_and(|name| proxy_name(name) || name == FAILED)
        });
    let file = match std::fs::symlink_metadata(path) {
        Ok(found) => found.is_file(),
        // Nothing to delete.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return false,
        Err(_) => false,
    };
    if !(named && file && inside(dir, path)) {
        log(
            "proxy_not_deleted",
            &json!({
                "path": path.to_string_lossy(),
                "message": "it is not a file of the proxies of the project",
            }),
        );
        return false;
    }
    std::fs::remove_file(path).is_ok()
}

/// A project as it is read, with every proxy that is not where the engine keeps proxies
/// ([`owned`]) taken away, and logged: a project written by hand, or by another program, cannot
/// have the engine read, or delete, a file elsewhere.
pub fn owned_only(mut project: Project, dir: &Path) -> Project {
    for media in &mut project.media {
        if let Some(proxy) = media
            .proxy
            .take_if(|proxy| !owned(dir, Path::new(&proxy.path)))
        {
            log(
                "proxy_dropped",
                &json!({
                    "media": media.id,
                    "path": proxy.path,
                    "message": "the proxy is not in the directory of the proxies of the project",
                }),
            );
        }
    }
    project
}

/// The proxy of media that may be used: one made from the file of its identity. Whether its file
/// is there and can be read is known only once it is opened.
pub fn usable(media: &Media) -> Option<&Proxy> {
    media
        .proxy
        .as_ref()
        .filter(|proxy| proxy.digest_of_source == media.identity.digest)
}

/// The `ffmpeg` that proxies are made with: `given` (`--proxy-ffmpeg`, which is absolute), else
/// `COLUMBINA_PROXY_FFMPEG` if it is an absolute path, else the one beside the engine, else, in a
/// build for development only, the one of the LGPL build of FFmpeg that it was built with
/// (`FFMPEG_DIR` as it was built). Never one found on `PATH`: a name that is not an absolute path
/// would be looked for there, and is not taken.
pub fn find_ffmpeg(given: Option<&Path>) -> Option<PathBuf> {
    let name = if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    if let Some(given) = given {
        return Some(given.to_owned());
    }
    if let Some(named) =
        std::env::var_os("COLUMBINA_PROXY_FFMPEG").filter(|named| !named.is_empty())
    {
        let named = PathBuf::from(named);
        if named.is_absolute() {
            return Some(named);
        }
        log(
            "proxy_ffmpeg",
            &json!({
                "message": format!(
                    "COLUMBINA_PROXY_FFMPEG is not an absolute path, and is not taken: {}",
                    named.display()
                ),
            }),
        );
    }
    let beside = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(name)))
        .filter(|path| path.is_file());
    #[cfg(debug_assertions)]
    let built = option_env!("FFMPEG_DIR")
        .map(|dir| Path::new(dir).join("bin").join(name))
        .filter(|path| path.is_file());
    #[cfg(not(debug_assertions))]
    let built = None;
    beside.or(built)
}

/// What is told of a proxy, in `media.proxy`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Note {
    pub media: String,
    /// `queued`, `running`, `done` or `failed`.
    pub state: &'static str,
    /// How far it is, from 0 to 1.
    pub progress: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl Note {
    fn new(media: &str, state: &'static str, progress: f64) -> Note {
        Note {
            media: media.to_owned(),
            state,
            progress,
            message: None,
        }
    }

    fn failed(media: &str, progress: f64, message: String) -> Note {
        Note {
            message: Some(message),
            ..Note::new(media, "failed", progress)
        }
    }
}

/// What became of a proxy that was made, once it was to be set on its media.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Made {
    /// It is set on the media.
    Set,
    /// The media is gone, or is of another file: the proxy is deleted, unless other media
    /// has it.
    Gone,
}

/// What the maker of proxies calls in the engine: to tell the clients, and to set a proxy that
/// it made on its media. Neither is called while the queue is locked.
pub struct Hooks {
    pub notify: Tell,
    pub made: SetMade,
    pub twin: Twin,
}

/// The proxy of another media item of the same file as media, if one has a proxy that may be
/// used (see [`twin_of`]).
pub type Twin = Box<dyn Fn(&str) -> Option<Proxy> + Send + Sync>;

/// The proxy of other media of `project` that is of the same file as media `id`, to be set on
/// it rather than made again: other media whose digest and facts are those of the media (the
/// digest reads only the size and the first and last 64 KiB of a file, which two recordings
/// written into files laid out ahead may share), with a proxy made from its file whose file is
/// there.
pub fn twin_of(project: &Project, id: &str) -> Option<Proxy> {
    let media = project.media.iter().find(|media| media.id == id)?;
    project
        .media
        .iter()
        .filter(|other| {
            other.id != media.id
                && other.identity.digest == media.identity.digest
                && other.facts == media.facts
        })
        .find_map(|other| usable(other).filter(|proxy| Path::new(&proxy.path).is_file()))
        .cloned()
}

/// How what a proxy is at is told.
pub type Tell = Box<dyn Fn(&Note) + Send + Sync>;

/// How a proxy that was made is set on its media.
pub type SetMade = Box<dyn Fn(&str, &Proxy) -> Made + Send + Sync>;

/// Media to make a proxy of.
#[derive(Clone, Debug)]
struct Job {
    media: String,
    source: PathBuf,
    digest: String,
    video: VideoFacts,
    duration: Time,
    /// Whether it is known to need a proxy; if not, that is found first.
    checked: bool,
}

impl Job {
    /// The job of media, if it has a picture.
    fn of(media: &Media) -> Option<Job> {
        Some(Job {
            media: media.id.clone(),
            source: PathBuf::from(&media.path),
            digest: media.identity.digest.clone(),
            video: media.facts.video.clone()?,
            duration: media.facts.duration,
            checked: false,
        })
    }

    /// How many frames the media has, as its facts say.
    fn frames(&self) -> i64 {
        clock::last_frame(self.duration, self.video.rate) + 1
    }
}

/// The job being done.
struct Running {
    media: String,
    cancelled: Arc<AtomicBool>,
    /// The `ffmpeg` that makes it, while it runs, to be killed.
    child: Arc<Mutex<Option<Child>>>,
}

#[derive(Default)]
struct Jobs {
    queue: VecDeque<Job>,
    running: Option<Running>,
    closing: bool,
}

struct Inner {
    /// `<project>/cache/proxies`.
    dir: PathBuf,
    ffmpeg: Option<PathBuf>,
    setup: Setup,
    jobs: Mutex<Jobs>,
    wake: Condvar,
    thread: Mutex<Option<JoinHandle<()>>>,
    /// What `ffmpeg -encoders` said, once it was asked.
    encoders: Mutex<Option<String>>,
    /// Whether NVENC itself failed: it is not tried again.
    nvenc_failed: AtomicBool,
    /// Why the proxy of each file (by its digest) could not be made, as `failed.json` keeps it.
    failures: Mutex<BTreeMap<String, String>>,
}

impl Inner {
    /// Why the proxy of a file could not be made before, if it could not.
    fn failure(&self, digest: &str) -> Option<String> {
        lock(&self.failures).get(digest).cloned()
    }

    /// Remember, or forget, that the proxy of a file cannot be made, and write it down.
    fn set_failure(&self, digest: &str, why: Option<&str>) {
        let mut failures = lock(&self.failures);
        let changed = match why {
            Some(why) => {
                failures
                    .insert(digest.to_owned(), why.to_owned())
                    .as_deref()
                    != Some(why)
            }
            None => failures.remove(digest).is_some(),
        };
        if !changed {
            return;
        }
        let path = self.dir.join(FAILED);
        let written = std::fs::create_dir_all(&self.dir).and_then(|()| {
            let part = self.dir.join(format!("{FAILED}.part"));
            let text = serde_json::to_string_pretty(&*failures).unwrap_or_default();
            std::fs::write(&part, text).and_then(|()| std::fs::rename(&part, &path))
        });
        if let Err(error) = written {
            log(
                "proxy_failed",
                &json!({ "message": format!("{} cannot be written: {error}", path.display()) }),
            );
        }
    }
}

/// The maker of proxies of an engine. It is cheap to clone. One that makes none does nothing.
#[derive(Clone)]
pub struct Proxies {
    inner: Option<Arc<Inner>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Proxies {
    /// The maker of proxies of the project in `project_dir`; it makes none until it is
    /// [`started`](Proxies::start), and never if `setup` says so.
    pub fn new(setup: &Setup, project_dir: &Path) -> Proxies {
        if !setup.make {
            return Proxies { inner: None };
        }
        let ffmpeg = find_ffmpeg(setup.ffmpeg.as_deref());
        log(
            "proxy_ffmpeg",
            &json!({ "path": ffmpeg.as_ref().map(|path| path.to_string_lossy()) }),
        );
        let dir = proxy_dir(project_dir);
        let failures = std::fs::read_to_string(dir.join(FAILED))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        Proxies {
            inner: Some(Arc::new(Inner {
                dir,
                ffmpeg,
                setup: setup.clone(),
                jobs: Mutex::default(),
                wake: Condvar::new(),
                thread: Mutex::new(None),
                encoders: Mutex::new(None),
                nvenc_failed: AtomicBool::new(false),
                failures: Mutex::new(failures),
            })),
        }
    }

    /// Start making proxies: of the media of `project` that has no proxy that can be used (none,
    /// one made from another file, or one whose file is not there), each once it is found to
    /// need one. The `.part` files that an engine that ended left are deleted first, and so are
    /// the proxies that no media of the project names (the history of a project is not kept
    /// from one engine to the next, so that nothing could name them again).
    pub fn start(&self, project: &Project, hooks: Hooks) {
        let Some(inner) = &self.inner else {
            return;
        };
        if let Ok(entries) = std::fs::read_dir(&inner.dir) {
            let named = |path: &Path| {
                project.media.iter().any(|media| {
                    media
                        .proxy
                        .as_ref()
                        .is_some_and(|proxy| Path::new(&proxy.path) == path)
                })
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let kept = path.file_name().is_some_and(|name| name == FAILED)
                    || !path.is_file()
                    || named(&path);
                if !kept && delete(&inner.dir, &path) {
                    log("proxy_deleted", &json!({ "path": path.to_string_lossy() }));
                }
            }
        }
        {
            let mut jobs = lock(&inner.jobs);
            for media in &project.media {
                let made = usable(media).is_some_and(|proxy| Path::new(&proxy.path).is_file());
                if !made {
                    jobs.queue.extend(Job::of(media));
                }
            }
        }
        let worker = Arc::clone(inner);
        let started = std::thread::Builder::new()
            .name("columbina-proxy".into())
            .spawn(move || run(&worker, &hooks));
        match started {
            Ok(thread) => *lock(&inner.thread) = Some(thread),
            Err(error) => log(
                "proxy_failed",
                &json!({ "message": format!("the maker of proxies cannot be started: {error}") }),
            ),
        }
    }

    /// Make the proxy of media of `project` that was taken in, once it is found to need one.
    pub fn imported(&self, project: &Project, media: &[String]) {
        let Some(inner) = &self.inner else {
            return;
        };
        let mut jobs = lock(&inner.jobs);
        for media in project.media.iter().filter(|item| media.contains(&item.id)) {
            jobs.queue.extend(Job::of(media));
        }
        inner.wake.notify_all();
    }

    /// Make the proxy of media again, as when its file could not be opened or
    /// decoded; unless it is being made, or waits to be.
    pub fn again(&self, media: &Media) {
        let Some(inner) = &self.inner else {
            return;
        };
        let mut jobs = lock(&inner.jobs);
        let waiting = jobs.queue.iter().any(|job| job.media == media.id)
            || jobs
                .running
                .as_ref()
                .is_some_and(|running| running.media == media.id);
        if !waiting {
            jobs.queue.extend(Job::of(media));
            inner.wake.notify_all();
        }
    }

    /// Make the proxy of media again, at the asking of a client
    /// (`media.make_proxy`): what is remembered of a proxy of its file that could not be made is
    /// forgotten. Whether proxies are made at all.
    pub fn retry(&self, media: &Media) -> bool {
        let Some(inner) = &self.inner else {
            return false;
        };
        inner.set_failure(&media.identity.digest, None);
        self.again(media);
        true
    }

    /// The project changed: the proxies of media that is no longer in it are not made, and the
    /// one being made is stopped. Gives what is to be told of those that waited; that of the one
    /// being made is told once it has stopped. This does not wait: it is called while the state
    /// of the engine is locked.
    pub fn project_changed(&self, project: &Project) -> Vec<Note> {
        let Some(inner) = &self.inner else {
            return Vec::new();
        };
        let there = |job: &str| project.media.iter().any(|media| media.id == job);
        let mut jobs = lock(&inner.jobs);
        let mut notes = Vec::new();
        jobs.queue.retain(|job| {
            let keep = there(&job.media);
            if !keep && job.checked {
                notes.push(Note::failed(&job.media, 0.0, removed()));
            }
            keep
        });
        if let Some(running) = &jobs.running {
            if !there(&running.media) {
                cancel(running);
            }
        }
        notes
    }

    /// Stop the proxy being made, delete its `.part`, and make no more: the engine ends. This
    /// waits a few seconds at most.
    pub fn stop(&self) {
        let Some(inner) = &self.inner else {
            return;
        };
        {
            let mut jobs = lock(&inner.jobs);
            jobs.closing = true;
            jobs.queue.clear();
            if let Some(running) = &jobs.running {
                cancel(running);
            }
            inner.wake.notify_all();
        }
        let thread = lock(&inner.thread).take();
        if let Some(thread) = thread {
            let deadline = Instant::now() + STOPPING;
            while !thread.is_finished() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
            if thread.is_finished() {
                let _ = thread.join();
            }
        }
    }
}

/// The message of a proxy of media that was removed.
fn removed() -> String {
    "the media was removed from the project".into()
}

/// Stop a job: it is not set, and its `ffmpeg` is killed.
fn cancel(running: &Running) {
    running.cancelled.store(true, Ordering::Release);
    if let Some(child) = lock(&running.child).as_mut() {
        let _ = child.kill();
    }
}

/// The thread of the maker: check each job that is not checked, then make the proxies, one at a
/// time, until the engine ends.
fn run(inner: &Inner, hooks: &Hooks) {
    loop {
        let (job, cancelled, child) = {
            let mut jobs = lock(&inner.jobs);
            loop {
                if jobs.closing {
                    return;
                }
                let next = jobs
                    .queue
                    .iter()
                    .position(|job| !job.checked)
                    .or_else(|| (!jobs.queue.is_empty()).then_some(0));
                if let Some(at) = next {
                    let job = jobs.queue.remove(at).expect("it is there");
                    let cancelled = Arc::new(AtomicBool::new(false));
                    let child = Arc::new(Mutex::new(None));
                    jobs.running = Some(Running {
                        media: job.media.clone(),
                        cancelled: Arc::clone(&cancelled),
                        child: Arc::clone(&child),
                    });
                    break (job, cancelled, child);
                }
                jobs = inner
                    .wake
                    .wait(jobs)
                    .unwrap_or_else(PoisonError::into_inner);
            }
        };
        if job.checked {
            let note = make(inner, hooks, &job, &cancelled, &child);
            let closing = {
                let mut jobs = lock(&inner.jobs);
                jobs.running = None;
                jobs.closing
            };
            // As the engine ends, its state is no longer to be told.
            if !closing {
                (hooks.notify)(&note);
            }
        } else {
            let needed = check(&job);
            let failed_before = needed.then(|| inner.failure(&job.digest)).flatten();
            let mut jobs = lock(&inner.jobs);
            jobs.running = None;
            if needed && !cancelled.load(Ordering::Acquire) && !jobs.closing {
                drop(jobs);
                if let Some(why) = failed_before {
                    let message = format!(
                        "it could not be made before, and is not tried again until \
                         media.make_proxy asks for it: {why}"
                    );
                    log(
                        "proxy_skipped",
                        &json!({ "media": job.media, "message": message }),
                    );
                    (hooks.notify)(&Note::failed(&job.media, 0.0, message));
                    continue;
                }
                lock(&inner.jobs).queue.push_back(Job {
                    checked: true,
                    ..job.clone()
                });
                log("proxy_queued", &json!({ "media": job.media }));
                (hooks.notify)(&Note::new(&job.media, "queued", 0.0));
            }
        }
    }
}

/// Whether the media of a job needs a proxy, from the first packets of its picture.
fn check(job: &Job) -> bool {
    match columbina_media::gop(&job.source, GOP_PACKETS) {
        Ok(gop) => needs_proxy(&job.video, &gop),
        Err(error) => {
            log(
                "proxy_skipped",
                &json!({ "media": job.media, "message": error.to_string() }),
            );
            false
        }
    }
}

/// How a proxy is encoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Recipe {
    /// Decoded and scaled on the card with CUDA, encoded by NVENC.
    Cuda,
    /// Decoded and scaled on the processor, encoded by NVENC.
    Nvenc,
    /// Decoded and scaled on the processor, encoded in `ProRes` 422 Proxy.
    Prores,
}

impl Recipe {
    fn name(self) -> &'static str {
        match self {
            Recipe::Cuda => "hevc_nvenc (cuda)",
            Recipe::Nvenc => "hevc_nvenc",
            Recipe::Prores => "prores_ks",
        }
    }
}

/// The arguments of `ffmpeg` that make the proxy of a job by a recipe, from `source` (the stream
/// `stream` of it) into `part`; the media read at most `readrate` times as fast as it plays, and,
/// where the frames are made on the processor, written so too.
fn arguments(
    recipe: Recipe,
    job: &Job,
    stream: usize,
    part: &Path,
    readrate: Option<&str>,
) -> Vec<String> {
    let (width, height) = proxy_size(job.video.width, job.video.height);
    let ten = job.video.bits > 8;
    let mut args: Vec<String> = [
        "-hide_banner",
        "-nostdin",
        "-loglevel",
        "error",
        "-nostats",
        "-progress",
        "pipe:1",
        "-y",
        // The input: a file, and nothing that a file names (a playlist), of the demuxers that
        // the engine reads media with, as it is stored.
        "-protocol_whitelist",
        "file",
        "-format_whitelist",
        &columbina_media::DEMUXERS.join(","),
        "-noautorotate",
    ]
    .iter()
    .map(|&arg| arg.to_owned())
    .collect();
    if let Some(rate) = readrate {
        args.extend(["-readrate".into(), rate.to_owned()]);
    }
    if recipe == Recipe::Cuda {
        args.extend(
            ["-hwaccel", "cuda", "-hwaccel_output_format", "cuda"]
                .iter()
                .map(|&arg| arg.to_owned()),
        );
    }
    args.push("-i".into());
    args.push(format!("file:{}", job.source.to_string_lossy()));
    // The picture alone, each frame of it with its timestamp, in its time base.
    args.extend([
        "-map".to_owned(),
        format!("0:{stream}"),
        "-an".into(),
        "-sn".into(),
        "-dn".into(),
        "-map_metadata".into(),
        "-1".into(),
        "-map_chapters".into(),
        "-1".into(),
        "-fps_mode".into(),
        "passthrough".into(),
        "-enc_time_base:v".into(),
        "demux".into(),
    ]);
    let same = (width, height) == (job.video.width, job.video.height);
    let nvenc_format = if ten { "p010le" } else { "nv12" };
    match recipe {
        Recipe::Cuda => {
            args.push("-vf".into());
            args.push(format!("scale_cuda={width}:{height}:format={nvenc_format}"));
        }
        Recipe::Nvenc | Recipe::Prores => {
            let mut filters = Vec::new();
            if !same {
                filters.push(format!("scale={width}:{height}:flags=bicubic"));
            }
            if let Some(rate) = readrate {
                // Frames as slow as the media is read, for the tests: `-readrate` alone lets
                // frames through at some rate however slow it is asked to be.
                filters.push(format!("realtime=speed={rate}:limit=3600"));
            }
            if !filters.is_empty() {
                args.push("-vf".into());
                args.push(filters.join(","));
            }
            args.push("-pix_fmt".into());
            args.push(
                if recipe == Recipe::Prores {
                    "yuv422p10le"
                } else {
                    nvenc_format
                }
                .into(),
            );
        }
    }
    args.extend(codec(recipe, ten).iter().map(|&arg| arg.to_owned()));
    args.extend(
        ["-protocol_whitelist", "file", "-f", "mov"]
            .iter()
            .map(|&arg| arg.to_owned()),
    );
    args.push(format!("file:{}", part.to_string_lossy()));
    args
}

/// The arguments of `ffmpeg` that choose the encoder of a recipe, for a picture of more than 8
/// bits (`ten`) or not.
fn codec(recipe: Recipe, ten: bool) -> &'static [&'static str] {
    match recipe {
        // `-g 0` is intra frames only (NVENC refuses `-g 1`).
        Recipe::Cuda | Recipe::Nvenc if ten => &[
            "-c:v",
            "hevc_nvenc",
            "-profile:v",
            "main10",
            "-g",
            "0",
            "-bf",
            "0",
            "-rc",
            "vbr",
            "-cq",
            "22",
        ],
        Recipe::Cuda | Recipe::Nvenc => &[
            "-c:v",
            "hevc_nvenc",
            "-profile:v",
            "main",
            "-g",
            "0",
            "-bf",
            "0",
            "-rc",
            "vbr",
            "-cq",
            "22",
        ],
        Recipe::Prores => &["-c:v", "prores_ks", "-profile:v", "0"],
    }
}

/// The recipes to try, in order, with what `ffmpeg -encoders` says; only those of `only`, where
/// it is given.
fn recipes(encoders: &str, nvenc_failed: bool, only: Option<Encoder>) -> Vec<Recipe> {
    let mut recipes = Vec::new();
    let nvenc = only.is_none_or(|only| only == Encoder::Nvenc);
    let prores = only.is_none_or(|only| only == Encoder::Prores);
    if nvenc && encoders.contains(" hevc_nvenc ") && !nvenc_failed {
        if cfg!(any(windows, target_os = "linux")) {
            recipes.push(Recipe::Cuda);
        }
        recipes.push(Recipe::Nvenc);
    }
    if prores && encoders.contains(" prores_ks ") {
        recipes.push(Recipe::Prores);
    }
    recipes
}

/// A command of `ffmpeg` that shows no window, at a low priority of the system.
fn command(ffmpeg: &Path) -> Command {
    #[cfg_attr(not(windows), allow(unused_mut, reason = "set only on Windows"))]
    let mut command = Command::new(ffmpeg);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        /// `CREATE_NO_WINDOW`.
        const NO_WINDOW: u32 = 0x0800_0000;
        /// `BELOW_NORMAL_PRIORITY_CLASS`.
        const BELOW_NORMAL: u32 = 0x0000_4000;
        command.creation_flags(NO_WINDOW | BELOW_NORMAL);
    }
    command
}

/// What `ffmpeg -encoders` says, asked once.
fn encoders(inner: &Inner, ffmpeg: &Path) -> String {
    let mut known = lock(&inner.encoders);
    if let Some(encoders) = known.as_ref() {
        return encoders.clone();
    }
    let said = command(ffmpeg)
        .args(["-hide_banner", "-encoders"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        .unwrap_or_default();
    *known = Some(said.clone());
    said
}

/// Make the proxy of a job that is known to need one (or set the proxy of other media of the
/// same file on it), set it on its media, and give what is to be told of it.
fn make(
    inner: &Inner,
    hooks: &Hooks,
    job: &Job,
    cancelled: &AtomicBool,
    child: &Arc<Mutex<Option<Child>>>,
) -> Note {
    if let Some(proxy) = &(hooks.twin)(&job.media) {
        return match (hooks.made)(&job.media, proxy) {
            Made::Set => {
                log(
                    "proxy_made",
                    &json!({ "media": job.media, "path": proxy.path, "encoder": null }),
                );
                Note::new(&job.media, "done", 1.0)
            }
            Made::Gone => failed(job, 0.0, removed()),
        };
    }
    let started = Instant::now();
    log(
        "proxy_started",
        &json!({ "media": job.media, "source": job.source.to_string_lossy() }),
    );
    (hooks.notify)(&Note::new(&job.media, "running", 0.0));
    match encode(inner, hooks, job, cancelled, child) {
        Ok((proxy, recipe, frames)) => match (hooks.made)(&job.media, &proxy) {
            Made::Set => {
                log(
                    "proxy_made",
                    &json!({
                        "media": job.media,
                        "path": proxy.path,
                        "encoder": recipe.name(),
                        "width": proxy.width,
                        "height": proxy.height,
                        "frames": frames,
                        "frames_of_media": job.frames(),
                        "seconds": started.elapsed().as_secs_f64(),
                    }),
                );
                Note::new(&job.media, "done", 1.0)
            }
            Made::Gone => failed(job, 1.0, removed()),
        },
        Err(failure) => {
            if failure.remember {
                inner.set_failure(&job.digest, Some(&failure.message));
            }
            failed(job, failure.progress, failure.message)
        }
    }
}

/// Whether a failure of `ffmpeg` is of the computer rather than of the file, and may pass: a
/// disk that is full, or a directory that may not be written. Such a failure is not remembered
/// in `failed.json`.
fn transient(message: &str) -> bool {
    let message = message.to_lowercase();
    [
        "no space left",
        "not enough space",
        "disk full",
        "disk quota",
        "permission denied",
        "access is denied",
        "read-only file system",
        "error opening output",
        "could not open file",
    ]
    .iter()
    .any(|said| message.contains(said))
}

/// What is told of a job that failed, once it is logged.
fn failed(job: &Job, progress: f64, message: String) -> Note {
    log(
        "proxy_failed",
        &json!({ "media": job.media, "message": message }),
    );
    Note::failed(&job.media, progress, message)
}

/// Why a proxy was not made.
struct Failure {
    /// How far it came.
    progress: f64,
    message: String,
    /// Whether the file itself is why: every encoder failed, or made a proxy that is not
    /// right. It is not tried again until a client asks for it.
    remember: bool,
}

impl Failure {
    /// The failure of a job that was stopped: its media was removed, or the engine ends.
    fn stopped(inner: &Inner, progress: f64) -> Failure {
        let message = if lock(&inner.jobs).closing {
            "the engine ended".into()
        } else {
            removed()
        };
        Failure {
            progress,
            message,
            remember: false,
        }
    }

    fn now(message: impl Into<String>) -> Failure {
        Failure {
            progress: 0.0,
            message: message.into(),
            remember: false,
        }
    }
}

/// The index of the stream of the picture of the media of a job, once its file is found to be
/// the one that was taken in.
fn picture_of(job: &Job) -> Result<usize, String> {
    let identity = columbina_media::identity(&job.source)
        .map_err(|error| format!("the file of the media cannot be read: {error}"))?;
    if identity.digest != job.digest {
        return Err("the file of the media is not the one that was taken in".into());
    }
    let gop = columbina_media::gop(&job.source, 1)
        .map_err(|error| format!("the media cannot be read: {error}"))?;
    Ok(gop.stream)
}

/// What came of one run of `ffmpeg`.
enum Ran {
    /// It ended well, having written this many frames.
    Wrote(i64),
    /// It failed, saying this.
    Failed(String),
    /// It wrote no frame for the time of the watchdog, and was killed.
    Stuck,
}

/// Where the proxy of a job is to be, as a path and as text, and its `.part`; the directory is
/// made.
fn destination(inner: &Inner, job: &Job) -> Result<(PathBuf, String, PathBuf), String> {
    std::fs::create_dir_all(&inner.dir)
        .map_err(|error| format!("{} cannot be made: {error}", inner.dir.display()))?;
    let path = inner.dir.join(file_name(&job.media, &job.digest));
    let text = path
        .to_str()
        .map(str::to_owned)
        .ok_or("the directory of the project is not UTF-8")?;
    let part = PathBuf::from(format!("{text}.part"));
    Ok((path, text, part))
}

/// Encode the proxy of a job by the first recipe that makes it, check it, and give it where it
/// is to be; or why it failed. A proxy that is cancelled is deleted.
fn encode(
    inner: &Inner,
    hooks: &Hooks,
    job: &Job,
    cancelled: &AtomicBool,
    child: &Arc<Mutex<Option<Child>>>,
) -> Result<(Proxy, Recipe, i64), Failure> {
    let Some(ffmpeg) = &inner.ffmpeg else {
        return Err(Failure::now(
            "there is no ffmpeg to make proxies with: none beside the engine, and \
             COLUMBINA_PROXY_FFMPEG names none",
        ));
    };
    let stream = picture_of(job).map_err(Failure::now)?;
    let (path, text, part) = destination(inner, job).map_err(Failure::now)?;
    let encoders = encoders(inner, ffmpeg);
    let recipes = recipes(
        &encoders,
        inner.nvenc_failed.load(Ordering::Acquire),
        inner.setup.encoder,
    );
    if recipes.is_empty() {
        return Err(Failure::now(format!(
            "{} has neither hevc_nvenc nor prores_ks to make proxies with",
            ffmpeg.display()
        )));
    }
    let mut why = Vec::new();
    let mut progress = 0.0;
    for recipe in recipes {
        let _ = delete(&inner.dir, &part);
        let args = arguments(recipe, job, stream, &part, inner.setup.readrate.as_deref());
        let ran = run_ffmpeg(
            inner,
            ffmpeg,
            &args,
            recipe,
            job,
            hooks,
            cancelled,
            child,
            &mut progress,
        );
        if cancelled.load(Ordering::Acquire) {
            let _ = delete(&inner.dir, &part);
            return Err(Failure::stopped(inner, progress));
        }
        let message = match ran {
            Ran::Wrote(frames) => match checked_proxy(&part, job) {
                Ok((width, height, codec)) => {
                    std::fs::rename(&part, &path).map_err(|error| {
                        let _ = delete(&inner.dir, &part);
                        Failure {
                            progress,
                            message: format!("the proxy cannot be put in place: {error}"),
                            remember: false,
                        }
                    })?;
                    let proxy = Proxy {
                        path: text,
                        width,
                        height,
                        codec,
                        digest_of_source: job.digest.clone(),
                    };
                    return Ok((proxy, recipe, frames));
                }
                Err(message) => message,
            },
            Ran::Failed(message) => {
                // NVENC itself failed (no card, no session): it is not tried again. A proxy
                // that is not right, or a failure of decoding, is the file's.
                let lower = message.to_lowercase();
                if recipe == Recipe::Nvenc && lower.contains("nvenc") {
                    inner.nvenc_failed.store(true, Ordering::Release);
                }
                message
            }
            Ran::Stuck => {
                let _ = delete(&inner.dir, &part);
                return Err(Failure {
                    progress,
                    message: format!(
                        "ffmpeg wrote no frame for {} s, and was stopped",
                        inner.setup.watchdog.as_secs()
                    ),
                    remember: false,
                });
            }
        };
        log(
            "proxy_recipe_failed",
            &json!({ "media": job.media, "encoder": recipe.name(), "message": message }),
        );
        why.push(format!("{}: {message}", recipe.name()));
        let _ = delete(&inner.dir, &part);
    }
    Err(Failure {
        progress,
        remember: !why.iter().any(|why| transient(why)),
        message: format!("the proxy cannot be made: {}", why.join("; ")),
    })
}

/// Read what a child says on stderr on a thread of its own, so that it never waits to write it,
/// keeping the last of it, to say why it failed.
fn drain(stderr: Option<std::process::ChildStderr>) -> Option<JoinHandle<String>> {
    std::thread::Builder::new()
        .name("columbina-proxy-stderr".into())
        .spawn(move || {
            let mut said = String::new();
            if let Some(mut stderr) = stderr {
                let mut buffer = [0_u8; 4096];
                while let Ok(read @ 1..) = stderr.read(&mut buffer) {
                    said.push_str(&String::from_utf8_lossy(&buffer[..read]));
                    if said.len() > 8192 {
                        let cut = said.len() - 4096;
                        let cut = (cut..said.len())
                            .find(|&at| said.is_char_boundary(at))
                            .unwrap_or(0);
                        said.drain(..cut);
                    }
                }
            }
            said
        })
        .ok()
}

/// What kills an `ffmpeg` that wrote no frame for its time.
struct Watchdog {
    moved: Arc<Mutex<Instant>>,
    done: Arc<AtomicBool>,
    stuck: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Watchdog {
    /// Watch the `ffmpeg` in `slot`, which may write no frame for `most`.
    fn start(slot: &Arc<Mutex<Option<Child>>>, most: Duration) -> Watchdog {
        let moved = Arc::new(Mutex::new(Instant::now()));
        let done = Arc::new(AtomicBool::new(false));
        let stuck = Arc::new(AtomicBool::new(false));
        let thread = std::thread::Builder::new()
            .name("columbina-proxy-watchdog".into())
            .spawn({
                let (moved, done, stuck) =
                    (Arc::clone(&moved), Arc::clone(&done), Arc::clone(&stuck));
                let slot = Arc::clone(slot);
                move || {
                    while !done.load(Ordering::Acquire) {
                        std::thread::sleep(Duration::from_millis(100));
                        if lock(&moved).elapsed() > most {
                            stuck.store(true, Ordering::Release);
                            if let Some(child) = lock(&slot).as_mut() {
                                let _ = child.kill();
                            }
                            return;
                        }
                    }
                }
            })
            .ok();
        Watchdog {
            moved,
            done,
            stuck,
            thread,
        }
    }

    /// A frame was written.
    fn moved(&self) {
        *lock(&self.moved) = Instant::now();
    }

    /// Stop watching; whether it killed the `ffmpeg`.
    fn stop(self) -> bool {
        self.done.store(true, Ordering::Release);
        if let Some(thread) = self.thread {
            let _ = thread.join();
        }
        self.stuck.load(Ordering::Acquire)
    }
}

/// Run `ffmpeg`, telling how far it is at most once a [`TOLD_EVERY`], until it ends, is killed,
/// or writes no frame for the time of the watchdog (then it is killed).
#[allow(
    clippy::too_many_arguments,
    reason = "one run, and all it needs to tell of it"
)]
fn run_ffmpeg(
    inner: &Inner,
    ffmpeg: &Path,
    args: &[String],
    recipe: Recipe,
    job: &Job,
    hooks: &Hooks,
    cancelled: &AtomicBool,
    slot: &Arc<Mutex<Option<Child>>>,
    progress: &mut f64,
) -> Ran {
    let spawned = command(ffmpeg)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut spawned = match spawned {
        Ok(spawned) => spawned,
        Err(error) => {
            return Ran::Failed(format!("{} cannot be started: {error}", ffmpeg.display()))
        }
    };
    contain(&spawned);
    log(
        "proxy_encoding",
        &json!({ "media": job.media, "encoder": recipe.name(), "pid": spawned.id() }),
    );
    let stdout = spawned.stdout.take();
    let stderr = spawned.stderr.take();
    {
        let mut slot = lock(slot);
        if cancelled.load(Ordering::Acquire) {
            let _ = spawned.kill();
        }
        *slot = Some(spawned);
    }
    let errors = drain(stderr);
    let watchdog = Watchdog::start(slot, inner.setup.watchdog);
    let total = job.frames().max(1);
    let mut frames = 0_i64;
    let mut told = Instant::now();
    if let Some(stdout) = stdout {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else {
                break;
            };
            if let Some(count) = line.strip_prefix("frame=") {
                let count = count.trim().parse().unwrap_or(frames);
                if count != frames {
                    watchdog.moved();
                }
                frames = count;
                #[allow(clippy::cast_precision_loss, reason = "a fraction, for the clients")]
                let fraction = (frames as f64 / total as f64).clamp(0.0, 1.0);
                *progress = fraction;
            }
            if line.starts_with("progress=") && told.elapsed() >= TOLD_EVERY {
                told = Instant::now();
                (hooks.notify)(&Note::new(&job.media, "running", *progress));
            }
        }
    }
    let status = lock(slot).take().map(|mut child| child.wait());
    let stuck = watchdog.stop();
    let said = errors
        .and_then(|thread| thread.join().ok())
        .unwrap_or_default();
    if stuck {
        return Ran::Stuck;
    }
    match status {
        Some(Ok(status)) if status.success() => Ran::Wrote(frames),
        Some(Ok(status)) => Ran::Failed(format!("ffmpeg ended with {status}: {}", cause(&said))),
        Some(Err(error)) => Ran::Failed(format!("ffmpeg cannot be waited for: {error}")),
        None => Ran::Failed("ffmpeg was lost".into()),
    }
}

/// The line of what `ffmpeg` said on stderr that says why it failed: the last, unless NVENC
/// itself said why (its lines begin `[hevc_nvenc @`), as it does where there is no card or no
/// driver of NVIDIA, and the last line then says only that nothing was written. That line names
/// NVENC, so that it is not tried again.
fn cause(said: &str) -> &str {
    let mut lines = said.lines().map(str::trim).filter(|line| !line.is_empty());
    lines
        .clone()
        .find(|line| line.starts_with("[hevc_nvenc @"))
        .or_else(|| lines.next_back())
        .unwrap_or("it said nothing")
}

/// Check a proxy that was made: its first and last frames are numbered as those of the media,
/// at the rate of the media, and every frame of its start is a key frame. Gives its width,
/// height and codec.
fn checked_proxy(part: &Path, job: &Job) -> Result<(u32, u32, String), String> {
    let facts = columbina_media::probe(part)
        .map_err(|error| format!("the proxy cannot be read: {error}"))?;
    let video = facts.video.ok_or("the proxy has no picture")?;
    let rate = job.video.rate;
    let media = columbina_media::frame_numbers(&job.source, rate, CHECKED_FRAMES)
        .map_err(|error| format!("the media cannot be read: {error}"))?;
    let proxy = columbina_media::frame_numbers(part, rate, CHECKED_FRAMES)
        .map_err(|error| format!("the proxy cannot be read: {error}"))?;
    if media != proxy {
        return Err(format!(
            "the frames of the proxy are not numbered as those of the media: the first are \
             {:?} and {:?}, the last {:?} and {:?}",
            proxy.0, media.0, proxy.1, media.1
        ));
    }
    let gop = columbina_media::gop(part, GOP_PACKETS)
        .map_err(|error| format!("the proxy cannot be read: {error}"))?;
    if !gop.all_intra() {
        return Err("the frames of the proxy are not all key frames".into());
    }
    Ok((video.width, video.height, video.codec))
}

/// Put `ffmpeg` in a job of the system that ends with the engine, so that it does not go on if
/// the engine is killed. Only on Windows; elsewhere it ends when it is done, and the next
/// engine deletes what it left.
fn contain(child: &Child) {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use std::sync::OnceLock;
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        };
        /// The job, as the number of its handle: it is never closed, and is closed by the
        /// system as the engine ends, which kills what is in it.
        static JOB: OnceLock<Option<usize>> = OnceLock::new();
        let job = JOB.get_or_init(|| {
            // SAFETY: a job with no name and the default security; the limit is written from a
            // structure of its own size.
            #[allow(unsafe_code)]
            unsafe {
                let job = CreateJobObjectW(None, windows::core::PCWSTR::null()).ok()?;
                let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
                limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    std::ptr::from_ref(&limits).cast(),
                    u32::try_from(std::mem::size_of_val(&limits)).ok()?,
                )
                .ok()?;
                Some(job.0 as usize)
            }
        });
        if let Some(job) = *job {
            // SAFETY: the job's handle is open for as long as the engine runs, and the child's
            // for as long as it is held.
            #[allow(unsafe_code)]
            let assigned = unsafe {
                AssignProcessToJobObject(
                    HANDLE(job as *mut std::ffi::c_void),
                    HANDLE(child.as_raw_handle()),
                )
            };
            if let Err(error) = assigned {
                log("proxy_job_object", &json!({ "message": error.to_string() }));
            }
        }
    }
    #[cfg(unix)]
    {
        // A low priority (nice 10).
        let pid = rustix::process::Pid::from_child(child);
        let _ = rustix::process::setpriority_process(Some(pid), 10);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use columbina_media::Rate;

    fn video(width: u32, height: u32, bits: u8, codec: &str) -> VideoFacts {
        VideoFacts {
            width,
            height,
            rate: Rate { num: 25, den: 1 },
            codec: codec.into(),
            bits,
            rotation: 0,
        }
    }

    fn gop(intra: bool, b_frames: bool) -> Gop {
        Gop {
            stream: 0,
            packets: 64,
            keys: if intra { 64 } else { 2 },
            b_frames,
        }
    }

    #[test]
    fn media_that_is_slow_to_seek_needs_a_proxy() {
        // (width, height, bits, codec, every frame a key frame, B-frames, needs one)
        let table = [
            // 4K HEVC Main 10 of a camera: long groups, B-frames.
            (3840, 2160, 10, "hevc", false, true, true),
            // Each of the four alone, with long groups.
            (3840, 2160, 8, "h264", false, false, true),
            (1920, 1080, 10, "h264", false, false, true),
            (1280, 720, 8, "hevc", false, false, true),
            (1920, 1080, 8, "av1", false, false, true),
            (1920, 1080, 8, "h264", false, true, true),
            (1920, 1080, 8, "mpeg2video", false, true, true),
            // 1080p H.264 of 8 bits without B-frames: quick enough.
            (1920, 1080, 8, "h264", false, false, false),
            (1280, 720, 8, "h264", false, false, false),
            (1920, 1080, 8, "vp9", false, false, false),
            // Every frame a key frame: never.
            (1920, 1080, 10, "prores", true, false, false),
            (3840, 2160, 10, "prores", true, false, false),
            (3840, 2160, 10, "dnxhd", true, false, false),
            (1920, 1080, 8, "mjpeg", true, false, false),
            (1920, 1080, 8, "h264", true, false, false),
            (3840, 2160, 10, "hevc", true, false, false),
        ];
        for (width, height, bits, codec, intra, b_frames, wanted) in table {
            assert_eq!(
                needs_proxy(&video(width, height, bits, codec), &gop(intra, b_frames)),
                wanted,
                "{width}×{height} {bits} bits {codec}, intra {intra}, B-frames {b_frames}"
            );
        }
        // No packet read is no key frame.
        let none = Gop {
            stream: 0,
            packets: 0,
            keys: 0,
            b_frames: false,
        };
        assert!(needs_proxy(&video(3840, 2160, 10, "hevc"), &none));
    }

    #[test]
    fn a_proxy_is_at_most_1920_on_its_long_edge_in_even_pixels() {
        assert_eq!(proxy_size(3840, 2160), (1920, 1080));
        assert_eq!(proxy_size(7680, 4320), (1920, 1080));
        assert_eq!(proxy_size(4096, 2160), (1920, 1012));
        assert_eq!(proxy_size(2160, 3840), (1080, 1920));
        assert_eq!(proxy_size(1920, 1080), (1920, 1080));
        assert_eq!(proxy_size(640, 360), (640, 360));
        assert_eq!(proxy_size(641, 361), (640, 360));
        assert_eq!(proxy_size(5760, 3240), (1920, 1080));
        assert_eq!(proxy_size(1, 1), (2, 2));
    }

    #[test]
    fn a_proxy_is_named_by_its_media_and_its_digest() {
        let digest = "0123456789abcdef".repeat(4);
        assert_eq!(file_name("m12", &digest), "m12-0123456789ab.mov");
        assert_eq!(file_name("a/b:c", &digest), "a_b_c-0123456789ab.mov");
    }

    #[test]
    fn the_recipes_are_nvenc_then_prores() {
        let both = " V....D hevc_nvenc  NVIDIA\n VFS... prores_ks  Apple\n";
        let cuda: &[Recipe] = if cfg!(any(windows, target_os = "linux")) {
            &[Recipe::Cuda, Recipe::Nvenc, Recipe::Prores]
        } else {
            &[Recipe::Nvenc, Recipe::Prores]
        };
        assert_eq!(recipes(both, false, None), cuda);
        assert_eq!(recipes(both, true, None), [Recipe::Prores]);
        assert_eq!(
            recipes(both, false, Some(Encoder::Prores)),
            [Recipe::Prores]
        );
        assert_eq!(
            recipes(both, false, Some(Encoder::Nvenc)),
            cuda[..cuda.len() - 1]
        );
        assert_eq!(
            recipes(" VFS... prores_ks  Apple\n", false, None),
            [Recipe::Prores]
        );
        assert_eq!(recipes("", false, None), []);
        assert_eq!(Encoder::parse("prores"), Ok(Encoder::Prores));
        assert!(Encoder::parse("x264").is_err());
    }

    #[test]
    fn the_proxy_keeps_every_frame_and_its_timestamp_and_no_sound() {
        let job = Job {
            media: "m1".into(),
            source: PathBuf::from("/media/a b.mp4"),
            digest: "0".repeat(64),
            video: video(3840, 2160, 10, "hevc"),
            duration: Time {
                ticks: 10,
                scale: 1,
            },
            checked: true,
        };
        let args = arguments(Recipe::Cuda, &job, 2, Path::new("/p/m1.mov.part"), None);
        let text = args.join(" ");
        for wanted in [
            "-protocol_whitelist file",
            "-noautorotate",
            "-hwaccel cuda -hwaccel_output_format cuda -i file:/media/a b.mp4",
            "-map 0:2 -an",
            "-fps_mode passthrough -enc_time_base:v demux",
            "-vf scale_cuda=1920:1080:format=p010le",
            "-c:v hevc_nvenc -profile:v main10 -g 0 -bf 0",
            "-f mov file:/p/m1.mov.part",
        ] {
            assert!(text.contains(wanted), "{wanted}: {text}");
        }
        assert!(!text.contains("-readrate"));
        let eight = Job {
            video: video(640, 360, 8, "h264"),
            ..job
        };
        let text = arguments(Recipe::Nvenc, &eight, 0, Path::new("/p.part"), Some("1")).join(" ");
        assert!(text.contains("-readrate 1"), "{text}");
        assert!(text.contains("-vf realtime=speed=1:limit=3600"), "{text}");
        let text = arguments(Recipe::Nvenc, &eight, 0, Path::new("/p.part"), None).join(" ");
        assert!(!text.contains("-vf"), "{text}");
        assert!(
            text.contains("-pix_fmt nv12 -c:v hevc_nvenc -profile:v main"),
            "{text}"
        );
        let text = arguments(Recipe::Prores, &eight, 0, Path::new("/p.part"), None).join(" ");
        assert!(
            text.contains("-pix_fmt yuv422p10le -c:v prores_ks -profile:v 0"),
            "{text}"
        );
    }

    #[test]
    fn a_failure_is_remembered_by_the_digest_of_the_file_and_forgotten_when_asked() {
        let dir = std::env::temp_dir().join(format!("columbina-proxy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let setup = Setup::default();
        let proxies = Proxies::new(&setup, &dir);
        let inner = proxies.inner.as_ref().unwrap();
        assert_eq!(inner.failure("abc"), None);
        inner.set_failure("abc", Some("no encoder made it"));
        let again = Proxies::new(&setup, &dir);
        let inner = again.inner.as_ref().unwrap();
        assert_eq!(inner.failure("abc").as_deref(), Some("no encoder made it"));
        inner.set_failure("abc", None);
        assert_eq!(
            Proxies::new(&setup, &dir).inner.unwrap().failure("abc"),
            None
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_a_file_of_the_proxies_of_the_project_is_one_of_its_proxies_or_deleted() {
        let root = std::env::temp_dir().join(format!("columbina-owned-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let project = root.join("project");
        let dir = proxy_dir(&project);
        std::fs::create_dir_all(&dir).unwrap();
        let outside = root.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        let name = "m1-0123456789ab.mov";
        let write = |path: &Path| std::fs::write(path, b"x").unwrap();
        // Its own: named so, in the directory.
        let own = dir.join(name);
        write(&own);
        assert!(owned(&dir, &own));
        assert!(delete(&dir, &own));
        assert!(!own.exists());
        let part = dir.join(format!("{name}.part"));
        write(&part);
        assert!(!owned(&dir, &part), "a .part is not a proxy");
        assert!(delete(&dir, &part), "but it is deleted");
        // A file of the same name elsewhere, through `..`, or of another name: never.
        let elsewhere = outside.join(name);
        write(&elsewhere);
        let through = dir
            .join("..")
            .join("..")
            .join("..")
            .join("outside")
            .join(name);
        for path in [&elsewhere, &through] {
            assert!(!owned(&dir, path), "{}", path.display());
            assert!(!delete(&dir, path), "{}", path.display());
        }
        assert!(elsewhere.exists());
        for name in [
            "notes.txt",
            "m1-0123456789AB.mov",
            "m1-0123456789a.mov",
            "-0123456789ab.mov",
            "m1.mov",
            "m 1-0123456789ab.mov",
        ] {
            let path = dir.join(name);
            write(&path);
            assert!(!owned(&dir, &path), "{name}");
            assert!(!delete(&dir, &path), "{name}");
            assert!(path.exists(), "{name}");
        }
        assert!(!owned(&dir, Path::new(name)), "a relative path");
        // The directory of the proxies a link or a junction to somewhere else: never.
        let linked = root.join("linked");
        std::fs::create_dir_all(linked.join("cache")).unwrap();
        let link = proxy_dir(&linked);
        #[cfg(windows)]
        let made = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&outside)
            .output()
            .is_ok_and(|output| output.status.success());
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(&outside, &link).is_ok();
        assert!(made, "the link is made");
        let through_link = link.join(name);
        assert!(through_link.exists());
        assert!(!owned(&link, &through_link));
        assert!(!delete(&link, &through_link));
        assert!(elsewhere.exists());
        // A project read with a proxy elsewhere has it taken away.
        let mut read = Project::sample();
        read.media[0].proxy = Some(Proxy {
            path: elsewhere.to_str().unwrap().to_owned(),
            width: 1,
            height: 1,
            codec: "hevc".into(),
            digest_of_source: "0".repeat(64),
        });
        assert_eq!(owned_only(read, &dir).media[0].proxy, None);
        let _ = std::fs::remove_dir(&link);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_proxy_is_shared_only_by_media_of_the_same_digest_and_facts() {
        let dir = std::env::temp_dir().join(format!("columbina-twin-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("m1-000000000000.mov");
        std::fs::write(&file, b"a proxy").unwrap();
        let mut project = Project::sample();
        project.media[0].proxy = Some(Proxy {
            path: file.to_str().unwrap().to_owned(),
            width: 1920,
            height: 1080,
            codec: "hevc".into(),
            digest_of_source: project.media[0].identity.digest.clone(),
        });
        let mut twin = project.media[0].clone();
        twin.id = "m2".into();
        twin.proxy = None;
        project.media.push(twin);
        assert_eq!(twin_of(&project, "m2"), project.media[0].proxy);
        // The same digest, and another picture: a file laid out ahead, recorded otherwise.
        for change in [
            |media: &mut Media| media.facts.duration.ticks += 1,
            |media: &mut Media| media.facts.video.as_mut().unwrap().width = 1280,
            |media: &mut Media| media.facts.video.as_mut().unwrap().bits = 10,
            |media: &mut Media| media.facts.video.as_mut().unwrap().codec = "hevc".into(),
            |media: &mut Media| media.facts.video.as_mut().unwrap().rate.num = 50,
        ] {
            let mut other = project.clone();
            change(&mut other.media[1]);
            assert_eq!(twin_of(&other, "m2"), None);
        }
        let mut other = project.clone();
        other.media[1].identity.digest = "f".repeat(64);
        assert_eq!(twin_of(&other, "m2"), None);
        // Nor one whose file is gone.
        std::fs::remove_file(&file).unwrap();
        assert_eq!(twin_of(&project, "m2"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn nvenc_that_cannot_start_is_named_as_why_ffmpeg_failed() {
        // What the ffmpeg of FFmpeg 8.1 says where there is no card of NVIDIA: its last line
        // does not name NVENC.
        let said = [
            "[hevc_nvenc @ 01] dl_fn->cuda_dl->cuInit(0) failed -> CUDA_ERROR_NO_DEVICE: no \
             CUDA-capable device is detected",
            "[vost#0:0/hevc_nvenc @ 02] [enc:hevc_nvenc @ 03] Error while opening encoder - maybe \
             incorrect parameters such as bit_rate, rate, width or height.",
            "[vf#0:0 @ 04] Error sending frames to consumers: Unknown error occurred",
            "[vost#0:0/hevc_nvenc @ 02] [enc:hevc_nvenc @ 03] Could not open encoder before EOF",
            "[out#0/mov @ 05] Nothing was written into output file, because at least one of its \
             streams received no packets.",
            "",
        ]
        .join("\n");
        assert!(
            cause(&said).starts_with("[hevc_nvenc @ 01] dl_fn->cuda_dl->cuInit(0) failed"),
            "{}",
            cause(&said)
        );
        // Otherwise the last line, which says why.
        let said = "[in#0 @ 0] Error opening input: Invalid data found\nError opening x.mp4.\n\n";
        assert_eq!(cause(said), "Error opening x.mp4.");
        assert_eq!(cause(" \n"), "it said nothing");
    }

    #[test]
    fn a_full_disk_or_a_directory_that_may_not_be_written_is_not_a_failure_of_the_file() {
        for said in [
            "ffmpeg ended with exit code: 1: [out#0/mov] Error writing trailer: No space left on device",
            "ffmpeg ended with exit code: 1: av_interleaved_write_frame(): There is not enough space on the disk.",
            "ffmpeg ended with exit code: 1: [out#0/mov] Error opening output file:/p.part: Permission denied",
            "ffmpeg ended with exit code: 1: Error opening output files: Access is denied.",
            "ffmpeg ended with exit code: 1: Read-only file system",
        ] {
            assert!(transient(said), "{said}");
        }
        for said in [
            "ffmpeg ended with exit code: 1: Nothing was written into output file",
            "the frames of the proxy are not numbered as those of the media",
            "ffmpeg ended with exit code: 1: [hevc_nvenc] OpenEncodeSessionEx failed",
        ] {
            assert!(!transient(said), "{said}");
        }
    }
}
