//! A clip's PCM window at the timeline rate, aligned to the media's picture origin, in the
//! channels of its track: as swresample makes them from the best stream of sound (auto), or a
//! channel for each lane of a mapping, taken from any channel of any stream of the media as
//! it is (RM20 pp. 3833-3836). It retains a quarter second on each side for sequential forward
//! and backward blocks, not the whole recording.

use std::ops::Range;
use std::path::{Path, PathBuf};

use columbina_media::{AudioDecoder, AudioFormat, MediaError, Samples};
use columbina_model::Media;

/// How the channels of a clip are made from its media: the `channels` of a clip of the model.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Layout {
    /// As many channels as the track has, made by swresample from the best stream of sound in
    /// the default layout of that many (a clip whose `channels` are `null`, as every clip of a
    /// project made before them is: its sound is what it was).
    Auto(u16),
    /// A channel for each lane, in the order of the channels of the track.
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "the graph (T4b) maps the clip's channels")
    )]
    Mapped(Vec<Lane>),
}

/// A channel of the media: the index of its stream among all the streams of the file (as
/// `columbina_media::sounds` lists them), and the channel within that stream, from 0, in the
/// order of the stream's own layout (5.1 is L R C LFE Ls Rs).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Pick {
    pub stream: usize,
    pub channel: u16,
}

/// A channel that a clip gives: the mean of the channels of the media picked for it (one, or
/// none for silence; two for a mono made of left and right, `0.5 · (L + R)`), times its gain.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Lane {
    pub sources: Vec<Pick>,
    /// Linear: 0 where the channel is disabled or muted.
    pub gain: f32,
}

/// At or below this a level is silence, as a fader's is.
const SILENT_DB: f64 = -100.0;

#[cfg_attr(
    not(test),
    allow(dead_code, reason = "the graph (T4b) maps the clip's channels")
)]
impl Lane {
    /// A channel as a clip of the model has it (RM20 pp. 3834-3836): its source, whether it is
    /// enabled (a channel that is not is taken out of the clip) and muted (kept in the clip,
    /// but silent), and its level in dB, at or below -100 dB silence.
    pub(crate) fn of(source: Option<Pick>, enabled: bool, mute: bool, level_db: f64) -> Self {
        Self::mean(source.into_iter().collect(), enabled, mute, level_db)
    }

    /// The mean of several channels of the media, as [`Lane::of`] has one.
    pub(crate) fn mean(sources: Vec<Pick>, enabled: bool, mute: bool, level_db: f64) -> Self {
        let gain = if !enabled || mute || level_db <= SILENT_DB {
            0.0
        } else {
            #[allow(clippy::cast_possible_truncation, reason = "audio gains are f32")]
            let gain = 10.0_f64.powf(level_db / 20.0) as f32;
            gain
        };
        Self { sources, gain }
    }
}

/// A stream of the media, decoded into a window of its own.
struct Stream {
    /// Its index in the file; none for the best stream of sound, mixed by swresample.
    index: Option<usize>,
    channels: usize,
    decoder: AudioDecoder,
    pending: Option<Samples>,
    ended: bool,
}

impl Stream {
    /// Open stream `index` of the file unmixed at `rate`, or the best stream in `format`. None
    /// where the file has no such stream of sound: what is picked of it is silent.
    fn open(
        path: &Path,
        index: Option<usize>,
        format: AudioFormat,
    ) -> Result<Option<Self>, MediaError> {
        let opened = match index {
            None => AudioDecoder::open(path, Some(format)),
            Some(index) => AudioDecoder::open_unmixed(path, index, Some(format.rate)),
        };
        let decoder = match opened {
            Ok(decoder) => decoder,
            Err(MediaError::NoStreams) if index.is_some() => return Ok(None),
            Err(error) => return Err(error),
        };
        Ok(Some(Self {
            index,
            channels: usize::from(decoder.format().channels),
            decoder,
            pending: None,
            ended: false,
        }))
    }

