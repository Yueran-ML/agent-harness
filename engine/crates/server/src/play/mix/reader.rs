//! A clip's stereo PCM window, aligned to the media's picture origin. It retains a quarter
//! second on each side for sequential forward and backward blocks, not the whole recording.

use std::ops::Range;
use std::path::{Path, PathBuf};

use columbina_media::{AudioDecoder, AudioFormat, Samples};
use columbina_model::Media;

use super::Stereo;

pub(super) struct Reader {
    path: PathBuf,
    format: AudioFormat,
    decoder: AudioDecoder,
    origin: i64,
    length: i64,
    ahead: i64,
    start: i64,
    pcm: Vec<Stereo>,
    pending: Option<Samples>,
    ended: bool,
}

impl Reader {
    pub(super) fn new(media: &Media, rate: u32) -> Result<Self, String> {
        let path = Path::new(&media.path);
        let decoder = AudioDecoder::open(path, Some(AudioFormat { rate, channels: 2 }))
            .map_err(|error| error.to_string())?;
        let origin = if media.facts.video.is_some() {
            columbina_media::video_start(path).map_err(|error| error.to_string())?
        } else {
            decoder.start()
        };
        let sample = |ticks, scale| {
            i64::try_from(i128::from(ticks) * i128::from(rate) / i128::from(scale))
                .map_err(|_| "media is outside audio sample time".to_owned())
        };
        Ok(Self {
            path: path.to_owned(),
            format: AudioFormat { rate, channels: 2 },
            origin: sample(origin.ticks, origin.scale)?,
            length: sample(media.facts.duration.ticks, media.facts.duration.scale)?,
            decoder,
            ahead: i64::from(rate) / 4,
            start: 0,
            pcm: Vec::new(),
            pending: None,
            ended: false,
        })
    }

    pub(super) fn read(&mut self, range: Range<i64>) -> Result<Vec<Stereo>, String> {
        let frames = usize::try_from(range.end - range.start).map_err(|_| "invalid PCM range")?;
        let mut output = vec![[0.0; 2]; frames];
        let from = range.start.max(0);
        let to = range.end.min(self.length);
        if from >= to {
            return Ok(output);
        }
        let cached_end = self.start + i64::try_from(self.pcm.len()).expect("bounded PCM cache");
        if from < self.start || to > cached_end {
            let start = (from - self.ahead).max(0);
            let end = to.saturating_add(self.ahead).min(self.length);
            let size = usize::try_from(end - start).map_err(|_| "PCM window is too large")?;
            let mut pcm = vec![[0.0; 2]; size];
            let next = if !self.pcm.is_empty() && start >= self.start && start < cached_end {
                let copied = cached_end.min(end) - start;
                let offset = usize::try_from(start - self.start).expect("inside old PCM window");
                let count = usize::try_from(copied).expect("inside new PCM window");
                pcm[..count].copy_from_slice(&self.pcm[offset..offset + count]);
                start + copied
            } else {
                // At media zero, decode from the stream's original continuous sample clock;
                // seeking in a coarse container time base may round the first packet.
                if start == 0 && !self.pcm.is_empty() {
                    // Rewinding to the picture origin must retain the original continuous
                    // sample positions too (Matroska PCM packet timestamps can be rounded).
                    self.decoder = AudioDecoder::open(&self.path, Some(self.format))
                        .map_err(|error| error.to_string())?;
                    self.pending = None;
                    self.ended = false;
                } else if start != 0 {
                    let absolute = start
                        .checked_add(self.origin)
                        .ok_or("audio seek is outside sample time")?;
                    self.decoder
                        .seek(absolute)
                        .map_err(|error| error.to_string())?;
                    self.pending = None;
                    self.ended = false;
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
        output[target..target + count].copy_from_slice(&self.pcm[source..source + count]);
        Ok(output)
    }

    fn fill(
        &mut self,
        start: i64,
        from: i64,
        to: i64,
        output: &mut [Stereo],
    ) -> Result<(), String> {
        while !self.ended {
            if self.pending.is_none() {
                self.pending = self.decoder.next().map_err(|error| error.to_string())?;
                if self.pending.is_none() {
                    self.ended = true;
                    break;
                }
            }
            let samples = self.pending.as_ref().expect("read above");
            let pairs = samples.data.as_chunks::<2>().0;
            let begins = samples
                .start
                .checked_sub(self.origin)
                .ok_or("decoded audio is outside sample time")?;
            let end =
                begins.saturating_add(i64::try_from(pairs.len()).expect("bounded decoder packet"));
            let left = from.max(begins);
            let right = to.min(end);
            if left < right {
                let source = usize::try_from(left - begins).expect("inside packet");
                let target = usize::try_from(left - start).expect("inside window");
                let count = usize::try_from(right - left).expect("positive overlap");
                output[target..target + count].copy_from_slice(&pairs[source..source + count]);
            }
            if end >= to {
                break;
            }
            self.pending = None;
        }
        Ok(())
    }
}
