//! One read-ahead worker per video track. Requests replace older requests; stale work is
//! never published into a newer project generation.

use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use columbina_model::project::TrackKind;
use columbina_model::resolve::{self, Content};
use columbina_model::Project;

use super::sources::{Read, Sources};

#[derive(Clone)]
pub struct Request {
    pub project: Arc<Project>,
    pub timeline: usize,
    pub track: usize,
    pub frame: i64,
    pub speed: i32,
    pub proxies: bool,
    pub scrubbing: bool,
    /// Changes on a project edit, load or proxy replacement, not on each playhead movement.
    pub generation: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Want {
    pub clip: String,
    pub media: String,
    pub tick: i64,
    pub frame: i64,
}

fn plan(request: &Request) -> Vec<Want> {
    let Some(timeline) = request.project.timelines.get(request.timeline) else {
        return Vec::new();
    };
    let Some(track) = timeline.tracks.get(request.track) else {
        return Vec::new();
    };
    if !track.enabled || track.kind != TrackKind::Video {
        return Vec::new();
    }
    let speed = i64::from(request.speed);
    let direction = if speed < 0 { -1 } else { 1 };
    let reach = i64::from(timeline.rate.num.div_ceil(timeline.rate.den)) * speed.abs().max(1);
    let ahead = |frame: i64| {
        frame >= 0
            && if direction > 0 {
                frame >= request.frame && frame <= request.frame.saturating_add(reach)
            } else {
                frame <= request.frame && frame >= request.frame.saturating_sub(reach)
            }
    };
    let mut frames = vec![request.frame];
    // Decode a few imminent frames, and the first two at every cut/transition in the next
    // second. A long clip does not fill memory with a whole second of uncompressed 4K.
    if !request.scrubbing {
        for step in 1..4 {
            frames.push(request.frame.saturating_add(speed * step));
        }
        for clip in track.clips.iter().filter(|clip| clip.enabled) {
            let first = if direction > 0 {
                clip.start
            } else {
                clip.end() - 1
            };
            if ahead(first) {
                frames.extend([first, first.saturating_add(direction)]);
            }
        }
        for transition in &track.transitions {
            if let Some(right) = track.clips.iter().find(|clip| clip.id == transition.right) {
                let (after, before) = transition.handles();
                let first = if direction > 0 {
                    right.start - before
                } else {
                    right.start + after - 1
                };
                if ahead(first) {
                    frames.extend([first, first.saturating_add(direction)]);
                }
            }
        }
    }
    frames.retain(|&frame| ahead(frame));
    frames.sort_unstable_by_key(|frame| frame.abs_diff(request.frame));
    frames.dedup();
    let mut wanted: Vec<Want> = Vec::new();
    for frame in frames {
        for layer in resolve::video_of_track_at(timeline, &request.project, track, frame) {
            if let Content::Media { media, tick } = layer.content {
                let want = Want {
                    clip: layer.clip,
                    media,
                    tick,
                    frame,
                };
                if !wanted.iter().any(|old| old.same_source(&want)) {
                    wanted.push(want);
                }
            }
        }
    }
    wanted
}

impl Want {
    fn same_source(&self, other: &Self) -> bool {
        self.clip == other.clip && self.media == other.media && self.tick == other.tick
    }
}

struct Cached {
    want: Want,
    result: Result<Read, String>,
}

#[derive(Default)]
struct State {
    request: Option<Request>,
    serial: u64,
    ready: Vec<Cached>,
    stopped: bool,
}

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
    available: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

pub struct TrackReader {
    shared: Arc<Shared>,
}

impl Drop for TrackReader {
    fn drop(&mut self) {
        self.shared.lock().stopped = true;
        self.shared.changed.notify_all();
        self.shared.available.notify_all();
    }
}

// A worker that ends unexpectedly wakes a synchronous seek rather than leaving it waiting.
struct WorkerGuard(Arc<Shared>);

impl Drop for WorkerGuard {
    fn drop(&mut self) {
        self.0.lock().stopped = true;
        self.0.available.notify_all();
    }
}

fn cached(
    state: &State,
    generation: u64,
    want: &Want,
    exact: bool,
) -> Option<Result<Read, String>> {
    if state.request.as_ref()?.generation != generation {
        return None;
    }
    state
        .ready
        .iter()
        .find(|cached| {
            cached.want.same_source(want)
                && (!exact || cached.result.as_ref().map_or(true, |read| read.exact))
        })
        .map(|cached| cached.result.clone())
}

impl TrackReader {
    pub fn new(sources: Sources, budget: usize) -> std::io::Result<Self> {
        let shared = Arc::new(Shared::default());
        std::thread::Builder::new()
            .name("columbina-track-read".into())
            .spawn({
                let shared = Arc::clone(&shared);
                move || {
                    let _guard = WorkerGuard(Arc::clone(&shared));
                    run(&shared, &sources, budget);
                }
            })?;
        Ok(Self { shared })
    }

