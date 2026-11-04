//! The sound of a timeline for the transport and the export: the graph of `columbina-audio`
//! (plan T4b), built from the project and fed by the media readers of its clips. The mix stays in
//! 32-bit floats to the device; only the copy for the device is clipped.

mod reader;

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::sync::Arc;

use columbina_audio::feed::{ClipFeed, ClipSource, FeedError};
use columbina_audio::graph::{self, route, Graph, Options, Purpose};
use columbina_audio::node::Layout as Channels;
use columbina_media::AudioFormat;
use columbina_model::{resolve, Project};
use serde_json::json;

use crate::log::log;
use reader::{Layout, Reader};

type Stereo = [f32; 2];

/// The most samples that the graph of a mixer processes at once.
const MAX_BLOCK: usize = 4096;

/// The readers of the clips of a graph, opened as the graph first asks for each.
struct Feed {
    project: Arc<Project>,
    timeline: usize,
    sources: Vec<ClipSource>,
    readers: BTreeMap<usize, Reader>,
    /// The clips read in the block under way.
    read: BTreeSet<usize>,
    /// What went wrong first, since it was last taken.
    error: Option<String>,
    /// The clips that the mix does not need and could not be read, told of once each.
    silenced: BTreeSet<usize>,
}

impl Feed {
    fn fill(
        &mut self,
        clip: usize,
        range: Range<i64>,
        out: &mut [&mut [f32]],
    ) -> Result<(), String> {
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
            let reader = Reader::new(
                media,
                timeline.sample_rate,
                &Layout::of_clip(model, channels),
            )?;
            self.readers.insert(clip, reader);
        }
        let reader = self.readers.get_mut(&clip).expect("opened above");
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
}

impl ClipFeed for Feed {
    fn read(
        &mut self,
        clip: usize,
        range: Range<i64>,
        out: &mut [&mut [f32]],
    ) -> Result<(), FeedError> {
        self.read.insert(clip);
        match self.fill(clip, range, out) {
            Ok(()) => Ok(()),
            // A clip that is not heard (a muted track, which a track that was off before schema
            // 4 is) plays silence when its media cannot be read, as it did when it was not read.
            Err(message) if !self.sources[clip].heard => {
                for out in out.iter_mut() {
                    out.fill(0.0);
                }
                if self.silenced.insert(clip) {
                    log(
                        "sound_of_a_clip_unread",
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
            self.feed = Some(Feed {
                project: Arc::clone(&self.project),
                timeline: self.timeline,
                sources: graph.clips().to_vec(),
                readers: BTreeMap::new(),
                read: BTreeSet::new(),
                error: None,
                silenced: BTreeSet::new(),
            });
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

    pub fn source(self, format: AudioFormat, from: i64) -> Box<dyn crate::audio::Source> {
        Box::new(OutputSource {
            mixer: self,
            format,
            from,
            played: 0,
            window: [vec![], vec![]],
            window_from: 0,
        })
    }
}

/// Converts the stereo timeline mix to the device rate/layout without restarting the
/// fractional resampling phase at each 20 ms output block. The mix of the timeline is kept
/// between blocks, so that the graph runs through each sample once, in order (the plan's check
/// of the code, 1), and each block mixes only what the last did not; the sound for the device is
/// clipped here, and only here.
struct OutputSource {
    mixer: Mixer,
    format: AudioFormat,
    from: i64,
    played: i64,
    /// The mix of the timeline from `window_from`, left and right.
    window: [Vec<f32>; 2],
    window_from: i64,
}

impl crate::audio::Source for OutputSource {
    #[allow(
        clippy::cast_precision_loss,
        reason = "the resampling phase is a fraction in [0, 1)"
    )]
    fn next(&mut self) -> Result<Option<Vec<f32>>, String> {
        let timeline = &self.mixer.project.timelines[self.mixer.timeline];
        let rate = timeline.sample_rate;
        let frames = self.format.rate / 50;
        let end = i128::from(resolve::duration(timeline))
            * i128::from(rate)
            * i128::from(timeline.rate.den)
            / i128::from(timeline.rate.num);
        let (start, device) = (self.from, self.format.rate);
        let at = |played: i64| {
            let position =
                i128::from(start) * i128::from(device) + i128::from(played) * i128::from(rate);
            (
                position.div_euclid(i128::from(device)),
                position.rem_euclid(i128::from(device)),
            )
        };
        let (first, _) = at(self.played);
        if first >= end {
            return Ok(None);
        }
        let (last, _) = at(self.played + i64::from(frames) - 1);
        let cutoff = (f64::from(self.format.rate) / f64::from(rate)).min(1.0);
        let radius = i128::from(graph::radius(cutoff));
        let from = i64::try_from(first - radius)
            .map_err(|_| "output time is outside audio sample time")?;
        let to = i64::try_from(last + radius + 1)
            .map_err(|_| "output time is outside audio sample time")?;
        self.mix(from, to)?;
        let window = [self.window[0].as_slice(), self.window[1].as_slice()];
        let mut output = Vec::with_capacity(
            usize::try_from(frames).expect("device run") * usize::from(self.format.channels),
        );
        for offset in 0..frames {
            let (sample, remainder) = at(self.played + i64::from(offset));
            let index =
                usize::try_from(sample - i128::from(from)).expect("sample in output window");
            let mut stereo = [0.0; 2];
            graph::resampled(
                &window,
                index,
                remainder as f64 / f64::from(self.format.rate),
                cutoff,
                &mut stereo,
            );
            if self.format.channels == 1 {
                output.push(
                    ((stereo[0] + stereo[1]) * std::f32::consts::FRAC_1_SQRT_2).clamp(-1.0, 1.0),
                );
            } else {
                output.extend(stereo.map(|side| side.clamp(-1.0, 1.0)));
                output.extend(std::iter::repeat_n(
                    0.0,
                    usize::from(self.format.channels) - 2,
                ));
            }
        }
        self.played += i64::from(frames);
        Ok(Some(output))
    }
}

impl OutputSource {
    /// Keeps the mix of the timeline from `from` to the sample before `to` in the window: what
    /// it already has of it is kept, and only what follows is mixed, so that the mixer runs on.
    fn mix(&mut self, from: i64, to: i64) -> Result<(), String> {
        let have = self.window_from
            + i64::try_from(self.window[0].len()).map_err(|_| "audio window is too large")?;
        if from < self.window_from || from > have {
            for side in &mut self.window {
                side.clear();
            }
        } else {
            let gone = usize::try_from(from - self.window_from).expect("within the window");
            for side in &mut self.window {
                side.drain(..gone);
            }
        }
        self.window_from = from;
        let have =
            from + i64::try_from(self.window[0].len()).map_err(|_| "audio window is too large")?;
        if to > have {
            let more = self.mixer.block(
                have,
                usize::try_from(to - have).map_err(|_| "output audio window is too large")?,
            )?;
            for pair in more {
                self.window[0].push(pair[0]);
                self.window[1].push(pair[1]);
            }
        }
        Ok(())
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
