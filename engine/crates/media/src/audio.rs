//! The sound of a file: its samples, decoded, and converted with swresample to 32-bit floats at
//! the rate and with the channels that they are played with.

use std::collections::VecDeque;
use std::path::Path;

use ff::format::context::Input;
use ffmpeg_next as ff;

use crate::ffi::{self, ChannelLayout, Resampler};
use crate::{probe, MediaError};

/// How many frames or packets of sound in a row may fail to decode, each passed over, before
/// decoding is given up.
const MOST_FAILURES: u32 = 16;

/// The greatest common divisor of two numbers above 0.
fn gcd(mut a: i128, mut b: i128) -> i128 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// The fewest samples of the stream that are decoded and dropped before the sample sought.
const LEAST_PREROLL: i64 = 2048;

/// The most silence that is put in a gap of the sound, or before its first sample after a
/// seek, so that a broken timestamp does not make hours of it: 10 s.
const MOST_SILENCE_SECONDS: i64 = 10;

/// How samples are given: their rate, and how many channels each has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioFormat {
    /// Samples a second, of each channel.
    pub rate: u32,
    /// How many channels: 1 is mono, 2 stereo (left, right), more in the default order of
    /// libav for that many (5.1 is left, right, centre, low frequencies, side left, side right).
    pub channels: u16,
}

/// Samples of sound, one after the other from `start`: 32-bit floats from -1 to 1, interleaved
/// (each instant has a float for each channel, in order).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Samples {
    /// The number of the first, at the rate of the [`AudioFormat`], counted from time 0 of the
    /// timestamps of the file (see [`AudioDecoder`]).
    pub start: i64,
    /// The floats, `channels` an instant.
    pub data: Vec<f32>,
}

impl Samples {
    /// How many instants they hold, of `channels` floats each.
    #[must_use]
    pub fn len(&self, channels: u16) -> usize {
        self.data.len() / usize::from(channels.max(1))
    }
}

/// The time that the sound of a file starts at: that of the best stream of sound, which
/// [`AudioDecoder::open`] opens, as [`AudioDecoder::start`] gives it, without opening its
/// decoder. The sound of the other streams of the file is placed by it, as the sound of a file
/// with a picture is placed by [`crate::video_start`].
///
/// # Errors
/// The file cannot be opened (as [`crate::probe`]), or it has no sound that this build of libav
/// can decode ([`MediaError::NoStreams`]).
pub fn sound_start(path: &Path) -> Result<columbina_model::Time, MediaError> {
    let input = probe::open(path)?;
    start_of_sound(&input)
}

/// The time that the best stream of sound of `input` starts at, as [`sound_start`] says.
fn start_of_sound(input: &Input) -> Result<columbina_model::Time, MediaError> {
    let stream = probe::sound_stream(input).ok_or(MediaError::NoStreams)?;
    let found = input.stream(stream).ok_or(MediaError::NoStreams)?;
    let (num, den) = probe::base(&found)?;
    Ok(time_of(first_tick_of(&found), (num, den)))
}

/// The tick that a stream starts at, or 0 where the file does not say.
fn first_tick_of(stream: &ff::format::stream::Stream) -> i64 {
    Some(stream.start_time())
        .filter(|&start| start != ff::ffi::AV_NOPTS_VALUE)
        .unwrap_or(0)
}

/// A tick of a time base `num/den` as a time of the model.
fn time_of(tick: i64, (num, den): (i64, i64)) -> columbina_model::Time {
    columbina_model::Time {
        ticks: tick.saturating_mul(num),
        scale: u32::try_from(den).unwrap_or(u32::MAX),
    }
}

