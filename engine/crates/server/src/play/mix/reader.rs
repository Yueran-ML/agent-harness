//! A clip's PCM window at the timeline rate, aligned to the media's picture origin, in the
//! channels of its track: as swresample makes them from the best stream of sound (auto), or a
//! channel for each lane of a mapping, taken from any channel of any stream of the media as
//! it is (RM20 pp. 3833-3836). It retains a quarter second on each side for sequential forward
//! and backward blocks, not the whole recording.

#[cfg(test)]
mod remix_tests;

use std::ops::Range;
use std::path::{Path, PathBuf};

use columbina_media::{AudioDecoder, AudioFormat, AudioStreams, MediaError, Samples};
use columbina_model::{ChannelSource, Clip, Media, RemixSource, Time};

/// How the channels of a clip are made from its media: the `channels` of a clip of the model.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Layout {
    /// As many channels as the track has, made by swresample from the best stream of sound in
    /// the default layout of that many (a clip whose `channels` are `null`, as every clip of a
    /// project made before them is: its sound is what it was).
    Auto(u16),
    /// A channel for each lane, in the order of the channels of the track.
    Mapped(Vec<Lane>),
}

impl Layout {
    /// The layout of a clip of the model on a track of `channels` channels: auto where its
    /// `channels` are `null`, and otherwise a lane for each channel that it names, the mean of
    /// the channels of the media that its `source` picks (none: silence).
    pub(crate) fn of_clip(clip: &Clip, channels: u16) -> Self {
        match &clip.channels {
            None => Self::Auto(channels),
            Some(maps) => Self::Mapped(
                maps.iter()
                    .map(|map| {
                        let sources = map.source.0.iter().copied().map(Pick::of).collect();
                        let mut lane = Lane::mean(sources, map.enabled, map.mute, map.level_db);
                        lane.remix = map.remix;
                        lane
                    })
                    .collect(),
            ),
        }
    }
}

/// A channel of the media: the index of its stream among all the streams of the file (as
/// `columbina_media::sounds` lists them), and the channel within that stream, from 0, in the
/// order of the stream's own layout (5.1 is L R C LFE Ls Rs).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Pick {
    pub stream: usize,
    pub channel: u16,
}

impl Pick {
    /// A pick of no channel of any file: silence, as one of a stream or a channel that the
    /// media has not is.
    const NOWHERE: Pick = Pick {
        stream: usize::MAX,
        channel: 0,
    };

    /// The pick of a channel that a clip of the model names. A channel past what a stream can
    /// have (65535) is of none, and so is silence: it is never cut down to another.
    fn of(source: ChannelSource) -> Self {
        match (
            usize::try_from(source.stream),
            u16::try_from(source.channel),
        ) {
            (Ok(stream), Ok(channel)) if stream != usize::MAX => Pick { stream, channel },
            _ => Self::NOWHERE,
        }
    }
}

/// A channel that a clip gives: the mean of the channels of the media picked for it (one, or
/// none for silence; two for a mono made of left and right, `0.5 · (L + R)`), times its gain.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Lane {
    pub sources: Vec<Pick>,
    pub remix: Option<RemixSource>,
    /// Linear: 0 where the channel is disabled or muted.
    pub gain: f32,
}

/// At or below this a level is silence, as a fader's is.
const SILENT_DB: f64 = -100.0;

impl Lane {
    /// A channel as a clip of the model has it (RM20 pp. 3834-3836): its source, whether it is
    /// enabled (a channel that is not is taken out of the clip) and muted (kept in the clip,
    /// but silent), and its level in dB, at or below -100 dB silence.
    #[cfg(test)]
    pub(crate) fn of(source: Option<Pick>, enabled: bool, mute: bool, level_db: f64) -> Self {
        Self::mean(source.into_iter().collect(), enabled, mute, level_db)
    }

    /// The mean of several channels of the media, as `Lane::of` (of the tests) has one.
    pub(crate) fn mean(sources: Vec<Pick>, enabled: bool, mute: bool, level_db: f64) -> Self {
        let gain = if !enabled || mute || level_db <= SILENT_DB {
            0.0
        } else {
            #[allow(clippy::cast_possible_truncation, reason = "audio gains are f32")]
            let gain = 10.0_f64.powf(level_db / 20.0) as f32;
            gain
        };
        Self {
            sources,
            remix: None,
            gain,
        }
    }
}

