//! The sound of a timeline for the transport and the export: the graph of `columbina-audio`
//! (plan T4b), built from the project and fed by the media readers of its clips ([`Feed`]). The
//! mix stays in 32-bit floats to the device; only the copy for the device is clipped.

mod reader;

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::sync::Arc;

use columbina_audio::feed::{ClipFeed, ClipSource, FeedError};
use columbina_audio::graph::{route, Graph, Options, Purpose};
use columbina_audio::node::Layout as Channels;
use columbina_audio::stream::Resampler;
use columbina_media::AudioFormat;
use columbina_model::{resolve, Project};
use serde_json::json;

use crate::log::log;
use reader::{Layout, Reader};

type Stereo = [f32; 2];

/// The most samples that the graph of a mixer processes at once.
const MAX_BLOCK: usize = 4096;

/// The readers of the clips of a graph, opened as the graph first asks for each: the feed of a
/// mixer, and the reading ahead and the starting of a graph that plays (`sound.rs`).
pub(crate) struct Feed {
    project: Arc<Project>,
    timeline: usize,
    sources: Vec<ClipSource>,
    /// The reader of each clip, the read that last used it, and what it reads.
    readers: BTreeMap<usize, (Reader, u64, ReaderKey)>,
    /// The clips read in the block under way.
    read: BTreeSet<usize>,
    /// What went wrong first, since it was last taken.
    error: Option<String>,
    /// The clips that could not be read and are silent, told of once each.
    silenced: BTreeSet<usize>,
    /// Whether a clip that is heard and cannot be read is silent too, told of once, as one that
    /// is not heard is (playback, where the mix goes on), rather than an error (an export).
    lenient: bool,
    /// The reads so far.
    reads: u64,
    /// Where readers let go of are kept for other feeds, and taken from.
    pool: Option<Arc<Readers>>,
}

/// What a reader reads: the media (its id and its file), at the rate of the timeline, in the
/// layout of a clip.
type ReaderKey = (String, String, u32, Layout);

/// The most readers kept for other feeds.
const KEPT_READERS: usize = 48;

/// Readers let go of by a feed, kept for the next feeds of graphs of the same timeline (T5's
/// review: a graph swapped in as the sound plays opens no decoder that one before it had open; a
/// reader moved elsewhere seeks, one that reads on from where it was does not).
#[derive(Default)]
pub(crate) struct Readers {
    kept: std::sync::Mutex<Vec<(ReaderKey, Reader)>>,
}

impl Readers {
    fn take(&self, key: &ReaderKey) -> Option<Reader> {
        let mut kept = self
            .kept
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let at = kept.iter().rposition(|(of, _)| of == key)?;
        Some(kept.remove(at).1)
    }

    fn keep(&self, key: ReaderKey, reader: Reader) {
        let mut kept = self
            .kept
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if kept.len() >= KEPT_READERS {
            kept.remove(0);
        }
        kept.push((key, reader));
    }
}

/// The readers a lenient feed keeps at the most of clips that it has not read lately.
const IDLE_READERS: usize = 8;

/// How many reads ago a reader was last used for it to be idle.
const IDLE_READS: u64 = 256;

impl Feed {
    /// The feed of the clips `sources` of timeline `timeline` of `project`; `lenient` as
    /// [`Feed::lenient`] says.
    pub(crate) fn new(
        project: Arc<Project>,
        timeline: usize,
        sources: &[ClipSource],
        lenient: bool,
    ) -> Self {
        Feed {
            project,
            timeline,
            sources: sources.to_vec(),
            readers: BTreeMap::new(),
            read: BTreeSet::new(),
            error: None,
            silenced: BTreeSet::new(),
            lenient,
            reads: 0,
            pool: None,
        }
    }

    /// The same, taking its readers from `pool` where it can, and keeping them there when it
    /// lets them go.
    pub(crate) fn pooled(mut self, pool: &Arc<Readers>) -> Self {
        self.pool = Some(Arc::clone(pool));
        self
    }

