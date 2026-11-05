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
use std::time::Duration;

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
                let mut kept = HashMap::new();
                loop {
                    let Some(state) = state.upgrade() else {
                        return;
                    };
                    pass(&state, &mut kept, limits, &stopped);
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

/// One pass: read what is wanted, but what was kept as it was seen and is so still, and set
/// what was read in one change of the engine.
fn pass(
    state: &Mutex<State>,
    kept: &mut HashMap<Id, (String, Seen)>,
    limits: Limits,
    stop: &(std::sync::Mutex<bool>, Condvar),
) {
    let wanted = wanted(state.blocking_lock().history.project());
    kept.retain(|id, _| wanted.iter().any(|one| &one.id == id));
    let mut found = Vec::new();
    for one in wanted {
        if stopped(stop) {
            return;
        }
        if kept
            .get(&one.id)
            .is_some_and(|(path, seen)| *path == one.path && unchanged(path, *seen))
        {
            continue;
        }
        match read(&one) {
            Found::Kept(seen) => {
                log(
                    "media_streams_kept",
                    &json!({"media": one.id, "path": one.path,
                        "message": "the file is not that of the media, or cannot be opened"}),
                );
                kept.insert(one.id.clone(), (one.path.clone(), seen));
            }
            Found::Later => {}
            streams @ Found::Read { .. } => found.push((one, streams)),
        }
    }
    if found.is_empty() {
        return;
    }
    let mut locked = state.blocking_lock();
    // Stopped meanwhile, as the engine ends: nothing more is set.
    if stopped(stop) {
        return;
    }
    let ops = settled(locked.history.project(), found);
    if ops.is_empty() {
        return;
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