/// The next packet of stream `stream` of `input`; none at the end of the file.
fn read_packet(input: &mut Input, stream: usize) -> Result<Option<ff::Packet>, MediaError> {
    loop {
        let mut packet = ff::Packet::empty();
        match packet.read(input) {
            Ok(()) if packet.stream() == stream => return Ok(Some(packet)),
            Ok(()) => {}
            Err(ff::Error::Eof) => return Ok(None),
            Err(error) => {
                return Err(MediaError::Read(format!(
                    "the file cannot be read on: {error}"
                )))
            }
        }
    }
}

/// Seek `input` so that the packets read next are of `stream` at or before `tick`, or failing
/// that from its start.
fn seek_input(input: &mut Input, stream: usize, tick: i64, first: i64) -> Result<(), MediaError> {
    if ffi::seek(input, stream, tick, true).is_err() {
        // Before every key frame of the stream: from its start.
        ffi::seek(input, stream, first, true)
            .or_else(|_| ffi::seek_to_byte(input, 0))
            .map_err(MediaError::Seek)?;
    }
    Ok(())
}

/// The decoder of the sound of a file.
///
/// It gives the samples of a stream of sound in the file, the best as libav ranks them or one
/// chosen by its index ([`AudioDecoder::open_stream`]), one run after another, with no gap and
/// no overlap between them: where the timestamps of the file leave a gap, it is filled with
/// silence, and where they overlap, the samples that would come again are dropped. A sample is
/// numbered by its time in the file's timestamps, from time 0 of the file (the time that the
/// streams of a file share, not the start of this stream), at the rate it is given at: the
/// sample at `t` seconds is `round(t × rate)`. So the sound of a
/// picture's frame `n` is at `(start + n / rate_of_frames) × rate`, where `start` is
/// [`crate::VideoDecoder::start`].
///
/// After [`AudioDecoder::seek`] to a sample, the first sample given is that sample: those
/// before it are dropped, and silence comes first if the sound starts after it. The decoder
/// starts some way before it (a pre-roll of at least 2048 samples of the stream, two frames of
/// AAC, or what the stream asks for), so that a codec whose frames overlap the ones before them
/// (AAC, Opus, MP3) gives the samples sought as it would have decoding from the start.
pub struct AudioDecoder {
    input: Input,
    track: Track,
}

/// A stream of sound that is decoded, and where its samples are placed: what an
/// [`AudioDecoder`] decodes, and each stream of [`AudioStreams`].
struct Track {
    stream: usize,
    decoder: ff::decoder::Audio,
    /// The time base of the stream, `num/den` seconds a tick.
    base: (i64, i64),
    /// The tick that the stream starts at, or 0 where the file does not say.
    first_tick: i64,
    /// How many samples, at the rate given, are decoded and dropped before one sought.
    preroll: i64,
    source: AudioFormat,
    format: AudioFormat,
    /// The layout that the samples are given in: the default layout of `format.channels`, or,
    /// unmixed, the layout of the stream, as its parameters say.
    layout: ChannelLayout,
    /// The converter of the frames that come, made for the first of them, and again for one of
    /// another format.
    resampler: Option<Resampler>,
    /// The number of the next sample to give, once the first after a seek was given.
    next: Option<i64>,
    /// The sample sought, until the first is given after it.
    sought: Option<i64>,
    /// Whether the end of the file was sent to the decoder.
    sent_end: bool,
    /// Whether the decoder and the converter have given everything.
    ended: bool,
    failures: u32,
}

impl AudioDecoder {
    /// Open the sound of a file, to be given in `format`, or as the stream has it (its rate and
    /// its channels) if none is given: the best stream of sound that this build of libav can
    /// decode, as libav ranks them.
    ///
    /// # Errors
    /// The file cannot be opened (as [`crate::probe`]), it has no sound that this build of
    /// libav can decode ([`MediaError::NoStreams`]), or its decoder cannot be opened.
    pub fn open(path: &Path, format: Option<AudioFormat>) -> Result<Self, MediaError> {
        let input = probe::open(path)?;
        let stream = probe::sound_stream(&input).ok_or(MediaError::NoStreams)?;
        Self::opened(input, stream, format, false)
    }