    fn fill(
        &mut self,
        clip: usize,
        range: Range<i64>,
        out: &mut [&mut [f32]],
    ) -> Result<(), String> {
        self.reads += 1;
        if !self.readers.contains_key(&clip) {
            let source = &self.sources[clip];
            let timeline = &self.project.timelines[self.timeline];
            let model = timeline
                .tracks
                .iter()
                .find(|track| track.id == source.track)
                .and_then(|track| track.clips.iter().find(|c| c.id == source.clip))
                .ok_or("the resolved audio clip is missing")?;
            let media = self
                .project
                .media
                .iter()
                .find(|media| media.id == source.media)
                .ok_or("the resolved audio media is missing")?;
            let channels =
                u16::try_from(source.channels).map_err(|_| "too many channels of sound")?;
            let layout = Layout::of_clip(model, channels);
            let key = (
                media.id.clone(),
                media.path.clone(),
                timeline.sample_rate,
                layout.clone(),
            );
            let kept = self.pool.as_ref().and_then(|pool| pool.take(&key));
            let reader = match kept {
                Some(reader) => reader,
                None => Reader::new(media, timeline.sample_rate, &layout)?,
            };
            self.readers.insert(clip, (reader, self.reads, key));
        }
        let reads = self.reads;
        let (reader, used, _) = self.readers.get_mut(&clip).expect("opened above");
        *used = reads;
        let frames = usize::try_from(range.end - range.start).map_err(|_| "a backward read")?;
        let pcm = reader.read(range)?;
        let channels = out.len();
        if pcm.len() != frames * channels {
            return Err("the audio reader returned the wrong number of samples".into());
        }
        for (n, frame) in pcm.chunks_exact(channels).enumerate() {
            for (out, &sample) in out.iter_mut().zip(frame) {
                out[n] = sample;
            }
        }
        Ok(())
    }

    /// Let the readers go that a lenient feed has not used lately, where it has many.
    fn prune(&mut self) {
        if self.lenient && self.readers.len() > IDLE_READERS {
            let reads = self.reads;
            let idle: Vec<usize> = self
                .readers
                .iter()
                .filter(|(_, (_, used, _))| reads.saturating_sub(*used) >= IDLE_READS)
                .map(|(clip, _)| *clip)
                .collect();
            for clip in idle {
                if let Some((reader, _, key)) = self.readers.remove(&clip) {
                    self.give_up(key, reader);
                }
            }
        }
    }

    /// What went wrong first since it was last taken, as a sentence; a sentence of its own where
    /// nothing was kept.
    pub(crate) fn take_error(&mut self) -> String {
        self.error
            .take()
            .unwrap_or_else(|| "the sound of a clip could not be read".into())
    }

    /// Lets `reader` go: into the pool, if there is one.
    fn give_up(&self, key: ReaderKey, reader: Reader) {
        if let Some(pool) = &self.pool {
            pool.keep(key, reader);
        }
    }
}

impl Drop for Feed {
    fn drop(&mut self) {
        for (_, (reader, _, key)) in std::mem::take(&mut self.readers) {
            self.give_up(key, reader);
        }
    }
}

impl ClipFeed for Feed {
    fn read(
        &mut self,
        clip: usize,
        range: Range<i64>,
        out: &mut [&mut [f32]],
    ) -> Result<(), FeedError> {
        self.read.insert(clip);
        let filled = self.fill(clip, range, out);
        self.prune();
        match filled {
            Ok(()) => Ok(()),
            // A clip that is not heard (a muted track, which a track that was off before schema
            // 4 is) plays silence when its media cannot be read, as it did when it was not read;
            // in playback, a clip that is heard does too, so that the rest plays on.
            Err(message) if self.lenient || !self.sources[clip].heard => {
                for out in out.iter_mut() {
                    out.fill(0.0);
                }
                if self.silenced.insert(clip) {
                    let event = if self.sources[clip].heard {
                        "sound_failed"
                    } else {
                        "sound_of_a_clip_unread"
                    };
                    log(
                        event,
                        &json!({"clip": self.sources[clip].clip, "message": message}),
                    );
                }
                Ok(())
            }
            Err(message) => {
                self.error.get_or_insert(message);
                Err(FeedError { clip })
            }
        }
    }
}

/// The mix of the monitored bus block by block, used by configured source playback and
/// tests with real media. Timeline playback and export run their graphs themselves.
pub struct Mixer {
    project: Arc<Project>,
    timeline: usize,
    purpose: Purpose,
    graph: Option<Graph>,
    feed: Option<Feed>,
    /// The sample of the timeline that the graph gives next, while it runs on.
    next: Option<i64>,
}