    pub fn request(&self, request: Request) {
        let mut state = self.shared.lock();
        if state.request.as_ref().is_none_or(|old| {
            old.generation != request.generation || old.proxies != request.proxies
        }) {
            state.ready.clear();
        }
        state.request = Some(request);
        state.serial += 1;
        drop(state);
        self.shared.changed.notify_one();
        self.shared.available.notify_all();
    }

    pub fn ready(&self, generation: u64, want: &Want, exact: bool) -> Option<Result<Read, String>> {
        cached(&self.shared.lock(), generation, want, exact)
    }

    pub fn wait(
        &self,
        generation: u64,
        want: &Want,
        exact: bool,
        timeout: Duration,
    ) -> Option<Result<Read, String>> {
        let until = Instant::now() + timeout;
        let mut state = self.shared.lock();
        loop {
            if let Some(result) = cached(&state, generation, want, exact) {
                return Some(result);
            }
            if state.stopped || state.request.as_ref()?.generation != generation {
                return None;
            }
            let remaining = until.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return None;
            }
            state = self
                .shared
                .available
                .wait_timeout(state, remaining)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
}

fn run(shared: &Shared, sources: &Sources, budget: usize) {
    let mut done = 0;
    loop {
        let (request, serial) = {
            let mut state = shared.lock();
            while !state.stopped && state.serial == done {
                state = shared
                    .changed
                    .wait(state)
                    .unwrap_or_else(PoisonError::into_inner);
            }
            if state.stopped {
                return;
            }
            let Some(request) = state.request.clone() else {
                continue;
            };
            (request, state.serial)
        };
        let wanted = plan(&request);
        {
            let mut state = shared.lock();
            if state.serial != serial {
                continue;
            }
            state.ready.retain_mut(|cached| {
                if let Some(want) = wanted.iter().find(|want| want.same_source(&cached.want)) {
                    cached.want = want.clone();
                    true
                } else {
                    false
                }
            });
        }
        let mut busy = false;
        for want in wanted {
            {
                let state = shared.lock();
                if state.stopped {
                    return;
                }
                if state.serial != serial {
                    break;
                }
                if cached(&state, request.generation, &want, !request.scrubbing).is_some() {
                    continue;
                }
            }
            let result = request
                .project
                .media
                .iter()
                .find(|media| media.id == want.media)
                .ok_or_else(|| format!("media {} is no longer in the project", want.media))
                .and_then(|media| {
                    sources
                        .picture(
                            media,
                            &want.clip,
                            want.tick,
                            request.proxies,
                            request.scrubbing,
                        )
                        .map_err(|error| error.to_string())
                });
            let result = match result {
                Ok(Some(read)) => Ok(read),
                Ok(None) => {
                    busy = true;
                    continue;
                }
                Err(error) => Err(error),
            };
            let mut state = shared.lock();
            if state.stopped {
                return;
            }
            // A later playhead on the same project can reuse this frame. A different project
            // or proxy preference cannot; in particular an undo must not publish old pictures.
            if state.request.as_ref().is_some_and(|latest| {
                latest.generation == request.generation && latest.proxies == request.proxies
            }) {
                state.ready.retain(|cached| !cached.want.same_source(&want));
                state.ready.push(Cached { want, result });
                trim(&mut state, budget);
                shared.available.notify_all();
            }
        }
        if busy {
            let state = shared.lock();
            if state.serial == serial && !state.stopped {
                // There are at most twelve leases. Let the other track readers return theirs
                // before retrying, and wake immediately for a newer seek or stop.
                drop(
                    shared
                        .changed
                        .wait_timeout(state, Duration::from_millis(2))
                        .unwrap_or_else(PoisonError::into_inner),
                );
            }
        } else {
            done = serial;
        }
    }
}

fn trim(state: &mut State, budget: usize) {
    let frame = state.request.as_ref().map_or(0, |request| request.frame);
    let bytes = |cached: &Cached| {
        cached
            .result
            .as_ref()
            .map_or(0, |read| read.picture.bytes())
    };
    let mut total: usize = state.ready.iter().map(bytes).sum();
    // Keep at least the two layers of the current dissolve, even when one frame alone is
    // larger than the per-track budget. Extra future frames are discarded furthest first.
    while (total > budget && state.ready.len() > 2) || state.ready.len() > 32 {
        let Some((furthest, _)) = state
            .ready
            .iter()
            .enumerate()
            .max_by_key(|(_, cached)| cached.want.frame.abs_diff(frame))
        else {
            break;
        };
        total -= bytes(&state.ready.remove(furthest));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::play::sources::tests::sample;
    use crate::test_media::number_in;
    use columbina_media::Hardware;
    use columbina_model::project::{
        Alignment, Clip, Timeline, Track, TrackKind, Transition, TransitionKind,
    };
    use columbina_model::{Rate, Time};

    fn project() -> Option<Arc<Project>> {
        let media = sample()?;
        let scale = media.facts.duration.scale;
        let rate = Rate { num: 25, den: 1 };
        let clip = |id, start, length, source| {
            Clip::of_media(
                id,
                "m1",
                Time {
                    ticks: rate.ticks_of(source, scale),
                    scale,
                },
                start,
                length,
            )
        };
        Some(Arc::new(Project {
            schema: columbina_model::SCHEMA,
            next_id: 20,
            bins: Vec::new(),
            media: vec![media],
            timelines: vec![Timeline {
                id: "t1".into(),
                name: "Timeline".into(),
                width: 1920,
                height: 1080,
                rate,
                sample_rate: 48000,
                tracks: vec![Track {
                    id: "V1".into(),
                    name: "Picture".into(),
                    kind: TrackKind::Video,
                    enabled: true,
                    locked: false,
                    language: String::new(),
                    cues: Vec::new(),
                    clips: vec![
                        clip("c1", 0, 25, 0),
                        clip("c2", 25, 25, 60),
                        clip("c3", 100, 20, 0),
                    ],
                    transitions: vec![Transition {
                        id: "x1".into(),
                        left: "c1".into(),
                        right: "c2".into(),
                        length: 4,
                        kind: TransitionKind::Dissolve,
                        align: Alignment::Center,
                    }],
                }],
                grade: columbina_model::Grade::default(),
            }],
            groups: Vec::new(),
        }))
    }

    fn request(project: Arc<Project>, frame: i64) -> Request {
        Request {
            project,
            timeline: 0,
            track: 0,
            frame,
            speed: 1,
            proxies: false,
            scrubbing: false,
            generation: 1,
        }
    }

    #[test]
    fn prefetch_crosses_the_next_cut_and_includes_both_transition_handles() {
        let Some(project) = project() else { return };
        let request = request(project, 10);
        let wanted = plan(&request);
        assert_eq!(wanted[0].frame, 10);
        for frame in [23, 24] {
            let clips: Vec<_> = wanted
                .iter()
                .filter(|want| want.frame == frame)
                .map(|want| want.clip.as_str())
                .collect();
            assert!(clips.contains(&"c1") && clips.contains(&"c2"), "{wanted:?}");
        }
        assert!(
            !wanted.iter().any(|want| want.clip == "c3"),
            "not within the next second"
        );
    }

    #[test]
    fn reverse_prefetch_is_prepared_before_the_cut_and_disabled_tracks_do_no_work() {
        let Some(project) = project() else { return };
        let mut request = request(project, 35);
        request.speed = -1;
        let wanted = plan(&request);
        assert_eq!(wanted[0].frame, 35);
        assert!(
            wanted
                .iter()
                .any(|want| want.clip == "c1" && want.frame == 26),
            "{wanted:?}"
        );
        Arc::make_mut(&mut request.project).timelines[0].tracks[0].enabled = false;
        assert_eq!(plan(&request), []);
    }

    #[test]
    fn a_track_reads_upcoming_frames_and_replaces_old_project_work() {
        let Some(project) = project() else { return };
        let sources = Sources::new(Hardware::Off, Arc::new(|_, _| {}));
        let reader = TrackReader::new(sources, 64 * 1024 * 1024).unwrap();
        let mut request = request(project, 10);
        let wanted = plan(&request);
        reader.request(request.clone());
        for want in &wanted {
            let read = reader
                .wait(1, want, true, Duration::from_secs(5))
                .expect("prefetched")
                .unwrap();
            let expected = if want.clip == "c1" {
                want.frame
            } else {
                60 + want.frame - 25
            };
            assert_eq!(
                number_in(
                    &read.picture.luma,
                    read.picture.luma_stride,
                    read.picture.bits
                ),
                Some(expected)
            );
        }
        request.frame = 74; // A gap; no stale frame from c1/c2 should appear here.
        request.generation = 2;
        reader.request(request);
        assert!(reader.ready(1, &wanted[0], true).is_none());
        assert!(reader.ready(2, &wanted[0], true).is_none());
    }

    #[test]
    fn an_edit_while_a_source_is_opening_does_not_publish_the_old_generation() {
        let Some(mut project) = project() else { return };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("broken-proxy.mp4");
        std::fs::write(&path, b"broken").unwrap();
        let media = &mut Arc::make_mut(&mut project).media[0];
        media.proxy = Some(columbina_model::Proxy {
            path: path.to_string_lossy().into_owned(),
            width: 1920,
            height: 1080,
            codec: "h264".into(),
            digest_of_source: media.identity.digest.clone(),
        });
        let (entered, waiting) = std::sync::mpsc::channel();
        let (resume, resumed) = std::sync::mpsc::channel();
        let resumed = Mutex::new(resumed);
        let sources = Sources::new(
            Hardware::Off,
            Arc::new(move |_, _| {
                entered.send(()).unwrap();
                resumed
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap();
            }),
        );
        let reader = TrackReader::new(sources, 64 * 1024 * 1024).unwrap();
        let mut old = request(project, 10);
        old.proxies = true;
        let old_want = plan(&old).remove(0);
        reader.request(old.clone());
        waiting.recv_timeout(Duration::from_secs(5)).unwrap();
        let mut changed = old;
        changed.generation = 2;
        changed.proxies = false;
        let timeline = &mut Arc::make_mut(&mut changed.project).timelines[0];
        if let columbina_model::project::Source::Media { source_in, .. } =
            &mut timeline.tracks[0].clips[0].source
        {
            source_in.ticks = timeline.rate.ticks_of(20, source_in.scale);
        }
        let new_want = plan(&changed).remove(0);
        reader.request(changed);
        resume.send(()).unwrap();
        let read = reader
            .wait(2, &new_want, true, Duration::from_secs(5))
            .unwrap()
            .unwrap();
        assert_eq!(
            number_in(
                &read.picture.luma,
                read.picture.luma_stride,
                read.picture.bits
            ),
            Some(30)
        );
        assert!(reader.ready(2, &old_want, true).is_none());
    }

    #[test]
    fn a_read_of_an_older_generation_is_not_published_into_a_newer_one() {
        // Only the generation changes: the same clips, frames and proxy preference, as when
        // the engine makes a proxy of the media. A frame read for the old generation (from the
        // old proxy) must not be taken for the new one.
        let Some(mut project) = project() else { return };
        let dir = tempfile::tempdir().unwrap();
        let proxy = |name: &str| {
            let path = dir.path().join(name);
            std::fs::write(&path, b"broken").unwrap();
            columbina_model::Proxy {
                path: path.to_string_lossy().into_owned(),
                width: 1920,
                height: 1080,
                codec: "h264".into(),
                digest_of_source: "0".repeat(64),
            }
        };
        Arc::make_mut(&mut project).media[0].proxy = Some(proxy("old-proxy.mp4"));
        let (entered, waiting) = std::sync::mpsc::channel();
        let (resume, resumed) = std::sync::mpsc::channel::<()>();
        let resumed = Mutex::new(resumed);
        // Each broken proxy, the first time it fails, holds its reader until resumed.
        let sources = Sources::new(
            Hardware::Off,
            Arc::new(move |_, _| {
                let _ = entered.send(());
                let _ = resumed.lock().unwrap().recv_timeout(Duration::from_secs(5));
            }),
        );
        let reader = TrackReader::new(sources, 64 * 1024 * 1024).unwrap();
        let mut old = request(project, 10);
        old.proxies = true;
        let want = plan(&old).remove(0);
        reader.request(old.clone());
        waiting.recv_timeout(Duration::from_secs(5)).unwrap();
        let mut newer = old;
        newer.generation = 2;
        Arc::make_mut(&mut newer.project).media[0].proxy = Some(proxy("new-proxy.mp4"));
        assert_eq!(plan(&newer)[0], want);
        reader.request(newer);
        resume.send(()).unwrap();
        // The new generation's first read now waits on its own broken proxy: nothing has been
        // read for it yet, and the old read must not stand in for it.
        waiting.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(reader.ready(2, &want, false).is_none());
        resume.send(()).unwrap();
        let read = reader
            .wait(2, &want, true, Duration::from_secs(5))
            .unwrap()
            .unwrap();
        assert_eq!(
            number_in(
                &read.picture.luma,
                read.picture.luma_stride,
                read.picture.bits
            ),
            Some(10)
        );
    }

    #[test]
    fn a_small_budget_keeps_both_current_transition_layers_and_the_worker_stops_on_drop() {
        let Some(project) = project() else { return };
        let sources = Sources::new(Hardware::Off, Arc::new(|_, _| {}));
        let reader = TrackReader::new(sources, 1).unwrap();
        let request = request(project, 24);
        let wanted = plan(&request);
        let current: Vec<_> = wanted.iter().filter(|want| want.frame == 24).collect();
        assert_eq!(current.len(), 2);
        reader.request(request);
        for want in &current {
            reader
                .wait(1, want, true, Duration::from_secs(5))
                .unwrap()
                .unwrap();
        }
        // Let the worker read and discard the larger future frames, then check the current pair.
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(reader.shared.lock().ready.len(), 2);
        assert!(current
            .iter()
            .all(|want| reader.ready(1, want, true).is_some()));
        let shared = Arc::downgrade(&reader.shared);
        drop(reader);
        let until = Instant::now() + Duration::from_secs(5);
        while shared.upgrade().is_some() {
            assert!(
                Instant::now() < until,
                "track reader still alive after drop"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