/// Where the samples come from: the best stream of sound, mixed by swresample (auto), or the
/// native streams from one reading of the file, plus one decoder per distinct automatic
/// output layout. Every lane using that layout shares its decoder and bounded PCM window.
enum Source {
    Mixed(AudioDecoder),
    Mapped {
        streams: AudioStreams,
        remixes: Vec<(u16, AudioDecoder)>,
    },
}

impl Source {
    fn open(path: &Path, rate: u32, layout: &Layout) -> Result<Self, MediaError> {
        Ok(match layout {
            Layout::Auto(channels) => Self::Mixed(AudioDecoder::open(
                path,
                Some(AudioFormat {
                    rate,
                    channels: *channels,
                }),
            )?),
            Layout::Mapped(lanes) => {
                let indices: Vec<usize> = lanes
                    .iter()
                    .flat_map(|lane| lane.sources.iter().map(|pick| pick.stream))
                    .collect();
                let streams = AudioStreams::open_unmixed(path, &indices, Some(rate))?;
                let mut counts: Vec<_> = lanes
                    .iter()
                    .filter_map(|lane| lane.remix.map(|source| source.channels))
                    .collect();
                counts.sort_unstable();
                counts.dedup();
                let remixes = counts
                    .into_iter()
                    .map(|channels| {
                        AudioDecoder::open(path, Some(AudioFormat { rate, channels }))
                            .map(|decoder| (channels, decoder))
                    })
                    .collect::<Result<_, _>>()?;
                Self::Mapped { streams, remixes }
            }
        })
    }

    /// How many streams it reads.
    fn len(&self) -> usize {
        match self {
            Self::Mixed(_) => 1,
            Self::Mapped { streams, remixes } => streams.len() + remixes.len(),
        }
    }

    /// The channels of the stream at `place`.
    fn channels(&self, place: usize) -> usize {
        usize::from(match self {
            Self::Mixed(decoder) => decoder.format().channels,
            Self::Mapped { streams, remixes } => {
                if place < streams.len() {
                    streams.format(place).channels
                } else {
                    remixes[place - streams.len()].1.format().channels
                }
            }
        })
    }

    fn seek(&mut self, sample: i64) -> Result<(), MediaError> {
        match self {
            Self::Mixed(decoder) => decoder.seek(sample),
            Self::Mapped { streams, remixes } => {
                streams.seek(sample)?;
                for (_, decoder) in remixes {
                    decoder.seek(sample)?;
                }
                Ok(())
            }
        }
    }

    fn next(&mut self, place: usize) -> Result<Option<Samples>, MediaError> {
        match self {
            Self::Mixed(decoder) => decoder.next(),
            Self::Mapped { streams, remixes } => {
                if place < streams.len() {
                    streams.next(place)
                } else {
                    remixes[place - streams.len()].1.next()
                }
            }
        }
    }

    fn start(&self) -> Time {
        match self {
            Self::Mixed(decoder) => decoder.start(),
            Self::Mapped { streams, .. } => streams.start(),
        }
    }

    fn picks(&self, lane: &Lane) -> Vec<Option<(usize, usize)>> {
        let Self::Mapped { streams, remixes } = self else {
            return vec![];
        };
        if let Some(source) = lane.remix {
            return vec![remixes
                .iter()
                .position(|(count, _)| *count == source.channels)
                .filter(|_| source.channel < source.channels)
                .map(|place| (streams.len() + place, usize::from(source.channel)))];
        }
        lane.sources
            .iter()
            .map(|pick| {
                let place = streams.place(pick.stream)?;
                let channel = usize::from(pick.channel);
                (channel < usize::from(streams.format(place).channels)).then_some((place, channel))
            })
            .collect()
    }
}

/// Where the decoding of a stream is: the run it gave last, which may reach past the window.
#[derive(Default)]
struct Lag {
    pending: Option<Samples>,
    ended: bool,
}

/// Decode the samples of the stream at `place` from `from` to `to` (of the media, from its
/// origin `origin`) into `output`, interleaved, which starts at `start`.
#[allow(
    clippy::too_many_arguments,
    reason = "the window and where it is filled"
)]
fn fill_stream(
    source: &mut Source,
    place: usize,
    lag: &mut Lag,
    origin: i64,
    start: i64,
    (from, to): (i64, i64),
    output: &mut [f32],
) -> Result<(), String> {
    let channels = source.channels(place);
    while !lag.ended {
        if lag.pending.is_none() {
            lag.pending = source.next(place).map_err(|error| error.to_string())?;
            if lag.pending.is_none() {
                lag.ended = true;
                break;
            }
        }
        let samples = lag.pending.as_ref().expect("read above");
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
            output[target..target + count].copy_from_slice(&samples.data[source..source + count]);
        }
        if end >= to {
            break;
        }
        lag.pending = None;
    }
    Ok(())
}