    /// Open it again, at the sample clock it starts with.
    fn reopen(&mut self, path: &Path, rate: u32) -> Result<(), MediaError> {
        let format = AudioFormat {
            rate,
            channels: u16::try_from(self.channels).unwrap_or(u16::MAX),
        };
        self.decoder = match self.index {
            None => AudioDecoder::open(path, Some(format))?,
            Some(index) => AudioDecoder::open_unmixed(path, index, Some(rate))?,
        };
        self.pending = None;
        self.ended = false;
        Ok(())
    }

    fn seek(&mut self, sample: i64) -> Result<(), MediaError> {
        self.decoder.seek(sample)?;
        self.pending = None;
        self.ended = false;
        Ok(())
    }

    /// Decode the samples from `from` to `to` (of the media, from its origin `origin`) into
    /// `output`, interleaved, which starts at `start`.
    fn fill(
        &mut self,
        origin: i64,
        start: i64,
        from: i64,
        to: i64,
        output: &mut [f32],
    ) -> Result<(), String> {
        let channels = self.channels;
        while !self.ended {
            if self.pending.is_none() {
                self.pending = self.decoder.next().map_err(|error| error.to_string())?;
                if self.pending.is_none() {
                    self.ended = true;
                    break;
                }
            }
            let samples = self.pending.as_ref().expect("read above");
            let begins = samples
                .start
                .checked_sub(origin)
                .ok_or("decoded audio is outside sample time")?;
            let length = samples.data.len() / channels;
            let end = begins.saturating_add(i64::try_from(length).expect("bounded decoder packet"));
            let left = from.max(begins);
            let right = to.min(end);
            if left < right {
                let source = usize::try_from(left - begins).expect("inside packet") * channels;
                let target = usize::try_from(left - start).expect("inside window") * channels;
                let count = usize::try_from(right - left).expect("positive overlap") * channels;
                output[target..target + count]
                    .copy_from_slice(&samples.data[source..source + count]);
            }
            if end >= to {
                break;
            }
            self.pending = None;
        }
        Ok(())
    }
}

pub(crate) struct Reader {
    path: PathBuf,
    rate: u32,
    /// How many channels it gives.
    channels: usize,
    /// The lanes of a mapping; none where the one stream's channels are given as they are.
    lanes: Option<Vec<Lane>>,
    /// The streams that are read, in the order of their indices.
    streams: Vec<Stream>,
    /// For each pick of each lane, where it is read: the place of its stream in `streams`, and
    /// the channel in that stream; none where the media lacks it, which is then silent.
    picks: Vec<Vec<Option<(usize, usize)>>>,
    origin: i64,
    length: i64,
    ahead: i64,
    start: i64,
    /// The window, `channels` floats an instant.
    pcm: Vec<f32>,
}

impl Reader {
    pub(crate) fn new(media: &Media, rate: u32, layout: &Layout) -> Result<Self, String> {
        let path = Path::new(&media.path);
        let cannot = |error: MediaError| error.to_string();
        let (channels, lanes, streams, picks) = match layout {
            Layout::Auto(channels) => {
                let format = AudioFormat {
                    rate,
                    channels: *channels,
                };
                let stream = Stream::open(path, None, format)
                    .map_err(cannot)?
                    .expect("the best stream is opened or fails");
                (usize::from(*channels), None, vec![stream], Vec::new())
            }
            Layout::Mapped(lanes) => {
                let mut indices: Vec<usize> = lanes
                    .iter()
                    .flat_map(|lane| lane.sources.iter().map(|pick| pick.stream))
                    .collect();
                indices.sort_unstable();
                indices.dedup();
                let mut streams = Vec::new();
                for index in indices {
                    let format = AudioFormat { rate, channels: 1 };
                    if let Some(stream) = Stream::open(path, Some(index), format).map_err(cannot)? {
                        streams.push(stream);
                    }
                }
                let picks = lanes
                    .iter()
                    .map(|lane| {
                        lane.sources
                            .iter()
                            .map(|pick| {
                                let place = streams
                                    .iter()
                                    .position(|stream| stream.index == Some(pick.stream))?;
                                let channel = usize::from(pick.channel);
                                (channel < streams[place].channels).then_some((place, channel))
                            })
                            .collect()
                    })
                    .collect();
                (lanes.len(), Some(lanes.clone()), streams, picks)
            }
        };
        let origin = if media.facts.video.is_some() {
            columbina_media::video_start(path).map_err(cannot)?
        } else if let Some(Stream { index: None, .. }) = streams.first() {
            streams[0].decoder.start()
        } else if streams.is_empty() {
            columbina_model::Time { ticks: 0, scale: 1 }
        } else {
            columbina_media::sound_start(path).map_err(cannot)?
        };
        let sample = |ticks, scale| {
            i64::try_from(i128::from(ticks) * i128::from(rate) / i128::from(scale))
                .map_err(|_| "media is outside audio sample time".to_owned())
        };
        Ok(Self {
            path: path.to_owned(),
            rate,
            channels,
            lanes,
            streams,
            picks,
            origin: sample(origin.ticks, origin.scale)?,
            length: sample(media.facts.duration.ticks, media.facts.duration.scale)?,
            ahead: i64::from(rate) / 4,
            start: 0,
            pcm: Vec::new(),
        })
    }

