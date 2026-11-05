//! The streams of sound of media that a project written before they were read lacks (plan T3c).
//!
//! Media probed before schema 4 read the streams of sound of its file (T3b) has a
//! `facts.audio` with no `streams` and no `layout`. As the engine opens such a project, a thread
//! of its own reads them from the files, away from the threads of sound and of requests, and
//! sets them by a change of the engine that is no step of the history (`set_media_streams`,
//! as a proxy that the engine made is set): it is told to every client as a `project.changed`
//! made by `"engine"`, saved as every change is (the engine saves each change by itself, and
//! asks no one), and never taken back by an undo; the steps that could be redone are kept.
//!
//! The media of each pass are read one after another, and set together, in one change. Media
//! whose file is not there, or cannot be read, is tried again every [`RETRY`] while the engine
//! runs, so that it is read once its drive is back. Media whose file is another file now (whose
//! digest is not that of the media, as a proxy of it would not be either), or one that libav
//! cannot open, is not read again until the file changes (its size or the time it was
//! modified): so a file that stays as it is is opened once. Every pass looks at the whole
//! project, so media that comes back without streams (an undo of its removal made before they
//! were read, or a client that adds it so) is read too.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Condvar, Weak};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use columbina_media::MediaError;
use columbina_model::{AudioStream, Id, Identity, Op, Project};
use serde_json::json;
use tokio::sync::Mutex;

use super::{log_kicked, State};
use crate::log::log;
use crate::queue::Limits;

/// How often media whose file could not be read is tried again (we decide; `DaVinci` finds offline
/// media again only as it is relinked).
pub const RETRY: Duration = Duration::from_secs(10);

/// Media whose streams of sound were not read, as a pass found it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Wanted {
    pub id: Id,
    pub path: String,
    pub identity: Identity,
}

/// What a pass found of the file of media.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Found {
    /// Its streams of sound, and the layout of the channels of its best one.
    Read {
        layout: Option<String>,
        streams: Vec<AudioStream>,
    },
    /// It is not there, or cannot be read now: it is tried again.
    Later,
    /// It is another file, or one that libav cannot open: it is tried again once it changes,
    /// as it was seen so.
    Kept(Seen),
}

/// A file as it was seen: its size and when it was modified.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Seen {
    size: u64,
    modified_us: i64,
}

/// The media of a project whose streams of sound were not read: those with sound whose
/// `streams` are not there, in the order of the media.
pub(super) fn wanted(project: &Project) -> Vec<Wanted> {
    project
        .media
        .iter()
        .filter(|media| {
            media
                .facts
                .audio
                .as_ref()
                .is_some_and(|audio| audio.streams.is_none())
        })
        .map(|media| Wanted {
            id: media.id.clone(),
            path: media.path.clone(),
            identity: media.identity.clone(),
        })
        .collect()
}

/// Read the streams of sound of the file of media: if it is the file of the media, its
/// streams; else what it is. This blocks.
pub(super) fn read(wanted: &Wanted) -> Found {
    let path = Path::new(&wanted.path);
    // Not there, or not to be read now.
    let Ok(identity) = columbina_media::identity(path) else {
        return Found::Later;
    };
    let seen = Seen {
        size: identity.size,
        modified_us: identity.modified_us,
    };
    if identity.digest != wanted.identity.digest {
        return Found::Kept(seen);
    }
    match columbina_media::sounds(path) {
        Ok(sounds) => {
            let layout = sounds
                .streams
                .iter()
                .find(|stream| Some(stream.index) == sounds.best)
                .and_then(|stream| stream.layout.clone());
            let streams = sounds
                .streams
                .into_iter()
                .filter_map(|stream| {
                    Some(AudioStream {
                        index: u32::try_from(stream.index).ok()?,
                        channels: stream.channels,
                        layout: stream.layout,
                    })
                })
                .collect();
            Found::Read { layout, streams }
        }
        // Gone, or not to be read, between the two.
        Err(MediaError::Io(_)) => Found::Later,
        Err(_) => Found::Kept(seen),
    }
}

/// Whether a file is as it was seen.
fn unchanged(path: &str, seen: Seen) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    let modified_us = metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|since| i64::try_from(since.as_micros()).ok());
    metadata.len() == seen.size && modified_us == Some(seen.modified_us)
}

/// The operations that set what was read of media, of those that the project still has as they
/// were found: the same file, and no streams yet.
pub(super) fn settled(project: &Project, read: Vec<(Wanted, Found)>) -> Vec<Op> {
    read.into_iter()
        .filter_map(|(wanted, found)| {
            let Found::Read { layout, streams } = found else {
                return None;
            };
            let media = project.media.iter().find(|media| media.id == wanted.id)?;
            let still = media.path == wanted.path
                && media.identity.digest == wanted.identity.digest
                && media
                    .facts
                    .audio
                    .as_ref()
                    .is_some_and(|audio| audio.streams.is_none());
            still.then_some(Op::SetMediaStreams {
                id: wanted.id,
                layout,
                streams: Some(streams),
            })
        })
        .collect()
}