    /// The decoder of stream `stream` of `input`, a stream of sound that can be decoded.
    fn opened(
        mut input: Input,
        stream: usize,
        format: Option<AudioFormat>,
        unmixed: bool,
    ) -> Result<Self, MediaError> {
        let track = Track::open(&input, stream, format, unmixed)?;
        ffi::discard_others(&mut input, stream);
        Ok(Self { input, track })
    }

    /// Open a stream of sound of a file, by its index among all the streams of the file (as
    /// libav and the `ffmpeg` of the command line number them, and [`crate::sounds`] lists
    /// them), to be given as [`AudioDecoder::open`] gives the best one.
    ///
    /// # Errors
    /// As [`AudioDecoder::open`]; [`MediaError::NoStreams`] where the file has no such stream,
    /// or it is not sound that this build of libav can decode.
    pub fn open_stream(
        path: &Path,
        stream: usize,
        format: Option<AudioFormat>,
    ) -> Result<Self, MediaError> {
        let input = probe::open(path)?;
        let stream = probe::decodable_sound(&input, stream).ok_or(MediaError::NoStreams)?;
        Self::opened(input, stream, format, false)
    }

    /// Open a stream of sound of a file, by its index (as [`AudioDecoder::open_stream`]), to be
    /// given with its own channels, in its own order, none mixed into another: only the format
    /// of its samples is converted, and their rate to `rate` if one is given. A stream whose
    /// channels are not laid out as libav lays out as many by default (3.0, L R C, where the
    /// default of three is 2.1, L R LFE; or a layout of its own) is given as it is, where
    /// [`AudioDecoder::open`] mixes it into the default layout of as many channels. The layout
    /// is the one that the stream says it has; should the frames change to another within the
    /// stream, they are mixed into it.
    ///
    /// # Errors
    /// As [`AudioDecoder::open_stream`].
    pub fn open_unmixed(path: &Path, stream: usize, rate: Option<u32>) -> Result<Self, MediaError> {
        let input = probe::open(path)?;
        let stream = probe::decodable_sound(&input, stream).ok_or(MediaError::NoStreams)?;
        Self::opened(
            input,
            stream,
            rate.map(|rate| AudioFormat { rate, channels: 0 }),
            true,
        )
    }

    /// The rate and channels that the samples are given in.
    #[must_use]
    pub fn format(&self) -> AudioFormat {
        self.track.format
    }

    /// The rate and channels of the stream, as the file says.
    #[must_use]
    pub fn source(&self) -> AudioFormat {
        self.track.source
    }

    /// The time that the stream starts at, in the timestamps of the file, from time 0 of the
    /// file: `ticks / scale` seconds (0 where the file does not say). The duration of media
    /// that is sound alone counts from there.
    #[must_use]
    pub fn start(&self) -> columbina_model::Time {
        time_of(self.track.first_tick, self.track.base)
    }

    /// Go to sample `sample` (see [`AudioDecoder`]): the next samples given start there.
    ///
    /// # Errors
    /// The file cannot be sought in.
    pub fn seek(&mut self, sample: i64) -> Result<(), MediaError> {
        let track = &mut self.track;
        let tick = track.tick_before(sample, track.preroll);
        seek_input(&mut self.input, track.stream, tick, track.first_tick)?;
        track.reset(sample);
        Ok(())
    }

    /// The next samples; none at the end of the sound.
    ///
    /// # Errors
    /// The sound cannot be decoded or converted, or the file cannot be read on.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<Option<Samples>, MediaError> {
        let input = &mut self.input;
        let stream = self.track.stream;
        self.track.next(&mut || read_packet(input, stream))
    }
}

/// How much earlier than its pre-roll [`AudioStreams`] seeks: a stream may lie a little behind
/// the one that the file is sought by.
const STREAMS_MARGIN_SECONDS_DIVISOR: i64 = 2;