impl Mixer {
    /// The mixer of timeline `timeline` of `project`, for `purpose`: playback hears the Listen
    /// of a sidechain, an export never does.
    pub fn new(project: Arc<Project>, timeline: usize, purpose: Purpose) -> Self {
        Self {
            project,
            timeline,
            purpose,
            graph: None,
            feed: None,
            next: None,
        }
    }

    /// The graph and its readers, built when first wanted.
    fn graph(&mut self) -> Result<(&mut Graph, &mut Feed), String> {
        if self.graph.is_none() {
            let graph = Graph::build(
                &self.project,
                self.timeline,
                Options::new(MAX_BLOCK, self.purpose),
            )
            .map_err(|error| error.to_string())?;
            self.feed = Some(Feed::new(
                Arc::clone(&self.project),
                self.timeline,
                graph.clips(),
                false,
            ));
            for clip in graph.unplayable() {
                log("sound_of_a_clip_unplayable", &json!({ "clip": clip }));
            }
            self.graph = Some(graph);
        }
        Ok((
            self.graph.as_mut().expect("built above"),
            self.feed.as_mut().expect("built above"),
        ))
    }

    /// The monitored bus of `frames` samples of the timeline from `from`, in stereo (a bus of
    /// another format folded or spread into two by `route::convert`), not clipped. The graph
    /// runs on from where it was when `from` follows the last block, and starts again at `from`
    /// otherwise.
    pub fn block(&mut self, from: i64, frames: usize) -> Result<Vec<Stereo>, String> {
        let next = self.next.take();
        let (graph, feed) = self.graph()?;
        let failed = |feed: &mut Feed| {
            feed.error
                .take()
                .unwrap_or_else(|| "the sound of a clip could not be read".into())
        };
        feed.read.clear();
        if next != Some(from) {
            graph.start(from, feed).map_err(|_| failed(feed))?;
        }
        let monitor = graph.monitor();
        let matrix = monitor.map(|output| {
            let layout = graph.outputs()[output].layout;
            route::convert(layout, Channels::Stereo)
        });
        let mut mixed = Vec::with_capacity(frames);
        let mut done = 0;
        while done < frames {
            let count = (frames - done).min(MAX_BLOCK);
            graph.process(feed, count).map_err(|_| failed(feed))?;
            match (monitor, &matrix) {
                (Some(output), Some(matrix)) => {
                    let sound = graph.output(output);
                    for n in 0..count {
                        let mut pair = [0.0; 2];
                        for (side, pair) in pair.iter_mut().enumerate() {
                            for channel in 0..matrix.inputs() {
                                let gain = matrix.gain(side, channel);
                                if gain != 0.0 {
                                    *pair += gain * sound.channel(channel)[n];
                                }
                            }
                        }
                        mixed.push(pair);
                    }
                }
                _ => mixed.extend(std::iter::repeat_n([0.0; 2], count)),
            }
            done += count;
        }
        let read = std::mem::take(&mut feed.read);
        feed.readers.retain(|clip, _| read.contains(clip));
        self.next = Some(from + i64::try_from(frames).map_err(|_| "audio block is too long")?);
        Ok(mixed)
    }

    /// The mix made into the sound of a device of `format` from sample `from` of the timeline,
    /// 20 ms at a time, until the end of the timeline (an export of T4b, until T6).
    pub fn source(self, format: AudioFormat, from: i64) -> Box<dyn crate::audio::Source> {
        let timeline = &self.project.timelines[self.timeline];
        let remaining =
            (i128::from(end_of(timeline)) - i128::from(from)).max(0) * i128::from(format.rate);
        let remaining = usize::try_from(
            (remaining + i128::from(timeline.sample_rate) - 1) / i128::from(timeline.sample_rate),
        )
        .unwrap_or(usize::MAX);
        let mut resampler = Resampler::new(
            timeline.sample_rate,
            format.rate,
            usize::from(format.channels),
            MAX_BLOCK,
        );
        resampler.start(from);
        Box::new(OutputSource {
            next: resampler.first_input(),
            mixer: self,
            resampler,
            remaining,
            frames: usize::try_from(format.rate / 50).unwrap_or(1).max(1),
        })
    }
}

