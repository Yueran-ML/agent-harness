//! Decoders borrowed by track readers without holding the pool lock during file I/O.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use columbina_media::{Converter, Hardware, MediaError, NativePicture, VideoDecoder};
use columbina_model::{Identity, Media, Rate, Time};

use crate::transport::{Again, Source};

#[derive(Clone, Debug, PartialEq, Eq)]
struct Input {
    media: String,
    path: PathBuf,
    identity: Identity,
    proxy: bool,
    rate: Rate,
    still: bool,
    duration: Time,
    rotation: i16,
    proxy_version: u64,
}

impl Input {
    fn of(media: &Media, proxy: bool) -> Option<Self> {
        let video = media.facts.video.as_ref()?;
        let path = if proxy {
            &crate::proxy::usable(media)?.path
        } else {
            &media.path
        };
        Some(Self {
            media: media.id.clone(),
            path: PathBuf::from(path),
            identity: media.identity.clone(),
            proxy,
            rate: video.rate,
            still: video.still,
            duration: media.facts.duration,
            rotation: video.rotation,
            proxy_version: 0,
        })
    }

    fn same_key(&self, other: &Self) -> bool {
        self.media == other.media && self.proxy == other.proxy
    }

    fn frame(&self, tick: i64) -> i64 {
        if self.still {
            return 0;
        }
        let tick = tick.clamp(0, (self.duration.ticks - 1).max(0));
        // Source frame n starts at floor(n * scale * den / num), as Rate::ticks_of and
        // span::source_tick do. Invert those integer boundaries: ceil((tick+1)*num/per)-1.
        // Flooring tick*num/per would show frame 0 at the first frame's boundary (614 ticks
        // for 25 fps in scale 15360). This does not round fractional-speed positions up by
        // half a frame: a tick before the boundary still belongs to the preceding frame.
        let over = (i128::from(tick) + 1) * i128::from(self.rate.num);
        let under = i128::from(self.duration.scale) * i128::from(self.rate.den);
        i64::try_from((over - 1) / under).unwrap_or(i64::MAX)
    }
}

struct Reader {
    decoder: VideoDecoder,
    converter: Converter,
    // A single idle picture is enough: track queues own the read-ahead pictures. Reuse only
    // when no queue, compositor or view still holds it. Pictures keep the sampling of chroma of
    // their file, and say how their codes are made R'G'B' (their matrix and range).
    picture: Arc<NativePicture>,
    /// The session of the graphics card that the decoder may use, if it was given one.
    _session: Option<Session>,
}

/// The decoders that may use the graphics card at once, of every pool of the engine together:
/// those of the timeline monitor and those of an export. A card decodes a few streams at once
/// well, and fails or slows every one of them past that; a decoder opened beyond the limit
/// decodes on the processor.
pub struct Sessions {
    most: usize,
    open: std::sync::atomic::AtomicUsize,
}

/// The most decoders of all pools together that use the graphics card at once.
const MAX_HARDWARE: usize = 12;