/// The decoder of several streams of sound of a file, each with its own channels, from one
/// reading of the file: what each gives is what an [`AudioDecoder`] opened on it with
/// [`AudioDecoder::open_unmixed`] gives, but the file is read once for all of them, and sought
/// once. A packet read for one of them that belongs to another is kept for that one until it
/// asks for it, so they are best read side by side.
pub struct AudioStreams {
    input: Input,
    tracks: Vec<Track>,
    /// The packets read for one stream and kept for another, a queue for each.
    kept: Vec<VecDeque<ff::Packet>>,
    start: columbina_model::Time,
}

impl AudioStreams {
    /// Open the streams of a file by their indices, as [`AudioDecoder::open_unmixed`] opens
    /// one. An index that is not of a stream of sound that this build of libav can decode is
    /// passed over: [`AudioStreams::place`] finds none for it.
    ///
    /// # Errors
    /// The file cannot be opened (as [`crate::probe`]), or the decoder of a stream cannot be.
    pub fn open_unmixed(
        path: &Path,
        streams: &[usize],
        rate: Option<u32>,
    ) -> Result<Self, MediaError> {
        let mut input = probe::open(path)?;
        let mut indices: Vec<usize> = streams
            .iter()
            .filter_map(|&index| probe::decodable_sound(&input, index))
            .collect();
        indices.sort_unstable();
        indices.dedup();
        let format = rate.map(|rate| AudioFormat { rate, channels: 0 });
        let tracks = indices
            .iter()
            .map(|&index| Track::open(&input, index, format, true))
            .collect::<Result<Vec<_>, _>>()?;
        let start = start_of_sound(&input).unwrap_or(columbina_model::Time { ticks: 0, scale: 1 });
        ffi::discard_all_but(&mut input, &indices);
        Ok(Self {
            input,
            kept: tracks.iter().map(|_| VecDeque::new()).collect(),
            tracks,
            start,
        })
    }

    /// Where stream `stream` of the file is among those opened, if it was.
    #[must_use]
    pub fn place(&self, stream: usize) -> Option<usize> {
        self.tracks.iter().position(|track| track.stream == stream)
    }

    /// How many streams were opened.
    #[must_use]
    pub fn len(&self) -> usize {
        self.tracks.len()
    }

    /// Whether none was.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }

    /// The rate and channels that the samples of the stream at `place` are given in.
    #[must_use]
    pub fn format(&self, place: usize) -> AudioFormat {
        self.tracks[place].format
    }

    /// The time that the sound of the file starts at, as [`sound_start`] says (time 0 where it
    /// has no best stream of sound): the streams are placed by it.
    #[must_use]
    pub fn start(&self) -> columbina_model::Time {
        self.start
    }

    /// Go to sample `sample` in every stream: the next samples of each start there. The file
    /// is sought once, by the first stream, half a second before the pre-roll of the stream
    /// that needs the longest.
    ///
    /// # Errors
    /// The file cannot be sought in.
    pub fn seek(&mut self, sample: i64) -> Result<(), MediaError> {
        let Some(first) = self.tracks.first() else {
            return Ok(());
        };
        let before = self
            .tracks
            .iter()
            .map(|track| track.preroll)
            .max()
            .unwrap_or(0)
            .saturating_add(i64::from(first.format.rate) / STREAMS_MARGIN_SECONDS_DIVISOR);
        let tick = first.tick_before(sample, before);
        seek_input(&mut self.input, first.stream, tick, first.first_tick)?;
        for (track, kept) in self.tracks.iter_mut().zip(&mut self.kept) {
            track.reset(sample);
            kept.clear();
        }
        Ok(())
    }