/// What reads the streams of sound of the media of a project that lacks them, on a thread of its
/// own, until it is stopped or the engine has gone.
pub struct Reader {
    stop: Arc<(std::sync::Mutex<bool>, Condvar)>,
    /// The thread, until it is let go of as the reader is stopped.
    thread: std::sync::Mutex<Option<JoinHandle<()>>>,
}

impl Reader {
    /// Begin to read: a pass at once, and one every [`RETRY`] after it.
    pub(super) fn start(state: &Arc<Mutex<State>>, limits: Limits) -> Reader {
        Reader::every(Arc::downgrade(state), limits, RETRY)
    }

    fn every(state: Weak<Mutex<State>>, limits: Limits, retry: Duration) -> Reader {
        let stop = Arc::new((std::sync::Mutex::new(false), Condvar::new()));
        let stopped = Arc::clone(&stop);
        let spawned = std::thread::Builder::new()
            .name("columbina-streams".into())
            .spawn(move || {
                let mut memory = Memory::default();
                loop {
                    let Some(state) = state.upgrade() else {
                        return;
                    };
                    pass(&state, &mut memory, limits, &stopped);
                    drop(state);
                    let (lock, told) = &*stopped;
                    let guard = lock
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    let (guard, _) = told
                        .wait_timeout_while(guard, retry, |stop| !*stop)
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    if *guard {
                        return;
                    }
                }
            });
        let thread = match spawned {
            Ok(thread) => Some(thread),
            Err(error) => {
                log(
                    "media_streams_failed",
                    &json!({"message": format!("the streams of media cannot be read: {error}")}),
                );
                None
            }
        };
        Reader {
            stop,
            thread: std::sync::Mutex::new(thread),
        }
    }

    /// Stop reading. This does not wait for a file that is being read, which may be on a drive
    /// that takes long to answer: what is read after this is set on nothing, and the thread ends
    /// with it.
    pub fn stop(&self) {
        let (lock, told) = &*self.stop;
        *lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = true;
        told.notify_all();
        // Let go of the thread, which ends by itself.
        drop(
            self.thread
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take(),
        );
    }
}