impl Sessions {
    pub fn new(most: usize) -> Self {
        Self {
            most,
            open: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// The sessions shared by every pool of the engine.
    pub fn shared() -> Arc<Sessions> {
        static SHARED: std::sync::OnceLock<Arc<Sessions>> = std::sync::OnceLock::new();
        Arc::clone(SHARED.get_or_init(|| Arc::new(Sessions::new(MAX_HARDWARE))))
    }

    fn take(self: &Arc<Self>) -> Option<Session> {
        use std::sync::atomic::Ordering;
        self.open
            .try_update(Ordering::AcqRel, Ordering::Acquire, |open| {
                (open < self.most).then_some(open + 1)
            })
            .ok()?;
        Some(Session(Arc::clone(self)))
    }

    #[cfg(test)]
    fn open(&self) -> usize {
        self.open.load(std::sync::atomic::Ordering::Acquire)
    }
}

/// A decoder's leave to use the graphics card, given back when it is closed.
struct Session(Arc<Sessions>);

impl Drop for Session {
    fn drop(&mut self) {
        self.0
            .open
            .fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
    }
}

struct Slot {
    input: Input,
    clip: String,
    reader: Option<Reader>,
    id: u64,
    used: u64,
    busy: bool,
    retired: bool,
}

#[derive(Default)]
struct Pool {
    slots: Vec<Slot>,
    next: u64,
    clock: u64,
    failed: Vec<Input>,
    proxy_versions: Vec<(String, u64)>,
    /// How many tracks read each media at once, as the monitor tells.
    demand: std::collections::BTreeMap<String, usize>,
}

impl Pool {
    /// How many decoders one media may have: two for each track that reads it (a transition
    /// reads two places of a track at once), two at least, and never more than the pool. Three
    /// tracks of one media at one frame would otherwise take turns with two decoders, each
    /// turn opening a decoder and seeking again.
    fn per_key(&self, media: &str) -> usize {
        let tracks = self.demand.get(media).copied().unwrap_or(1);
        (2 * tracks).clamp(MAX_PER_KEY, MAX_DECODERS)
    }

    fn proxy_version(&self, media: &str) -> u64 {
        self.proxy_versions
            .iter()
            .find(|(id, _)| id == media)
            .map_or(0, |(_, version)| *version)
    }
}

#[derive(Clone)]
pub struct Sources {
    shared: Arc<Mutex<Pool>>,
    hardware: Hardware,
    again: Again,
    sessions: Arc<Sessions>,
}

#[derive(Clone)]
pub struct Read {
    pub picture: Arc<NativePicture>,
    pub exact: bool,
    pub source: Source,
    pub rotation: i16,
}

struct Lease {
    sources: Sources,
    id: u64,
    reader: Option<Reader>,
}

impl Drop for Lease {
    fn drop(&mut self) {
        let mut pool = self.sources.lock();
        if let Some(index) = pool.slots.iter().position(|slot| slot.id == self.id) {
            if pool.slots[index].retired {
                pool.slots.remove(index);
            } else {
                let slot = &mut pool.slots[index];
                slot.reader = self.reader.take();
                slot.busy = false;
            }
        }
    }
}

impl Lease {
    fn read(&mut self, input: &Input, tick: i64, scrubbing: bool) -> Result<Read, MediaError> {
        if self.reader.is_none() {
            let session = (self.sources.hardware == Hardware::Auto)
                .then(|| self.sources.sessions.take())
                .flatten();
            let hardware = if session.is_some() {
                Hardware::Auto
            } else {
                Hardware::Off
            };
            let decoder = if input.proxy {
                VideoDecoder::open_at_rate(&input.path, hardware, input.rate)?
            } else {
                VideoDecoder::open(&input.path, hardware)?
            };
            self.reader = Some(Reader {
                decoder,
                converter: Converter::new(),
                picture: Arc::default(),
                _session: session,
            });
        }
        let reader = self.reader.as_mut().expect("opened above");
        let requested = input.frame(tick);
        let frame = if scrubbing {
            reader
                .decoder
                .keyframe_at_or_before(requested)
                .unwrap_or(requested)
        } else {
            requested
        };
        // Avoid a full copy when this buffer is still held by a ready frame or the view.
        if Arc::get_mut(&mut reader.picture).is_none() {
            reader.picture = Arc::default();
        }
        let picture = Arc::get_mut(&mut reader.picture).expect("the reader owns this buffer");
        let decoded = reader.decoder.decoded_at(frame)?;
        match reader.converter.native_into(&decoded, picture) {
            Err(MediaError::Device(why)) => {
                drop(decoded);
                reader.decoder.device_failed(&why);
                let decoded = reader.decoder.decoded_at(frame)?;
                reader.converter.native_into(&decoded, picture)?;
            }
            result => result?,
        }
        Ok(Read {
            picture: Arc::clone(&reader.picture),
            exact: frame == requested,
            source: if input.proxy {
                Source::Proxy
            } else {
                Source::Original
            },
            rotation: input.rotation,
        })
    }
}

const MAX_DECODERS: usize = 12;
const MAX_PER_KEY: usize = 2;

impl Sources {
    pub fn new(hardware: Hardware, again: Again) -> Self {
        Self::with_sessions(hardware, again, Sessions::shared())
    }

    pub fn with_sessions(hardware: Hardware, again: Again, sessions: Arc<Sessions>) -> Self {
        Self {
            shared: Arc::default(),
            hardware,
            again,
            sessions,
        }
    }

    /// Tell how many tracks read each media now, so that each may keep a decoder.
    pub fn demand(&self, demand: &std::collections::BTreeMap<String, usize>) {
        self.lock().demand.clone_from(demand);
    }

    fn lock(&self) -> MutexGuard<'_, Pool> {
        self.shared.lock().unwrap_or_else(PoisonError::into_inner)
    }

    // Reserve before opening. Even readers still opening count towards the limits. Busy
    // slots are never evicted and no decoder or file I/O happens with the pool locked.
    fn borrow(&self, input: &Input, clip: &str) -> Option<Lease> {
        let mut pool = self.lock();
        let same: Vec<_> = pool
            .slots
            .iter()
            .enumerate()
            .filter(|(_, slot)| slot.input.same_key(input))
            .map(|(i, _)| i)
            .collect();
        let matching = same.iter().copied().find(|&i| {
            let slot = &pool.slots[i];
            !slot.busy && !slot.retired && slot.input == *input && slot.clip == clip
        });
        let index = if let Some(index) = matching {
            index
        } else {
            let replace = if same.len() >= pool.per_key(&input.media) {
                same.iter()
                    .copied()
                    .filter(|&i| !pool.slots[i].busy)
                    .min_by_key(|&i| pool.slots[i].used)
            } else if pool.slots.len() == MAX_DECODERS {
                pool.slots
                    .iter()
                    .enumerate()
                    .filter(|(_, slot)| !slot.busy)
                    .min_by_key(|(_, slot)| slot.used)
                    .map(|(i, _)| i)
            } else {
                Some(pool.slots.len())
            }?;
            if replace < pool.slots.len() {
                // Same key and file, different clip: reuse the decoder at this position.
                // A changed path/identity must reopen, even though the key is unchanged.
                let slot = &mut pool.slots[replace];
                if slot.input != *input || slot.retired {
                    slot.reader = None;
                }
                slot.input = input.clone();
                slot.clip = clip.into();
                slot.retired = false;
            } else {
                let id = pool.next;
                pool.next += 1;
                pool.slots.push(Slot {
                    input: input.clone(),
                    clip: clip.into(),
                    reader: None,
                    id,
                    used: 0,
                    busy: false,
                    retired: false,
                });
            }
            replace
        };
        pool.clock += 1;
        let clock = pool.clock;
        let slot = &mut pool.slots[index];
        slot.used = clock;
        slot.busy = true;
        Some(Lease {
            sources: self.clone(),
            id: slot.id,
            reader: slot.reader.take(),
        })
    }

    /// Read the source tick resolved by the model. `None` means all suitable readers are
    /// temporarily borrowed: a track reader may retry, without blocking other tracks.
    pub fn picture(
        &self,
        media: &Media,
        clip: &str,
        tick: i64,
        proxies: bool,
        scrubbing: bool,
    ) -> Result<Option<Read>, MediaError> {
        let mut short_proxy = None;
        if let Some(mut input) = proxies.then(|| Input::of(media, true)).flatten() {
            let failed = {
                let pool = self.lock();
                input.proxy_version = pool.proxy_version(&input.media);
                pool.failed.contains(&input)
            };
            if !failed {
                let Some(mut lease) = self.borrow(&input, clip) else {
                    return Ok(None);
                };
                match lease.read(&input, tick, scrubbing) {
                    Ok(read) => return Ok(Some(read)),
                    Err(MediaError::OutOfRange { .. }) => short_proxy = Some(input),
                    Err(_) => {
                        drop(lease);
                        self.failed_proxy(&input);
                    }
                }
            }
        }
        let input = Input::of(media, false).ok_or(MediaError::NoVideo)?;
        let Some(mut lease) = self.borrow(&input, clip) else {
            return Ok(None);
        };
        let read = lease.read(&input, tick, scrubbing)?;
        drop(lease);
        // An ordinary end of video must not mark a proxy broken (audio may be longer).
        // It is truncated only if the original can actually supply the missing frame.
        if let Some(input) = short_proxy {
            self.failed_proxy(&input);
        }
        Ok(Some(read))
    }

    fn failed_proxy(&self, input: &Input) {
        // Close idle readers before repair. In-flight readers are retired on return. The
        // callback asks for regeneration without deleting by path: another track may still
        // read it, or a new proxy may already be replacing it at that same path.
        let repair = {
            let mut pool = self.lock();
            if pool.proxy_version(&input.media) != input.proxy_version {
                return;
            }
            let first = !pool.failed.contains(input);
            if first {
                pool.failed.push(input.clone());
            }
            for slot in &mut pool.slots {
                if slot.input == *input {
                    slot.retired = true;
                }
            }
            pool.slots.retain(|slot| !slot.retired || slot.busy);
            first
        };
        if repair {
            (self.again)(&input.media, None);
        }
    }

    /// A freshly written proxy at the same path is a new input, not a cached failure or an
    /// already open old file. In-flight work is allowed to finish on its project snapshot.
    pub fn proxy_ready(&self, media: &str) {
        let mut pool = self.lock();
        if let Some((_, version)) = pool.proxy_versions.iter_mut().find(|(id, _)| id == media) {
            *version += 1;
        } else {
            pool.proxy_versions.push((media.into(), 1));
        }
        pool.failed.retain(|input| input.media != media);
        for slot in &mut pool.slots {
            if slot.input.media == media && slot.input.proxy {
                slot.retired = true;
            }
        }
        pool.slots.retain(|slot| !slot.retired || slot.busy);
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::test_media::{media, number_in, Sample};
    use columbina_model::{Identity, Proxy};

    pub(in crate::play) fn sample() -> Option<Media> {
        let made = media(Sample::Fhd)?;
        let facts = columbina_media::probe(&made.path).unwrap();
        Some(Media {
            id: "m1".into(),
            path: made.path.to_string_lossy().into_owned(),
            identity: Identity {
                size: 1,
                modified_us: 0,
                digest: "0".repeat(64),
            },
            facts,
            proxy: None,
            name: "Numbered frames".into(),
            bin: None,
            mark_in: None,
            mark_out: None,
        })
    }

    fn ticks(media: &Media, frame: i64) -> i64 {
        let rate = media.facts.video.as_ref().unwrap().rate;
        i64::try_from(
            i128::from(frame) * i128::from(media.facts.duration.scale) * i128::from(rate.den)
                / i128::from(rate.num),
        )
        .unwrap()
    }

    fn assert_frame(read: &Read, frame: i64) {
        let picture = &read.picture;
        assert_eq!(picture.frame, frame);
        assert_eq!(
            number_in(&picture.luma, picture.luma_stride, picture.bits),
            Some(frame)
        );
    }

    #[test]
    fn source_frame_boundaries_use_the_models_quantized_ticks() {
        let Some(mut media) = sample() else { return };
        for (rate, scale) in [
            (Rate { num: 25, den: 1 }, 15360),
            (
                Rate {
                    num: 30000,
                    den: 1001,
                },
                1000,
            ),
            (
                Rate {
                    num: 24000,
                    den: 1001,
                },
                90000,
            ),
        ] {
            media.facts.video.as_mut().unwrap().rate = rate;
            media.facts.duration = Time {
                ticks: i64::from(scale) * 10,
                scale,
            };
            let input = Input::of(&media, false).unwrap();
            for frame in 1..200 {
                let boundary = rate.ticks_of(frame, scale);
                assert_eq!(
                    input.frame(boundary),
                    frame,
                    "{rate:?}, {scale}, {boundary}"
                );
                assert_eq!(input.frame(boundary - 1), frame - 1);
            }
        }
    }

    #[test]
    fn all_twelve_borrowed_readers_block_a_thirteenth_and_a_failed_open_returns_its_slot() {
        let Some(mut media) = sample() else { return };
        let sources = Sources::new(Hardware::Off, Arc::new(|_, _| {}));
        let mut held = Vec::new();
        for id in 0..12 {
            media.id = format!("m{id}");
            held.push(
                sources
                    .borrow(&Input::of(&media, false).unwrap(), "clip")
                    .unwrap(),
            );
        }
        media.id = "extra".into();
        let input = Input::of(&media, false).unwrap();
        assert!(sources.borrow(&input, "clip").is_none());
        held.pop();
        assert!(sources.borrow(&input, "clip").is_some());
        drop(held);
        media.path = "no-such-timeline-source.mp4".into();
        assert!(sources.picture(&media, "bad", 0, false, false).is_err());
        assert!(sources.lock().slots.iter().all(|slot| !slot.busy));
    }

    #[test]
    fn pictures_held_by_other_frames_are_never_overwritten() {
        let Some(media) = sample() else { return };
        let sources = Sources::new(Hardware::Off, Arc::new(|_, _| {}));
        let first = sources
            .picture(&media, "clip", 0, false, false)
            .unwrap()
            .unwrap();
        for frame in 1..5 {
            let read = sources
                .picture(&media, "clip", ticks(&media, frame), false, false)
                .unwrap()
                .unwrap();
            assert_frame(&read, frame);
            assert_frame(&first, 0);
        }
    }

    #[test]
    fn a_source_is_read_at_its_own_rate_including_backward_and_repeated_frames() {
        let Some(media) = sample() else { return };
        let sources = Sources::new(Hardware::Off, Arc::new(|_, _| {}));
        for frame in [0, 1, 17, 17, 73, 72, 71, 0, 99] {
            let read = sources
                .picture(&media, "clip", ticks(&media, frame), false, false)
                .unwrap()
                .unwrap();
            assert_frame(&read, frame);
            assert!(read.exact);
            assert_eq!(read.source, Source::Original);
        }
    }

    #[test]
    fn two_positions_of_one_source_keep_separate_decoders_for_a_transition() {
        let Some(media) = sample() else { return };
        let sources = Sources::new(Hardware::Off, Arc::new(|_, _| {}));
        for offset in 0..4 {
            for (clip, frame) in [("left", 70 + offset), ("right", 10 + offset)] {
                assert_frame(
                    &sources
                        .picture(&media, clip, ticks(&media, frame), false, false)
                        .unwrap()
                        .unwrap(),
                    frame,
                );
            }
        }
        let pool = sources.lock();
        assert_eq!(pool.slots.len(), 2);
        assert!(pool
            .slots
            .iter()
            .all(|slot| slot.reader.as_ref().unwrap().decoder.seeks() <= 1));
    }

    #[test]
    fn three_tracks_of_one_media_keep_a_decoder_each_instead_of_taking_turns() {
        let Some(media) = sample() else { return };
        let sources = Sources::new(Hardware::Off, Arc::new(|_, _| {}));
        sources.demand(&[("m1".to_owned(), 3)].into_iter().collect());
        for offset in 0..6 {
            for (clip, frame) in [
                ("v1", 10 + offset),
                ("v2", 40 + offset),
                ("v3", 70 + offset),
            ] {
                assert_frame(
                    &sources
                        .picture(&media, clip, ticks(&media, frame), false, false)
                        .unwrap()
                        .unwrap(),
                    frame,
                );
            }
        }
        let pool = sources.lock();
        assert_eq!(pool.slots.len(), 3);
        // Each decoder sought once, to its clip; none was closed and opened again.
        assert!(pool
            .slots
            .iter()
            .all(|slot| slot.reader.as_ref().unwrap().decoder.seeks() <= 1));
    }

    #[test]
    fn decoders_past_the_cards_sessions_decode_on_the_processor() {
        let Some(mut media) = sample() else { return };
        let sessions = Arc::new(Sessions::new(1));
        let sources =
            Sources::with_sessions(Hardware::Auto, Arc::new(|_, _| {}), Arc::clone(&sessions));
        // An export's pool shares the sessions of the monitor's.
        let export =
            Sources::with_sessions(Hardware::Auto, Arc::new(|_, _| {}), Arc::clone(&sessions));
        let first = sources
            .picture(&media, "a", 0, false, false)
            .unwrap()
            .unwrap();
        assert_frame(&first, 0);
        assert_eq!(sessions.open(), 1);
        media.id = "m2".into();
        let second = export.picture(&media, "b", ticks(&media, 3), false, false);
        assert_frame(&second.unwrap().unwrap(), 3);
        assert_eq!(sessions.open(), 1);
        let pool = export.lock();
        assert_eq!(
            pool.slots[0].reader.as_ref().unwrap().decoder.path_used(),
            columbina_media::HardwarePath::Software
        );
        drop(pool);
        drop(sources);
        assert_eq!(
            sessions.open(),
            0,
            "a closed decoder gives its session back"
        );
    }

    #[test]
    fn borrowing_is_bounded_and_an_idle_decoder_is_evicted_in_lru_order() {
        let Some(mut media) = sample() else { return };
        let sources = Sources::new(Hardware::Off, Arc::new(|_, _| {}));
        let original = Input::of(&media, false).unwrap();
        let first = sources.borrow(&original, "first").unwrap();
        let second = sources.borrow(&original, "second").unwrap();
        assert!(sources.borrow(&original, "third").is_none());
        drop((first, second));
        for id in 2..=11 {
            media.id = format!("m{id}");
            drop(
                sources
                    .borrow(&Input::of(&media, false).unwrap(), "clip")
                    .unwrap(),
            );
        }
        assert_eq!(sources.lock().slots.len(), 12);
        // Touch the older of m1's two readers. The untouched second reader is evicted.
        drop(sources.borrow(&original, "first").unwrap());
        media.id = "m12".into();
        drop(
            sources
                .borrow(&Input::of(&media, false).unwrap(), "clip")
                .unwrap(),
        );
        let pool = sources.lock();
        assert_eq!(pool.slots.len(), 12);
        let kept: Vec<_> = pool
            .slots
            .iter()
            .filter(|slot| slot.input.media == "m1")
            .collect();
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].clip, "first");
    }