    /// The next samples of the stream at `place`; none at the end of its sound.
    ///
    /// # Errors
    /// The sound cannot be decoded or converted, or the file cannot be read on.
    pub fn next(&mut self, place: usize) -> Result<Option<Samples>, MediaError> {
        let Self {
            input,
            tracks,
            kept,
            ..
        } = self;
        let indices: Vec<usize> = tracks.iter().map(|track| track.stream).collect();
        let mut pull = || -> Result<Option<ff::Packet>, MediaError> {
            if let Some(packet) = kept[place].pop_front() {
                return Ok(Some(packet));
            }
            loop {
                let mut packet = ff::Packet::empty();
                match packet.read(input) {
                    Ok(()) => match indices.iter().position(|&index| index == packet.stream()) {
                        Some(found) if found == place => return Ok(Some(packet)),
                        Some(found) => kept[found].push_back(packet),
                        None => {}
                    },
                    Err(ff::Error::Eof) => return Ok(None),
                    Err(error) => {
                        return Err(MediaError::Read(format!(
                            "the file cannot be read on: {error}"
                        )))
                    }
                }
            }
        };
        tracks[place].next(&mut pull)
    }
}

impl Track {
    /// The decoder of stream `stream` of `input`, a stream of sound that can be decoded, giving
    /// `format`; unmixed, the channels of `format` are those of the stream, laid out as it
    /// says.
    fn open(
        input: &Input,
        stream: usize,
        format: Option<AudioFormat>,
        unmixed: bool,
    ) -> Result<Self, MediaError> {
        let found = input.stream(stream).ok_or(MediaError::NoStreams)?;
        let (base, first_tick, preroll, decoder, source) = {
            let base = probe::base(&found)?;
            let first_tick = first_tick_of(&found);
            let parameters = ffi::parameters(&found);
            let source = AudioFormat {
                rate: u32::try_from(parameters.sample_rate)
                    .ok()
                    .filter(|&rate| rate > 0)
                    .ok_or_else(|| MediaError::Unsupported("its sound has no rate".into()))?,
                channels: u16::try_from(parameters.channels)
                    .ok()
                    .filter(|&channels| channels > 0)
                    .ok_or_else(|| MediaError::Unsupported("its sound has no channels".into()))?,
            };
            let preroll = i64::from(parameters.frame_size.max(0))
                .saturating_mul(2)
                .max(i64::from(parameters.seek_preroll.max(0)))
                .max(LEAST_PREROLL);
            let context = ff::codec::Context::from_parameters(found.parameters())
                .map_err(|error| MediaError::Open(error.to_string()))?;
            let decoder = context.decoder().audio().map_err(|error| {
                MediaError::Decode(format!(
                    "the decoder of its sound cannot be opened: {error}"
                ))
            })?;
            (base, first_tick, preroll, decoder, source)
        };
        let mut format = format.unwrap_or(source);
        if unmixed {
            format.channels = source.channels;
        }
        if format.rate == 0 || format.channels == 0 {
            return Err(MediaError::Unsupported(
                "sound is given at a rate and with channels above 0".into(),
            ));
        }
        let layout = if unmixed {
            ChannelLayout::of_stream(&found).map_err(MediaError::Decode)?
        } else {
            ChannelLayout::default(format.channels)
        };
        // The pre-roll at the rate given, rounded up.
        let preroll = (u128::from(preroll.unsigned_abs()) * u128::from(format.rate))
            .div_ceil(u128::from(source.rate))
            .try_into()
            .unwrap_or(i64::MAX);
        Ok(Self {
            stream,
            decoder,
            base,
            first_tick,
            preroll,
            source,
            format,
            layout,
            resampler: None,
            next: None,
            sought: None,
            sent_end: false,
            ended: false,
            failures: 0,
        })
    }

    /// The tick of the stream at or before `before` samples before `sample`.
    fn tick_before(&self, sample: i64, before: i64) -> i64 {
        let (num, den) = self.base;
        // In ticks of num / den s.
        let from = sample.saturating_sub(before);
        let over = i128::from(from) * i128::from(den);
        let under = i128::from(num) * i128::from(self.format.rate);
        i64::try_from(over.div_euclid(under)).unwrap_or(i64::MAX)
    }