/// Whether the reader was stopped.
fn stopped(stop: &(std::sync::Mutex<bool>, Condvar)) -> bool {
    *stop
        .0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// How many media read are set in one change, at most: the rest of a pass goes on in the next.
const BATCH: usize = 16;

/// How long a pass reads before what it read is set, at most.
const BATCH_TIME: Duration = Duration::from_secs(2);

/// The longest that media whose file could not be read waits to be tried again: the wait
/// doubles from [`RETRY`] each time it is tried in vain (we decide).
const LONGEST_WAIT: Duration = Duration::from_secs(320);

/// What the reader remembers of the media it could not read.
#[derive(Default)]
pub(super) struct Memory {
    /// Media whose file is another file, or one that libav cannot open, as it was seen: its
    /// path and digest as the media has them, and the file then.
    kept: HashMap<Id, (String, String, Seen)>,
    /// Media whose file was not there: when it is tried again, and how long it waits after.
    later: HashMap<Id, (Instant, Duration)>,
}

impl Memory {
    /// Whether media is to be read now: not one kept whose file is as it was seen, nor one that
    /// waits to be tried again.
    pub(super) fn due(&self, one: &Wanted, now: Instant) -> bool {
        let kept = self.kept.get(&one.id).is_some_and(|(path, digest, seen)| {
            *path == one.path && *digest == one.identity.digest && unchanged(path, *seen)
        });
        let waits = self.later.get(&one.id).is_some_and(|(at, _)| now < *at);
        !kept && !waits
    }

    /// Remember what was found of media.
    pub(super) fn found(&mut self, one: &Wanted, found: &Found, now: Instant) {
        match found {
            Found::Kept(seen) => {
                self.later.remove(&one.id);
                let kept = (one.path.clone(), one.identity.digest.clone(), *seen);
                self.kept.insert(one.id.clone(), kept);
            }
            Found::Later => {
                self.kept.remove(&one.id);
                let wait = self
                    .later
                    .get(&one.id)
                    .map_or(RETRY, |(_, wait)| (*wait * 2).min(LONGEST_WAIT));
                self.later.insert(one.id.clone(), (now + wait, wait));
            }
            Found::Read { .. } => {
                self.kept.remove(&one.id);
                self.later.remove(&one.id);
            }
        }
    }

    /// Forget media that is no longer wanted.
    fn keep_only(&mut self, wanted: &[Wanted]) {
        let still = |id: &Id| wanted.iter().any(|one| &one.id == id);
        self.kept.retain(|id, _| still(id));
        self.later.retain(|id, _| still(id));
    }
}

/// One pass: read what is wanted and due, and set what was read, [`BATCH`] media or
/// [`BATCH_TIME`] of reading at a time, each in one change of the engine.
fn pass(
    state: &Mutex<State>,
    memory: &mut Memory,
    limits: Limits,
    stop: &(std::sync::Mutex<bool>, Condvar),
) {
    let wanted = wanted(state.blocking_lock().history.project());
    memory.keep_only(&wanted);
    let mut found = Vec::new();
    let mut since = Instant::now();
    for one in wanted {
        if stopped(stop) {
            return;
        }
        if !memory.due(&one, Instant::now()) {
            continue;
        }
        let read = read(&one);
        memory.found(&one, &read, Instant::now());
        match read {
            Found::Kept(_) => log(
                "media_streams_kept",
                &json!({"media": one.id, "path": one.path,
                    "message": "the file is not that of the media, or cannot be opened"}),
            ),
            Found::Later => {}
            streams @ Found::Read { .. } => found.push((one, streams)),
        }
        if found.len() >= BATCH || (!found.is_empty() && since.elapsed() >= BATCH_TIME) {
            if !set(state, std::mem::take(&mut found), limits, stop) {
                return;
            }
            since = Instant::now();
        }
    }
    set(state, found, limits, stop);
}

/// Set what was read in one change of the engine; false where the reader was stopped.
fn set(
    state: &Mutex<State>,
    found: Vec<(Wanted, Found)>,
    limits: Limits,
    stop: &(std::sync::Mutex<bool>, Condvar),
) -> bool {
    if found.is_empty() {
        return true;
    }
    let mut locked = state.blocking_lock();
    // Stopped meanwhile, as the engine ends: nothing more is set.
    if stopped(stop) {
        return false;
    }
    let ops = settled(locked.history.project(), found);
    if ops.is_empty() {
        return true;
    }
    let count = ops.len();
    let applied = locked.history.apply_unrecorded(ops, "engine");
    if let Ok(applied) = &applied {
        locked.publish(applied);
    }
    let kicked = std::mem::take(&mut locked.kicked);
    drop(locked);
    for (id, kind) in kicked {
        log_kicked(id, Some(kind), limits);
    }
    match applied {
        Ok(_) => log("media_streams_read", &json!({"media": count})),
        Err(error) => log(
            "media_streams_failed",
            &json!({"message": error.to_string()}),
        ),
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    use columbina_model::Project;

    /// The sample project, its m1 a file of `path` as it is now.
    fn project_of(path: &Path) -> Project {
        let mut project = Project::sample();
        project.media[0].path = path.to_str().unwrap().into();
        project.media[0].identity = columbina_media::identity(path).unwrap();
        project
    }

    /// A WAV of two channels of silence, a tenth of a second long, that says they are the left
    /// and the right (`WAVE_FORMAT_EXTENSIBLE`, its mask 3), so that libav names their layout.
    fn wav(path: &Path) {
        wav_of(path, 4800);
    }

    /// As [`wav`], `samples` long.
    fn wav_of(path: &Path, samples: u32) {
        let data = samples * 4;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(60 + data).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&40_u32.to_le_bytes());
        bytes.extend_from_slice(&0xFFFE_u16.to_le_bytes());
        bytes.extend_from_slice(&2_u16.to_le_bytes());
        bytes.extend_from_slice(&48000_u32.to_le_bytes());
        bytes.extend_from_slice(&(48000_u32 * 4).to_le_bytes());
        bytes.extend_from_slice(&4_u16.to_le_bytes());
        bytes.extend_from_slice(&16_u16.to_le_bytes());
        bytes.extend_from_slice(&22_u16.to_le_bytes());
        bytes.extend_from_slice(&16_u16.to_le_bytes());
        bytes.extend_from_slice(&3_u32.to_le_bytes());
        // KSDATAFORMAT_SUBTYPE_PCM.
        bytes.extend_from_slice(&[
            0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xaa, 0x00, 0x38,
            0x9b, 0x71,
        ]);
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data.to_le_bytes());
        bytes.resize(bytes.len() + data as usize, 0);
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn media_of_sound_without_streams_is_wanted_and_no_other() {
        let mut project = Project::sample();
        assert_eq!(
            wanted(&project)
                .iter()
                .map(|one| one.id.as_str())
                .collect::<Vec<_>>(),
            ["m1"]
        );
        project.media[0].facts.audio.as_mut().unwrap().streams = Some(vec![]);
        assert_eq!(wanted(&project), [], "streams read, if none");
        project.media[0].facts.audio = None;
        assert_eq!(wanted(&project), [], "no sound");
    }

    #[test]
    fn the_streams_of_the_file_of_the_media_are_read_and_set() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.wav");
        wav(&path);
        let project = project_of(&path);
        let wanted = wanted(&project);
        let found = read(&wanted[0]);
        assert_eq!(
            found,
            Found::Read {
                layout: Some("stereo".into()),
                streams: vec![AudioStream {
                    index: 0,
                    channels: 2,
                    layout: Some("stereo".into()),
                }],
            }
        );
        let ops = settled(&project, vec![(wanted[0].clone(), found)]);
        assert_eq!(
            ops,
            [Op::SetMediaStreams {
                id: "m1".into(),
                layout: Some("stereo".into()),
                streams: Some(vec![AudioStream {
                    index: 0,
                    channels: 2,
                    layout: Some("stereo".into()),
                }]),
            }]
        );
    }

    #[test]
    fn a_file_that_is_not_there_is_read_later_and_another_file_is_kept_as_it_was_seen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.wav");
        wav(&path);
        let project = project_of(&path);
        let one = wanted(&project).remove(0);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(read(&one), Found::Later);
        // Another file at its path: its digest is not the media's.
        std::fs::write(&path, b"not the media").unwrap();
        let Found::Kept(seen) = read(&one) else {
            panic!("another file is kept as it was seen");
        };
        assert_eq!(seen.size, 13);
        // Media, but other media than the media's: its sound is not read of it.
        wav_of(&path, 9600);
        assert!(matches!(read(&one), Found::Kept(_)));
        std::fs::write(&path, b"not the media").unwrap();
        let Found::Kept(seen) = read(&one) else {
            panic!("another file is kept as it was seen");
        };
        assert!(unchanged(&one.path, seen));
        std::fs::write(&path, b"not the media, nor this").unwrap();
        assert!(
            !unchanged(&one.path, seen),
            "a file of another size changed"
        );
        // The file of the media, but not media: libav cannot open it.
        let broken = project_of(&path);
        let again = wanted(&broken).remove(0);
        assert!(matches!(read(&again), Found::Kept(_)));
    }

    #[test]
    fn media_away_waits_longer_each_time_and_media_kept_is_kept_by_its_path_and_digest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.wav");
        wav(&path);
        let project = project_of(&path);
        let one = wanted(&project).remove(0);
        let mut memory = Memory::default();
        let now = Instant::now();
        assert!(memory.due(&one, now));
        // Away: tried again after 10 s, then 20 s, 40 s, ... and 320 s at the most.
        memory.found(&one, &Found::Later, now);
        assert!(!memory.due(&one, now + (RETRY - Duration::from_millis(1))));
        assert!(memory.due(&one, now + RETRY));
        let mut at = now + RETRY;
        for wait in [20, 40, 80, 160, 320, 320] {
            memory.found(&one, &Found::Later, at);
            let wait = Duration::from_secs(wait);
            assert!(!memory.due(&one, at + (wait - Duration::from_millis(1))));
            assert!(memory.due(&one, at + wait));
            at += wait;
        }
        // Read at last: it waits no more.
        let read = Found::Read {
            layout: None,
            streams: vec![],
        };
        memory.found(&one, &read, at);
        assert!(memory.due(&one, at));
        // Kept as the file was seen: not read while it stays so, for the same media.
        let seen = Seen {
            size: one.identity.size,
            modified_us: one.identity.modified_us,
        };
        memory.found(&one, &Found::Kept(seen), at);
        assert!(!memory.due(&one, at));
        // Media whose digest or path is another now (named anew) is read.
        let mut other = one.clone();
        other.identity.digest = "f".repeat(64);
        assert!(memory.due(&other, at));
        let mut moved = one.clone();
        moved.path = dir.path().join("b.wav").to_str().unwrap().into();
        assert!(memory.due(&moved, at));
    }

    #[test]
    fn only_media_that_is_still_as_it_was_found_is_set() {
        let read = Found::Read {
            layout: None,
            streams: vec![],
        };
        let project = Project::sample();
        let one = wanted(&project).remove(0);
        let set = |project: &Project, found: Found| settled(project, vec![(one.clone(), found)]);
        assert_eq!(set(&project, read).len(), 1);
        let again = || Found::Read {
            layout: None,
            streams: vec![],
        };
        // Taken away meanwhile.
        let mut gone = project.clone();
        gone.media.clear();
        gone.timelines.clear();
        assert_eq!(set(&gone, again()), []);
        // Of another file meanwhile.
        let mut other = project.clone();
        other.media[0].identity.digest = "f".repeat(64);
        assert_eq!(set(&other, again()), []);
        // Read meanwhile.
        let mut done = project.clone();
        done.media[0].facts.audio.as_mut().unwrap().streams = Some(vec![]);
        assert_eq!(set(&done, again()), []);
        // Not read.
        assert_eq!(set(&project, Found::Later), []);
    }
}