    #[test]
    fn scrubbing_reads_a_key_frame_and_an_exact_read_then_settles() {
        let Some(media) = sample() else { return };
        let sources = Sources::new(Hardware::Off, Arc::new(|_, _| {}));
        let tick = ticks(&media, 73);
        let quick = sources
            .picture(&media, "clip", tick, false, true)
            .unwrap()
            .unwrap();
        assert_frame(&quick, 50);
        assert!(!quick.exact);
        let exact = sources
            .picture(&media, "clip", tick, false, false)
            .unwrap()
            .unwrap();
        assert_frame(&exact, 73);
        assert!(exact.exact);
    }

    #[test]
    fn a_broken_proxy_falls_back_once_and_a_replaced_proxy_is_tried_again() {
        let Some(mut media) = sample() else { return };
        let dir = tempfile::tempdir().unwrap();
        let proxy = dir.path().join("proxy.mp4");
        std::fs::write(&proxy, b"broken proxy").unwrap();
        media.proxy = Some(Proxy {
            path: proxy.to_string_lossy().into_owned(),
            width: 1920,
            height: 1080,
            codec: "h264".into(),
            digest_of_source: media.identity.digest.clone(),
        });
        let failures = Arc::new(Mutex::new(Vec::new()));
        let sources = Sources::new(Hardware::Off, {
            let failures = Arc::clone(&failures);
            Arc::new(move |media, _| failures.lock().unwrap().push(media.to_owned()))
        });
        for frame in [1, 2] {
            let read = sources
                .picture(&media, "clip", ticks(&media, frame), true, false)
                .unwrap()
                .unwrap();
            assert_frame(&read, frame);
            assert_eq!(read.source, Source::Original);
        }
        assert_eq!(*failures.lock().unwrap(), ["m1"]);
        std::fs::copy(&media.path, &proxy).unwrap();
        sources.proxy_ready("m1");
        let read = sources
            .picture(&media, "clip", ticks(&media, 3), true, false)
            .unwrap()
            .unwrap();
        assert_frame(&read, 3);
        assert_eq!(read.source, Source::Proxy);
    }