pub(crate) struct Reader {
    path: PathBuf,
    rate: u32,
    layout: Layout,
    /// How many channels it gives.
    channels: usize,
    source: Source,
    /// Where the decoding of each stream of `source` is.
    lags: Vec<Lag>,
    /// For each pick of each lane of a mapping, where it is read: the place of its stream in
    /// `source`, and the channel in that stream; none where the media lacks it, which is then
    /// silent.
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
        let source = Source::open(path, rate, layout).map_err(cannot)?;
        let channels = match layout {
            Layout::Auto(channels) => usize::from(*channels),
            Layout::Mapped(lanes) => lanes.len(),
        };
        let picks = match layout {
            Layout::Mapped(lanes) => lanes.iter().map(|lane| source.picks(lane)).collect(),
            Layout::Auto(_) => Vec::new(),
        };
        let origin = if media.facts.video.is_some() {
            columbina_media::video_start(path).map_err(cannot)?
        } else {
            source.start()
        };
        let sample = |ticks, scale| {
            i64::try_from(i128::from(ticks) * i128::from(rate) / i128::from(scale))
                .map_err(|_| "media is outside audio sample time".to_owned())
        };
        Ok(Self {
            path: path.to_owned(),
            rate,
            layout: layout.clone(),
            channels,
            lags: (0..source.len()).map(|_| Lag::default()).collect(),
            source,
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
                    self.source = Source::open(&self.path, self.rate, &self.layout)
                        .map_err(|error| error.to_string())?;
                    self.lags.iter_mut().for_each(|lag| *lag = Lag::default());
                } else if start != 0 {
                    let absolute = start
                        .checked_add(self.origin)
                        .ok_or("audio seek is outside sample time")?;
                    self.source
                        .seek(absolute)
                        .map_err(|error| error.to_string())?;
                    self.lags.iter_mut().for_each(|lag| *lag = Lag::default());
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
    /// as it is, or each stream into a window of its own, side by side, from which the lanes
    /// are made.
    fn fill(&mut self, start: i64, from: i64, to: i64, output: &mut [f32]) -> Result<(), String> {
        let origin = self.origin;
        let Layout::Mapped(lanes) = &self.layout else {
            return fill_stream(
                &mut self.source,
                0,
                &mut self.lags[0],
                origin,
                start,
                (from, to),
                output,
            );
        };
        let frames = usize::try_from(to - from).expect("an ordered window");
        let mut windows = Vec::with_capacity(self.lags.len());
        for (place, lag) in self.lags.iter_mut().enumerate() {
            let mut window = vec![0.0; frames * self.source.channels(place)];
            fill_stream(
                &mut self.source,
                place,
                lag,
                origin,
                from,
                (from, to),
                &mut window,
            )?;
            windows.push(window);
        }
        let offset = usize::try_from(from - start).expect("inside window");
        for (lane, (spec, picks)) in lanes.iter().zip(&self.picks).enumerate() {
            if picks.is_empty() || spec.gain <= 0.0 {
                continue;
            }
            #[allow(clippy::cast_precision_loss, reason = "a lane picks a few channels")]
            let gain = spec.gain / picks.len() as f32;
            for frame in 0..frames {
                let mut sum = 0.0;
                for &(place, channel) in picks.iter().flatten() {
                    sum += windows[place][frame * self.source.channels(place) + channel];
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
        assert!((amplitude(&left, 880.0) - 0.5).abs() < 1e-3);
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

    #[test]
    fn a_channel_picked_is_the_channel_of_the_stream_in_its_own_layout() {
        let Some(media) = of(Sample::ThreeZero) else {
            return;
        };
        // The third channel of 3.0 is its C, which swresample would lay on the left and right of
        // 2.1, the default of three channels.
        let centre = Layout::Mapped(vec![Lane::of(pick(0, 2), true, false, 0.0)]);
        let wave = second(&mut Reader::new(&media, 48000, &centre).unwrap(), 0);
        assert!((amplitude(&wave, 300.0) - 0.25).abs() < 1e-3);
        for other in [440.0, 660.0] {
            assert!(amplitude(&wave, other) < 1e-5, "{other} Hz");
        }
    }

    #[test]
    fn a_stream_that_starts_later_is_heard_from_where_it_starts() {
        let Some(media) = of(Sample::LateStream) else {
            return;
        };
        let late = Layout::Mapped(vec![
            Lane::of(pick(0, 0), true, false, 0.0),
            Lane::of(pick(1, 0), true, false, 0.0),
        ]);
        let mut reader = Reader::new(&media, 48000, &late).unwrap();
        for from in [0, 30000, 0] {
            let pcm = reader.read(from..from + 48000).unwrap();
            let right: Vec<f32> = pcm.iter().skip(1).step_by(2).copied().collect();
            let starts = usize::try_from(24000 - from).unwrap_or(0).min(48000);
            assert!(
                right[..starts].iter().all(|&sample| sample == 0.0),
                "{from}"
            );
            assert!(
                (amplitude(&right[starts..], 880.0) - 0.5).abs() < 2e-3,
                "{from}"
            );
        }
    }

    /// A clip of the sample whose channels are mapped so.
    fn mapped(maps: Option<Vec<columbina_model::ChannelMap>>) -> Clip {
        let mut clip = Project::sample().timelines[0].tracks[1].clips[0].clone();
        clip.channels = maps;
        clip
    }

    fn channel(picks: &[(u32, u32)]) -> columbina_model::ChannelMap {
        columbina_model::ChannelMap {
            remix: None,
            source: columbina_model::ChannelSources(
                picks
                    .iter()
                    .map(|&(stream, channel)| ChannelSource { stream, channel })
                    .collect(),
            ),
            enabled: true,
            mute: false,
            level_db: 0.0,
        }
    }

    #[test]
    fn the_channels_of_a_clip_of_the_model_are_laid_out_as_it_names_them() {
        // None: auto, in the channels of the track, as every clip before schema 4 was.
        assert_eq!(Layout::of_clip(&mapped(None), 2), Layout::Auto(2));
        assert_eq!(Layout::of_clip(&mapped(None), 6), Layout::Auto(6));
        // One pick, a mean of two, none, and a channel past 65535, which is of none.
        let clip = mapped(Some(vec![
            channel(&[(1, 0)]),
            channel(&[(0, 0), (0, 1)]),
            channel(&[]),
            channel(&[(0, 65_536)]),
        ]));
        let Layout::Mapped(lanes) = Layout::of_clip(&clip, 4) else {
            panic!("mapped")
        };
        let at = |stream, channel| Pick { stream, channel };
        let picks: Vec<Vec<Pick>> = lanes.iter().map(|lane| lane.sources.clone()).collect();
        assert_eq!(
            picks,
            [
                vec![at(1, 0)],
                vec![at(0, 0), at(0, 1)],
                vec![],
                vec![Pick::NOWHERE]
            ]
        );
        assert!(lanes
            .iter()
            .all(|lane| (lane.gain - 1.0).abs() < f32::EPSILON));
    }

    #[test]
    fn a_clip_of_no_mapping_reads_as_auto_did_and_one_that_maps_reads_its_channels() {
        let Some(media) = of(Sample::LeftRight) else {
            return;
        };
        let read = |layout: &Layout| {
            Reader::new(&media, 48000, layout)
                .unwrap()
                .read(0..48000)
                .unwrap()
        };
        let auto = read(&Layout::Auto(2));
        let model = read(&Layout::of_clip(&mapped(None), 2));
        assert_eq!(auto.len(), model.len());
        assert!(auto
            .iter()
            .zip(&model)
            .all(|(a, b)| a.to_bits() == b.to_bits()));
        // The left alone, on a track of one channel; and a channel of 65536, cut down to none,
        // is silence.
        let left = read(&Layout::of_clip(&mapped(Some(vec![channel(&[(0, 0)])])), 1));
        assert_eq!(left.len(), 48000);
        assert!((amplitude(&left, 440.0) - 0.25).abs() < 1e-3 && amplitude(&left, 880.0) < 1e-5);
        let far = mapped(Some(vec![channel(&[(0, 65_536)])]));
        let far = read(&Layout::of_clip(&far, 1));
        assert!(far.iter().all(|&sample| sample == 0.0));
    }
}