    /// Start afresh after the file was sought for sample `sample`.
    fn reset(&mut self, sample: i64) {
        self.decoder.flush();
        self.resampler = None;
        self.next = None;
        self.sought = Some(sample);
        self.sent_end = false;
        self.ended = false;
        self.failures = 0;
    }

    /// The next samples, from the packets that `pull` gives; none at the end of the sound.
    fn next(
        &mut self,
        pull: &mut dyn FnMut() -> Result<Option<ff::Packet>, MediaError>,
    ) -> Result<Option<Samples>, MediaError> {
        loop {
            if self.ended {
                return Ok(None);
            }
            let mut frame = ff::frame::Audio::empty();
            match self.decoder.receive_frame(&mut frame) {
                Ok(()) => {
                    self.failures = 0;
                    let tick = frame.timestamp().or(frame.pts());
                    let fresh = !self
                        .resampler
                        .as_ref()
                        .is_some_and(|resampler| resampler.takes(&frame));
                    let (skip, at) = match tick {
                        Some(tick) if fresh => self.aligned(&frame, tick),
                        Some(tick) => (0, Some(self.sample_of(tick))),
                        None => (0, None),
                    };
                    if skip >= frame.samples() {
                        continue;
                    }
                    let mut data = Vec::new();
                    self.convert(Some((&frame, skip)), &mut data)?;
                    if let Some(samples) = self.place(at, data) {
                        return Ok(Some(samples));
                    }
                }
                Err(ff::Error::Other { errno }) if errno == ff::util::error::EAGAIN => {
                    self.feed(pull)?;
                }
                Err(ff::Error::Eof) => {
                    self.ended = true;
                    let mut data = Vec::new();
                    if self.resampler.is_some() {
                        self.convert(None, &mut data)?;
                    }
                    return Ok(self.place(None, data));
                }
                Err(error) => self.failed(error)?,
            }
        }
    }

    /// The number of the sample at a tick of the stream, rounded to the nearest, half up.
    fn sample_of(&self, tick: i64) -> i64 {
        let (num, den) = self.base;
        let over = i128::from(tick) * i128::from(num) * i128::from(self.format.rate);
        let under = i128::from(den);
        let rounded = (2 * over + under).div_euclid(2 * under);
        i64::try_from(rounded).unwrap_or(if rounded < 0 { i64::MIN } else { i64::MAX })
    }

    /// Where a converter made afresh starts on a frame at `tick`: how many of its first samples
    /// it passes over, and the sample given (at the rate given) of the first it takes.
    ///
    /// A converter to another rate lays the samples it makes on a grid that starts at its first
    /// sample. So that the grid is the same whichever frame it starts on (after a seek, or from
    /// the start of the file), it starts on a sample of the stream whose time is a whole number
    /// of samples at both rates: one every `in / gcd(in, out)` samples, counted from time 0 of
    /// the file. What it makes differs from what it would have made going on from before only
    /// for the first few samples, which the pre-roll drops.
    fn aligned(&self, frame: &ff::frame::Audio, tick: i64) -> (usize, Option<i64>) {
        let (num, den) = self.base;
        let input = i128::from(frame.rate());
        let output = i128::from(self.format.rate);
        if input <= 0 || input == output {
            return (0, Some(self.sample_of(tick)));
        }
        // The sample of the stream at the tick, rounded to the nearest.
        let over = i128::from(tick) * i128::from(num) * input;
        let under = i128::from(den);
        let first = (2 * over + under).div_euclid(2 * under);
        let step = input / gcd(input, output);
        let skip = (step - first.rem_euclid(step)) % step;
        let at = (first + skip) * output / input;
        (
            usize::try_from(skip).unwrap_or(usize::MAX),
            i64::try_from(at).ok(),
        )
    }