/// The sample of the timeline at the end of `timeline`, at its rate of sound.
pub(crate) fn end_of(timeline: &columbina_model::Timeline) -> i64 {
    i64::try_from(
        i128::from(resolve::duration(timeline))
            * i128::from(timeline.sample_rate)
            * i128::from(timeline.rate.den)
            / i128::from(timeline.rate.num),
    )
    .unwrap_or(i64::MAX)
}

/// The mix of the timeline made into the rate and channels of a device by the streaming
/// resampler of T5 (`columbina_audio::stream`), which keeps the samples of the mix that its
/// filter reaches back to: the graph runs through each sample once, in order (the plan's check of
/// the code, 1), and the sound for the device is clipped there, and only there.
struct OutputSource {
    mixer: Mixer,
    resampler: Resampler,
    /// The sample of the timeline mixed next.
    next: i64,
    /// Output-device frames left, not timeline samples.
    remaining: usize,
    /// The frames of each run.
    frames: usize,
}

impl crate::audio::Source for OutputSource {
    fn next(&mut self) -> Result<Option<Vec<f32>>, String> {
        if self.remaining == 0 {
            return Ok(None);
        }
        let channels = self.resampler.channels();
        let frames = self.frames.min(self.remaining);
        let mut output = vec![0.0; frames * channels];
        let mut made = 0;
        loop {
            made += self.resampler.pull(&mut output[made * channels..]);
            if made == frames {
                self.remaining -= frames;
                return Ok(Some(output));
            }
            let count = self.resampler.room().clamp(1, MAX_BLOCK);
            let mixed = self.mixer.block(self.next, count)?;
            let (left, right): (Vec<f32>, Vec<f32>) =
                mixed.iter().map(|pair| (pair[0], pair[1])).unzip();
            self.resampler.push(&left, &right);
            self.next += i64::try_from(count).map_err(|_| "audio block is too long")?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use columbina_model::{Time, TrackKind};

    fn real_project(sample: crate::test_media::Sample) -> Option<Project> {
        let made = crate::test_media::media(sample)?;
        let mut project = Project::sample();
        let facts = columbina_media::probe(&made.path).unwrap();
        project.media[0].path = made.path.to_string_lossy().into_owned();
        project.media[0].facts = facts.clone();
        let timeline = &mut project.timelines[0];
        timeline
            .tracks
            .retain(|track| track.kind == TrackKind::Audio);
        let clip = &mut timeline.tracks[0].clips[0];
        clip.source = columbina_model::Source::Media {
            media: "m1".into(),
            source_in: Time {
                ticks: 0,
                scale: facts.duration.scale,
            },
        };
        clip.length = facts.duration.ticks * 25 / i64::from(facts.duration.scale);
        Some(project)
    }

    #[test]
    fn real_pcm_reads_keep_sample_positions_across_windows_and_reverse_jumps() {
        let Some(project) = real_project(crate::test_media::Sample::Wav) else {
            return;
        };
        let mut decoder = columbina_media::AudioDecoder::open(
            std::path::Path::new(&project.media[0].path),
            Some(AudioFormat {
                rate: 48000,
                channels: 2,
            }),
        )
        .unwrap();
        let mut expected = Vec::new();
        while let Some(packet) = decoder.next().unwrap() {
            expected.extend(packet.data.as_chunks::<2>().0.iter().copied());
        }
        let mut mixer = Mixer::new(Arc::new(project), 0, Purpose::Preview);
        for from in [0, 960, 12000, 60000, 59000, 2000, 0] {
            let actual = mixer.block(from, 960).unwrap();
            let at = usize::try_from(from).unwrap();
            assert_eq!(actual, &expected[at..at + 960], "PCM at {from}");
            assert_eq!(mixer.feed.as_ref().unwrap().readers.len(), 1);
        }
    }

    #[test]
    fn real_mixing_keeps_the_video_origin_after_seeking_back_to_zero() {
        let Some(project) = real_project(crate::test_media::Sample::MkvOffsetAudio) else {
            return;
        };
        let mut mixer = Mixer::new(Arc::new(project), 0, Purpose::Preview);
        for from in [0, 60000, 0] {
            let actual = mixer.block(from, 960).unwrap();
            for (sample, pair) in actual.iter().enumerate() {
                assert_eq!(
                    *pair,
                    [0.25, -0.75],
                    "sound before the picture leaked into {from}+{sample}"
                );
            }
        }
    }

    #[test]
    fn a_real_mix_converts_to_the_device_rate_and_front_channels() {
        let Some(project) = real_project(crate::test_media::Sample::Wav) else {
            return;
        };
        let project = Arc::new(project);
        let mut source = Mixer::new(Arc::clone(&project), 0, Purpose::Preview).source(
            AudioFormat {
                rate: 44100,
                channels: 6,
            },
            0,
        );
        let mut played = Vec::new();
        for _ in 0..5 {
            played.extend(source.next().unwrap().unwrap());
        }
        assert_eq!(played.len(), 4410 * 6);
        for frame in played.as_chunks::<6>().0 {
            assert_eq!(frame[0], frame[1]);
            assert_eq!(frame[2..], [0.0; 4]);
        }
        // Same 440 Hz tone after sample-rate conversion; no zero or duplicated block.
        let left: Vec<f32> = played
            .as_chunks::<6>()
            .0
            .iter()
            .map(|frame| frame[0])
            .collect();
        let rises = left
            .windows(2)
            .filter(|pair| pair[0] <= 0.0 && pair[1] > 0.0)
            .count();
        assert!((43..=45).contains(&rises), "{rises} cycles in 100 ms");
        assert!(left.iter().any(|value| value.abs() > 0.08));
    }

    /// The blocks that a mix of the sound of `project` played on a device of `format` from
    /// sample `from` gives, at most `most` of them, until it ends.
    fn played(
        project: &Arc<Project>,
        format: AudioFormat,
        from: i64,
        most: usize,
    ) -> Vec<Vec<f32>> {
        let mut source = Mixer::new(Arc::clone(project), 0, Purpose::Preview).source(format, from);
        let mut blocks = vec![];
        while let Some(block) = source.next().unwrap() {
            blocks.push(block);
            if blocks.len() == most {
                break;
            }
        }
        blocks
    }

    #[test]
    fn a_real_mix_goes_on_from_block_to_block_and_ends_with_the_timeline() {
        let Some(project) = real_project(crate::test_media::Sample::Wav) else {
            return;
        };
        let project = Arc::new(project);
        let stereo = AudioFormat {
            rate: 44100,
            channels: 2,
        };
        // Blocks of 882 frames, 960 samples of the timeline: the second from 0 is the first from
        // 960.
        let from_start = played(&project, stereo, 0, 2);
        let from_second = played(&project, stereo, 960, 1);
        assert_eq!(from_start[1], from_second[0]);
        assert_ne!(from_start[0], from_start[1]);
        // A second from its end, of two, the timeline ends after 50 blocks.
        assert_eq!(played(&project, stereo, 48000, 60).len(), 50);
        // 59 frames at 30000/1001 a second end at sample 94494; from 48000, blocks start 960
        // samples apart.
        let mut ntsc = (*project).clone();
        ntsc.timelines[0].rate = columbina_model::Rate {
            num: 30000,
            den: 1001,
        };
        ntsc.timelines[0].tracks[0].clips[0].length = 59;
        assert_eq!(played(&Arc::new(ntsc), stereo, 48000, 60).len(), 49);
        // For one speaker, the two sides at equal power.
        let one = AudioFormat {
            rate: 44100,
            channels: 1,
        };
        let mono = played(&project, one, 0, 2);
        for (mono, stereo) in mono.iter().zip(&from_start) {
            assert_eq!(mono.len() * 2, stereo.len());
            for (one, pair) in mono.iter().zip(stereo.as_chunks::<2>().0) {
                let want = ((pair[0] + pair[1]) * std::f32::consts::FRAC_1_SQRT_2).clamp(-1.0, 1.0);
                assert!((one - want).abs() < 1e-6, "{one} != {want}");
            }
        }
        assert!(mono[0].iter().any(|value| value.abs() > 0.1));
        assert!(mono[0].iter().any(|value| *value < -0.1));
    }

    #[test]
    fn a_muted_track_of_offline_media_leaves_the_mix_of_the_others_as_it_is() {
        let Some(project) = real_project(crate::test_media::Sample::Wav) else {
            return;
        };
        // A track muted (a track that was off before schema 4), of media that is not there.
        let mut with = project.clone();
        let mut offline = with.media[0].clone();
        offline.id = "m9".into();
        offline.path = "C:/nowhere/that/is/offline.wav".into();
        with.media.push(offline);
        let mut track = with.timelines[0].tracks[0].clone();
        track.id = "A9".into();
        track.audio.mute = true;
        track.clips[0].id = "c9".into();
        track.clips[0].source = columbina_model::Source::Media {
            media: "m9".into(),
            source_in: Time {
                ticks: 0,
                scale: 48000,
            },
        };
        with.timelines[0].tracks.push(track);
        let (with, without) = (Arc::new(with), Arc::new(project));
        for purpose in [Purpose::Preview, Purpose::Export] {
            let heard = Mixer::new(Arc::clone(&without), 0, purpose)
                .block(0, 9600)
                .unwrap();
            let mut mixer = Mixer::new(Arc::clone(&with), 0, purpose);
            assert_eq!(mixer.block(0, 9600).unwrap(), heard, "{purpose:?}");
            assert_eq!(mixer.block(9600, 960).unwrap().len(), 960);
        }
        // Heard, the same track fails the mix, as it always did.
        let mut loud = (*with).clone();
        loud.timelines[0].tracks[1].audio.mute = false;
        assert!(Mixer::new(Arc::new(loud), 0, Purpose::Export)
            .block(0, 9600)
            .is_err());
    }

    #[test]
    #[allow(clippy::float_cmp, reason = "the clipped copy is exactly at the rails")]
    fn a_hot_mix_is_clipped_only_in_the_copy_for_the_device() {
        let Some(mut project) = real_project(crate::test_media::Sample::Wav) else {
            return;
        };
        project.timelines[0].tracks[0].clips[0].gain_db = 24.0.into();
        let project = Arc::new(project);
        let mixed = Mixer::new(Arc::clone(&project), 0, Purpose::Preview)
            .block(0, 4800)
            .unwrap();
        let loudest = mixed
            .iter()
            .flatten()
            .fold(0.0_f32, |most, sample| most.max(sample.abs()));
        assert!(loudest > 1.2, "the mix itself is not clipped: {loudest}");
        let stereo = AudioFormat {
            rate: 48000,
            channels: 2,
        };
        let played = played(&project, stereo, 0, 5);
        assert!(played.iter().flatten().all(|sample| sample.abs() <= 1.0));
        assert!(played.iter().flatten().any(|&sample| sample == 1.0));
    }

    /// A WAV of 16 bits, stereo, at 48 kHz, of `frames` frames that `sample` gives.
    fn wav(path: &std::path::Path, frames: u32, sample: impl Fn(u32) -> i16) {
        let data = frames * 4;
        let mut bytes = Vec::with_capacity(44 + data as usize);
        bytes.extend(b"RIFF");
        bytes.extend((36 + data).to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16_u32.to_le_bytes());
        bytes.extend(1_u16.to_le_bytes());
        bytes.extend(2_u16.to_le_bytes());
        bytes.extend(48000_u32.to_le_bytes());
        bytes.extend((48000_u32 * 4).to_le_bytes());
        bytes.extend(4_u16.to_le_bytes());
        bytes.extend(16_u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend(data.to_le_bytes());
        for n in 0..frames {
            let value = sample(n).to_le_bytes();
            bytes.extend(value);
            bytes.extend(value);
        }
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn a_mix_played_on_a_device_of_a_lower_rate_drops_what_is_above_its_nyquist_frequency() {
        if crate::test_media::media(crate::test_media::Sample::Wav).is_none() {
            return;
        }
        // A tone at 24 kHz, the most that 48 kHz holds, is above the 22.05 kHz of 44.1 kHz.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("high.wav");
        wav(&path, 96000, |n| if n % 2 == 0 { 16000 } else { -16000 });
        let mut project = real_project(crate::test_media::Sample::Wav).unwrap();
        let facts = columbina_media::probe(&path).unwrap();
        project.media[0].path = path.to_string_lossy().into_owned();
        project.media[0].facts = facts;
        let project = Arc::new(project);
        let format = AudioFormat {
            rate: 44100,
            channels: 2,
        };
        let blocks = played(&project, format, 24000, 10);
        let left: Vec<f64> = blocks
            .iter()
            .flat_map(|block| {
                block
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|pair| f64::from(pair[0]))
            })
            .collect();
        #[allow(clippy::cast_precision_loss)]
        let loudness =
            (left.iter().map(|value| value * value).sum::<f64>() / left.len() as f64).sqrt();
        assert!(loudness < 0.05, "{loudness}");
    }
}