    /// How many channels it gives.
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "the graph (T4b) reads any number of channels")
    )]
    pub(crate) fn channels(&self) -> usize {
        self.channels
    }

    /// The samples of `range`, `channels` floats an instant; silence outside the media.
    pub(crate) fn read(&mut self, range: Range<i64>) -> Result<Vec<f32>, String> {
        let channels = self.channels;
        let frames = usize::try_from(range.end - range.start).map_err(|_| "invalid PCM range")?;
        let mut output = vec![0.0; frames * channels];
        let from = range.start.max(0);
        let to = range.end.min(self.length);
        if from >= to {
            return Ok(output);
        }
        let cached = i64::try_from(self.pcm.len() / channels.max(1)).expect("bounded PCM cache");
        let cached_end = self.start + cached;
        if from < self.start || to > cached_end {
            let start = (from - self.ahead).max(0);
            let end = to.saturating_add(self.ahead).min(self.length);
            let size = usize::try_from(end - start).map_err(|_| "PCM window is too large")?;
            let mut pcm = vec![0.0; size * channels];
            let next = if !self.pcm.is_empty() && start >= self.start && start < cached_end {
                let copied = cached_end.min(end) - start;
                let offset = usize::try_from(start - self.start).expect("inside old PCM window");
                let count = usize::try_from(copied).expect("inside new PCM window");
                pcm[..count * channels]
                    .copy_from_slice(&self.pcm[offset * channels..(offset + count) * channels]);
                start + copied
            } else {
                // At media zero, decode from the stream's original continuous sample clock;
                // seeking in a coarse container time base may round the first packet.
                if start == 0 && !self.pcm.is_empty() {
                    // Rewinding to the picture origin must retain the original continuous
                    // sample positions too (Matroska PCM packet timestamps can be rounded).
                    for stream in &mut self.streams {
                        stream
                            .reopen(&self.path, self.rate)
                            .map_err(|error| error.to_string())?;
                    }
                } else if start != 0 {
                    let absolute = start
                        .checked_add(self.origin)
                        .ok_or("audio seek is outside sample time")?;
                    for stream in &mut self.streams {
                        stream.seek(absolute).map_err(|error| error.to_string())?;
                    }
                }
                start
            };
            self.fill(start, next, end, &mut pcm)?;
            self.start = start;
            self.pcm = pcm;
        }
        let source = usize::try_from(from - self.start).expect("requested PCM was filled");
        let target = usize::try_from(from - range.start).expect("nonnegative output offset");
        let count = usize::try_from(to - from).expect("positive PCM range");
        output[target * channels..(target + count) * channels]
            .copy_from_slice(&self.pcm[source * channels..(source + count) * channels]);
        Ok(output)
    }

    /// Fill the window `output`, which starts at `start`, from `from` to `to`: the one stream
    /// as it is, or each stream into a window of its own, from which the lanes are made.
    fn fill(&mut self, start: i64, from: i64, to: i64, output: &mut [f32]) -> Result<(), String> {
        let origin = self.origin;
        let Some(lanes) = &self.lanes else {
            return self.streams[0].fill(origin, start, from, to, output);
        };
        let frames = usize::try_from(to - from).expect("an ordered window");
        let mut windows = Vec::with_capacity(self.streams.len());
        for stream in &mut self.streams {
            let mut window = vec![0.0; frames * stream.channels];
            stream.fill(origin, from, from, to, &mut window)?;
            windows.push(window);
        }
        let offset = usize::try_from(from - start).expect("inside window");
        for (lane, (spec, picks)) in lanes.iter().zip(&self.picks).enumerate() {
            if spec.sources.is_empty() || spec.gain <= 0.0 {
                continue;
            }
            #[allow(clippy::cast_precision_loss, reason = "a lane picks a few channels")]
            let gain = spec.gain / spec.sources.len() as f32;
            for frame in 0..frames {
                let mut sum = 0.0;
                for &(place, channel) in picks.iter().flatten() {
                    sum += windows[place][frame * self.streams[place].channels + channel];
                }
                output[(offset + frame) * self.channels + lane] = sum * gain;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_media::{media, Sample};
    use columbina_model::Project;

    /// The media of a sample of the tests, probed.
    fn of(sample: Sample) -> Option<Media> {
        let made = media(sample)?;
        let mut media = Project::sample().media.remove(0);
        media.path = made.path.to_string_lossy().into_owned();
        media.facts = columbina_media::probe(&made.path).unwrap();
        Some(media)
    }

    /// The amplitude of the sine of `hertz` in samples at 48 kHz, by the Goertzel filter.
    #[allow(clippy::cast_precision_loss)]
    fn amplitude(wave: &[f32], hertz: f64) -> f64 {
        let step = std::f64::consts::TAU * hertz / 48000.0;
        let coefficient = 2.0 * step.cos();
        let (mut before, mut last) = (0.0_f64, 0.0_f64);
        for &sample in wave {
            let next = f64::from(sample) + coefficient * last - before;
            before = last;
            last = next;
        }
        let power = last * last + before * before - coefficient * last * before;
        2.0 * power.max(0.0).sqrt() / wave.len() as f64
    }

    /// Channel `channel` of a second of what a reader gives from 0.5 s.
    fn second(reader: &mut Reader, channel: usize) -> Vec<f32> {
        let channels = reader.channels();
        reader
            .read(24000..72000)
            .unwrap()
            .into_iter()
            .skip(channel)
            .step_by(channels)
            .collect()
    }

    /// The source of a lane: a channel of a stream.
    #[allow(clippy::unnecessary_wraps, reason = "a lane takes an optional source")]
    fn pick(stream: usize, channel: u16) -> Option<Pick> {
        Some(Pick { stream, channel })
    }

    #[test]
    fn a_mono_track_that_takes_the_left_of_a_stereo_clip_hears_nothing_of_its_right() {
        let Some(media) = of(Sample::LeftRight) else {
            return;
        };
        let left = Layout::Mapped(vec![Lane::of(pick(0, 0), true, false, 0.0)]);
        let mut reader = Reader::new(&media, 48000, &left).unwrap();
        assert_eq!(reader.channels(), 1);
        let wave = second(&mut reader, 0);
        assert!((amplitude(&wave, 440.0) - 0.25).abs() < 1e-3);
        // Below -100 dBFS.
        assert!(
            amplitude(&wave, 880.0) < 1e-5,
            "{}",
            amplitude(&wave, 880.0)
        );
        // Taking the right instead, the other way round.
        let right = Layout::Mapped(vec![Lane::of(pick(0, 1), true, false, 0.0)]);
        let wave = second(&mut Reader::new(&media, 48000, &right).unwrap(), 0);
        assert!((amplitude(&wave, 880.0) - 0.25).abs() < 1e-3);
        assert!(amplitude(&wave, 440.0) < 1e-5);
    }

    #[test]
    fn a_channel_is_taken_from_any_stream_and_laid_on_any_channel_of_the_track() {
        let Some(media) = of(Sample::TwoMonoStreams) else {
            return;
        };
        // The second microphone on the left, the first on the right.
        let crossed = Layout::Mapped(vec![
            Lane::of(pick(1, 0), true, false, 0.0),
            Lane::of(pick(0, 0), true, false, 0.0),
        ]);
        let mut reader = Reader::new(&media, 48000, &crossed).unwrap();
        assert_eq!(reader.channels(), 2);
        let (left, right) = (second(&mut reader, 0), second(&mut reader, 1));
        assert!((amplitude(&left, 880.0) - 0.25).abs() < 1e-3);
        assert!(amplitude(&left, 440.0) < 1e-5);
        assert!((amplitude(&right, 440.0) - 0.25).abs() < 1e-3);
        assert!(amplitude(&right, 880.0) < 1e-5);
        // Auto, the best stream alone, made stereo.
        let mut auto = Reader::new(&media, 48000, &Layout::Auto(2)).unwrap();
        assert!(amplitude(&second(&mut auto, 1), 880.0) < 1e-5);
    }

    #[test]
    fn levels_mutes_and_channels_taken_out_of_a_clip_are_heard_as_they_say() {
        let Some(media) = of(Sample::LeftRight) else {
            return;
        };
        let level = |lane: Lane| {
            let mut reader = Reader::new(&media, 48000, &Layout::Mapped(vec![lane])).unwrap();
            amplitude(&second(&mut reader, 0), 440.0)
        };
        let full = level(Lane::of(pick(0, 0), true, false, 0.0));
        let down = level(Lane::of(pick(0, 0), true, false, -6.0));
        let db = 20.0 * (down / full).log10();
        assert!((db + 6.0).abs() < 1e-3, "{db}");
        let up = level(Lane::of(pick(0, 0), true, false, 12.0));
        assert!((20.0 * (up / full).log10() - 12.0).abs() < 1e-3);
        for silent in [
            Lane::of(pick(0, 0), true, true, 0.0),
            Lane::of(pick(0, 0), false, false, 0.0),
            Lane::of(pick(0, 0), true, false, -100.0),
            Lane::of(None, true, false, 0.0),
            // What the media lacks: a third channel, a second stream.
            Lane::of(pick(0, 2), true, false, 0.0),
            Lane::of(pick(1, 0), true, false, 0.0),
        ] {
            let mut reader =
                Reader::new(&media, 48000, &Layout::Mapped(vec![silent.clone()])).unwrap();
            let pcm = reader.read(0..96000).unwrap();
            assert!(pcm.iter().all(|&sample| sample == 0.0), "{silent:?}");
        }
    }

    #[test]
    fn left_and_right_made_one_are_their_mean() {
        let Some(media) = of(Sample::LeftRight) else {
            return;
        };
        let both = Layout::Mapped(vec![Lane::mean(
            vec![
                Pick {
                    stream: 0,
                    channel: 0,
                },
                Pick {
                    stream: 0,
                    channel: 1,
                },
            ],
            true,
            false,
            0.0,
        )]);
        let mut reader = Reader::new(&media, 48000, &both).unwrap();
        let wave = second(&mut reader, 0);
        // 0.5 (L + R): each of the two tones at half its amplitude, -6.02 dB.
        assert!((amplitude(&wave, 440.0) - 0.125).abs() < 1e-3);
        assert!((amplitude(&wave, 880.0) - 0.125).abs() < 1e-3);
        let alone = |channel| {
            let lane = Lane::of(pick(0, channel), true, false, 0.0);
            let mut reader = Reader::new(&media, 48000, &Layout::Mapped(vec![lane])).unwrap();
            reader.read(0..4800).unwrap()
        };
        let (left, right) = (alone(0), alone(1));
        let mean = reader.read(0..4800).unwrap();
        for ((mean, left), right) in mean.iter().zip(&left).zip(&right) {
            assert!((mean - 0.5 * (left + right)).abs() < 1e-7);
        }
    }

    #[test]
    fn a_mono_clip_made_stereo_by_auto_is_3_db_down_on_each_side() {
        let Some(media) = of(Sample::Mono) else {
            return;
        };
        let mut mono = Reader::new(&media, 48000, &Layout::Auto(1)).unwrap();
        let alone = amplitude(&second(&mut mono, 0), 440.0);
        let mut stereo = Reader::new(&media, 48000, &Layout::Auto(2)).unwrap();
        for side in 0..2 {
            let made = amplitude(&second(&mut stereo, side), 440.0);
            let db = 20.0 * (made / alone).log10();
            assert!((db + 3.0103).abs() < 0.01, "side {side}: {db} dB");
        }
    }

    #[test]
    fn the_six_channels_of_a_5_1_clip_are_read_as_they_are_or_picked_one_by_one() {
        let Some(media) = of(Sample::SixSines) else {
            return;
        };
        let mut auto = Reader::new(&media, 48000, &Layout::Auto(6)).unwrap();
        let lanes = (0..6)
            .map(|channel| Lane::of(pick(0, channel), true, false, 0.0))
            .collect();
        let mut mapped = Reader::new(&media, 48000, &Layout::Mapped(lanes)).unwrap();
        let whole = auto.read(0..96000).unwrap();
        assert_eq!(mapped.read(0..96000).unwrap(), whole);
        for channel in 0..6 {
            let wave = second(&mut auto, channel);
            let hertz = 100.0 * f64::from(u32::try_from(channel).unwrap() + 1);
            assert!((amplitude(&wave, hertz) - 0.1).abs() < 1e-3, "{channel}");
        }
        // The low frequencies and the centre swapped.
        let mut swapped = Reader::new(
            &media,
            48000,
            &Layout::Mapped(vec![
                Lane::of(pick(0, 3), true, false, 0.0),
                Lane::of(pick(0, 2), true, false, 0.0),
            ]),
        )
        .unwrap();
        assert!((amplitude(&second(&mut swapped, 0), 400.0) - 0.1).abs() < 1e-3);
        assert!((amplitude(&second(&mut swapped, 1), 300.0) - 0.1).abs() < 1e-3);
    }

    #[test]
    fn mapped_reads_keep_sample_positions_across_windows_and_reverse_jumps() {
        let Some(media) = of(Sample::TwoMonoStreams) else {
            return;
        };
        // Each stream decoded from its start, laid crossed: the second on the left.
        let stream = |index| {
            let path = std::path::Path::new(&media.path);
            let mut decoder = AudioDecoder::open_unmixed(path, index, Some(48000)).unwrap();
            let mut all = Vec::new();
            while let Some(run) = decoder.next().unwrap() {
                all.extend(run.data);
            }
            all
        };
        let (first, second) = (stream(0), stream(1));
        let expected: Vec<f32> = second
            .iter()
            .zip(&first)
            .flat_map(|(left, right)| [*left, *right])
            .collect();
        let crossed = Layout::Mapped(vec![
            Lane::of(pick(1, 0), true, false, 0.0),
            Lane::of(pick(0, 0), true, false, 0.0),
        ]);
        let mut reader = Reader::new(&media, 48000, &crossed).unwrap();
        for from in [0, 960, 12000, 60000, 59000, 30000, 2000, 47_001, 0] {
            let actual = reader.read(from..from + 960).unwrap();
            let at = usize::try_from(from).unwrap() * 2;
            assert_eq!(actual, &expected[at..at + 1920], "PCM at {from}");
        }
        // Past the end, silence.
        let tail = reader.read(95_000..97_000).unwrap();
        assert_eq!(tail[..2000], expected[190_000..]);
        assert!(tail[2000..].iter().all(|&sample| sample == 0.0));
    }
}