    /// Convert a frame, passing over its first samples, or what the converter holds back,
    /// adding the samples to `data`.
    fn convert(
        &mut self,
        frame: Option<(&ff::frame::Audio, usize)>,
        data: &mut Vec<f32>,
    ) -> Result<(), MediaError> {
        if let Some((frame, _)) = frame {
            if !self
                .resampler
                .as_ref()
                .is_some_and(|resampler| resampler.takes(frame))
            {
                // What the old one holds back is let go: the format changed mid-stream.
                self.resampler = Some(
                    Resampler::new(frame, self.format.rate, &self.layout)
                        .map_err(MediaError::Decode)?,
                );
            }
        }
        match &mut self.resampler {
            Some(resampler) => resampler.convert(frame, data).map_err(MediaError::Decode),
            None => Ok(()),
        }
    }

    /// Place samples that were decoded at sample `at` (if their frame had a timestamp) after
    /// those given before: cut to start at the sample sought, and laid after the last given,
    /// with silence in a gap and the overlap dropped. None if nothing of them is left.
    fn place(&mut self, at: Option<i64>, mut data: Vec<f32>) -> Option<Samples> {
        let channels = usize::from(self.format.channels);
        let rate = i64::from(self.format.rate);
        // Within 10 ms of where it is due, a run follows the one before it: the converter
        // gives its samples a little late, and timestamps are rounded.
        let close = (rate / 100).max(2);
        let start = match (self.next, at) {
            (Some(next), Some(at)) if (at - next).abs() > close => at,
            (Some(next), _) => next,
            (None, Some(at)) => at,
            (None, None) => self.sought.unwrap_or(0),
        };
        // Where the samples given go on from: the sample sought, or the end of the last run.
        let from = self.sought.or(self.next).unwrap_or(start);
        // The run is where it is, whatever of it is given: so that the runs after a seek that
        // are dropped (the pre-roll) are counted from the first, as the converter gives them.
        let length = i64::try_from(data.len() / channels).unwrap_or(i64::MAX);
        self.next = Some(start.saturating_add(length));
        let most = MOST_SILENCE_SECONDS * rate;
        let start = if start > from && start - from <= most {
            // A gap: silence up to `start`.
            let silence = usize::try_from(start - from).unwrap_or(0) * channels;
            data.splice(0..0, std::iter::repeat_n(0.0, silence));
            from
        } else if start < from {
            // An overlap, or what comes before the sample sought: dropped.
            let drop = usize::try_from(from - start)
                .unwrap_or(usize::MAX)
                .saturating_mul(channels)
                .min(data.len());
            data.drain(..drop);
            from
        } else {
            start
        };
        if data.is_empty() {
            return None;
        }
        self.sought = None;
        Some(Samples { start, data })
    }

    /// Pass over a frame or a packet that failed to decode, unless too many failed in a row.
    fn failed(&mut self, error: ff::Error) -> Result<(), MediaError> {
        self.failures += 1;
        if self.failures <= MOST_FAILURES {
            return Ok(());
        }
        Err(MediaError::Decode(format!(
            "{MOST_FAILURES} frames of sound in a row failed, the last with: {error}"
        )))
    }

    /// Send the decoder the next packet of the sound that `pull` gives, or the end of the file.
    fn feed(
        &mut self,
        pull: &mut dyn FnMut() -> Result<Option<ff::Packet>, MediaError>,
    ) -> Result<(), MediaError> {
        if self.sent_end {
            return Err(MediaError::Decode(
                "the decoder of the sound wants more after the end of the file".into(),
            ));
        }
        loop {
            let Some(packet) = pull()? else {
                self.sent_end = true;
                return self
                    .decoder
                    .send_eof()
                    .map_err(|error| MediaError::Decode(error.to_string()));
            };
            match self.decoder.send_packet(&packet) {
                Ok(()) => return Ok(()),
                Err(error) => self.failed(error)?,
            }
        }
    }
}