    #[test]
    fn a_truncated_proxy_falls_back_if_the_original_still_has_the_frame() {
        let Some(mut media) = sample() else { return };
        let Some(shorter) = crate::test_media::media(Sample::Prores) else {
            return;
        };
        media.proxy = Some(Proxy {
            path: shorter.path.to_string_lossy().into_owned(),
            width: 1920,
            height: 1080,
            codec: "prores".into(),
            digest_of_source: media.identity.digest.clone(),
        });
        let sources = Sources::new(Hardware::Off, Arc::new(|_, _| {}));
        let read = sources
            .picture(&media, "clip", ticks(&media, 73), true, false)
            .unwrap()
            .unwrap();
        assert_frame(&read, 73);
        assert_eq!(read.source, Source::Original);
    }

    #[test]
    fn an_old_failed_read_cannot_invalidate_a_replacement_at_the_same_proxy_path() {
        let Some(mut media) = sample() else { return };
        media.proxy = Some(Proxy {
            path: media.path.clone(),
            width: 1920,
            height: 1080,
            codec: "h264".into(),
            digest_of_source: media.identity.digest.clone(),
        });
        let sources = Sources::new(Hardware::Off, Arc::new(|_, _| {}));
        let old = Input::of(&media, true).unwrap();
        let held = sources.borrow(&old, "old read").unwrap();
        sources.proxy_ready("m1");
        // The old decoder reports a failure after the replacement finished writing.
        sources.failed_proxy(&old);
        drop(held);
        let read = sources
            .picture(&media, "new read", 0, true, false)
            .unwrap()
            .unwrap();
        assert_eq!(read.source, Source::Proxy);
        assert_eq!(sources.lock().failed, []);
    }
}
