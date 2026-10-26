//! What a file holds: how long it lasts, its picture and its sound.

use std::path::Path;

use columbina_model::{AudioFacts, Facts, Rate, Time, VideoFacts};
use ff::format::context::Input;
use ff::format::stream::{Disposition, Stream};
use ff::media::Type;
use ffmpeg_next as ff;

use crate::{ffi, timing, MediaError};

/// What a file holds, as libav finds it. This blocks.
///
/// - The picture is the best stream of pictures that is not the cover of an album, and the
///   sound the best stream of sound that libav can decode, as libav ranks them.
/// - Its duration is that of the picture if it has one, else that of the sound, in the time
///   base of that stream, from the start of the stream (see the crate's doc). Where the file
///   does not say how long the stream lasts, as Matroska does not, the end of the stream is
///   read from its last packets: the end of the last of them, from the start of the stream.
/// - The width and height of the picture are as they are stored, before it is turned; its rate
///   is as the crate's doc says; its bits are those of luma; its codec is named as ffprobe
///   names it (`h264`, `hevc`, `prores`); its rotation is how far it is turned clockwise to be
///   shown upright, from its display matrix, to the nearest of 0, 90, 180 and 270. A display
///   matrix that mirrors the picture is read as a mirror left to right and a turn: the turn
///   is kept, and the mirror is not, since the facts of a file hold none. (The editing of
///   Python keeps the turn as ffprobe gives it, counterclockwise: its 90 is 270 here.)
///
/// # Errors
/// The path is not UTF-8; the file is not there, cannot be read, or is not media of the formats
/// of [`DEMUXERS`]; it holds neither picture nor sound; or what it holds cannot be used: a
/// picture that this build of libav cannot decode, no duration, a picture without a size or a
/// rate.
pub fn probe(path: &Path) -> Result<Facts, MediaError> {
    let mut input = open(path)?;
    let video = video_stream(&input)?;
    let audio = input
        .streams()
        .best(Type::Audio)
        .filter(|stream| ff::decoder::find(stream.parameters().id()).is_some())
        .map(|stream| stream.index());
    let timed = video.or(audio).ok_or(MediaError::NoStreams)?;
    let stream_of = |index: usize| input.stream(index).ok_or(MediaError::NoStreams);
    let video = video.map(|index| picture(&stream_of(index)?)).transpose()?;
    let audio = audio.map(|index| sound(&stream_of(index)?)).transpose()?;
    let duration = duration(&mut input, timed, video.as_ref().map(|video| video.rate))?;
    Ok(Facts {
        duration,
        video,
        audio,
    })
}

/// The demuxers of libav that a file may be read with: those of the editing of Python
/// (`columbina.video.media.DEMUXERS`), and MPEG-PS (`mpeg`, of DVDs and cameras), which
/// that list lacks. A list of other files, such as an HLS playlist or a list of `concat`, is of none of
/// them, so that libav opens no file that such a list names; nor is text, which the demuxers of
/// pictures drawn in text would take for one.
pub const DEMUXERS: [&str; 13] = [
    "mov",
    "matroska",
    "avi",
    "mpegts",
    "mpeg",
    "wav",
    "mp3",
    "flac",
    "ogg",
    "aac",
    "png_pipe",
    "jpeg_pipe",
    "webp_pipe",
];

/// Refuse a path that a project cannot hold, or that libav cannot be given.
pub(crate) fn checked(path: &Path) -> Result<&str, MediaError> {
    let text = path.to_str().ok_or(MediaError::NotUtf8)?;
    if text.contains('\0') {
        return Err(MediaError::NotUtf8);
    }
    Ok(text)
}

/// Open a file with libav, as a file and never as anything else that libav can open.
pub(crate) fn open(path: &Path) -> Result<Input, MediaError> {
    crate::init();
    checked(path)?;
    if !std::fs::metadata(path)?.is_file() {
        return Err(crate::identity::not_a_file());
    }
    // A path is given to libav as a URL of the file protocol, and no other protocol may be
    // used: so that no name is taken for another protocol (`concat:`, `http:`), and no
    // playlist in the file reaches beyond the files of this computer.
    let full = std::path::absolute(path)?;
    let url = format!("file:{}", checked(&full)?);
    let mut options = ff::Dictionary::new();
    options.set("protocol_whitelist", "file");
    options.set("format_whitelist", &DEMUXERS.join(","));
    ff::format::input_with_dictionary(&url, options)
        .map_err(|error| MediaError::Open(error.to_string()))
}

/// The index of the picture of a file, if it has one: the best stream of pictures that is not
/// the cover of an album.
///
/// # Errors
/// This build of libav cannot decode it: then the file cannot be used, rather than be taken
/// for sound alone.
pub(crate) fn video_stream(input: &Input) -> Result<Option<usize>, MediaError> {
    let picture = |stream: &Stream| {
        stream.parameters().medium() == Type::Video
            && !stream.disposition().contains(Disposition::ATTACHED_PIC)
    };
    let Some(stream) = input
        .streams()
        .best(Type::Video)
        .filter(picture)
        .or_else(|| input.streams().find(picture))
    else {
        return Ok(None);
    };
    let codec = stream.parameters().id();
    if ff::decoder::find(codec).is_none() {
        return Err(MediaError::Unsupported(format!(
            "this build of libav cannot decode its picture, of {}",
            codec.name()
        )));
    }
    Ok(Some(stream.index()))
}

/// The rate of the frames of a picture (see the crate's doc).
pub(crate) fn rate(stream: &Stream) -> Option<Rate> {
    let pair = |rational: ff::Rational| (rational.numerator(), rational.denominator());
    timing::rate_of(pair(stream.rate()), pair(stream.avg_frame_rate()))
}

/// The time base of a stream as `(num, den)`, both above 0.
pub(crate) fn base(stream: &Stream) -> Result<(i64, i64), MediaError> {
    let base = stream.time_base();
    let (num, den) = (base.numerator(), base.denominator());
    if num <= 0 || den <= 0 {
        return Err(MediaError::Unsupported(format!(
            "its stream has a time base of {num}/{den}"
        )));
    }
    Ok((num.into(), den.into()))
}

/// How long a stream lasts, in its time base written as the scale `den` (see the crate's doc).
/// `rate` is the rate of its frames, if it is a picture.
fn duration(input: &mut Input, index: usize, rate: Option<Rate>) -> Result<Time, MediaError> {
    let stream = input.stream(index).ok_or(MediaError::NoStreams)?;
    let (num, den) = base(&stream)?;
    let scale =
        u32::try_from(den).map_err(|_| MediaError::Unsupported("a time base too fine".into()))?;
    let said = stream.duration();
    let ticks = if said > 0 {
        said
    } else {
        end_of(input, index, rate)?
    };
    let ticks = i64::try_from(i128::from(ticks) * i128::from(num))
        .map_err(|_| MediaError::Unsupported("it lasts too long".into()))?;
    Ok(Time { ticks, scale })
}

/// How far before the end of a file its last packets are read for, in microseconds.
const END_READ: i64 = 10_000_000;

/// Where a stream ends, from its start, in its time base: the end of its last packet, read
/// from the packets of the last [`END_READ`] of the file (and its key frame before that). The
/// end of a packet is its timestamp and its duration, or a frame of `rate` where it has none.
fn end_of(input: &mut Input, index: usize, rate: Option<Rate>) -> Result<i64, MediaError> {
    let unknown = || MediaError::Unsupported("it does not say how long it lasts".into());
    let (start, (num, den)) = {
        let stream = input.stream(index).ok_or(MediaError::NoStreams)?;
        (
            Some(stream.start_time()).filter(|&start| start != ff::ffi::AV_NOPTS_VALUE),
            base(&stream)?,
        )
    };
    let frame = rate.map_or(0, |rate| {
        // The ticks of a frame, rounded down: `den * rate.den / (num * rate.num)`.
        let ticks =
            i128::from(den) * i128::from(rate.den) / (i128::from(num) * i128::from(rate.num));
        i64::try_from(ticks).unwrap_or(0)
    });
    let total = input.duration();
    if total > 0 {
        // From the start of the file, which the stream's start is near, in the time base.
        let near_end = i128::from(total - END_READ).max(0) * i128::from(den)
            / (i128::from(num) * i128::from(ff::ffi::AV_TIME_BASE));
        let near_end = start
            .unwrap_or(0)
            .saturating_add(i64::try_from(near_end).unwrap_or(0));
        // Where it cannot seek, it reads from where it is: the start.
        let _ = ffi::seek(input, index, near_end, true);
    }
    ffi::discard_others(input, index);
    let mut first = None;
    let mut end: Option<i64> = None;
    loop {
        let mut packet = ff::Packet::empty();
        match packet.read(input) {
            Ok(()) if packet.stream() == index => {
                if let Some(pts) = packet.pts().or(packet.dts()) {
                    let lasts = if packet.duration() > 0 {
                        packet.duration()
                    } else {
                        frame
                    };
                    let ends = pts.saturating_add(lasts);
                    end = Some(end.map_or(ends, |end| end.max(ends)));
                    first = Some(first.map_or(pts, |first: i64| first.min(pts)));
                }
            }
            Ok(()) => {}
            Err(_) => break,
        }
    }
    let end = end.ok_or_else(unknown)?;
    let start = start.or(first).ok_or_else(unknown)?;
    Ok(end.saturating_sub(start).max(0))
}

/// The picture of a file.
fn picture(stream: &Stream) -> Result<VideoFacts, MediaError> {
    let parameters = ffi::parameters(stream);
    let (Ok(width), Ok(height)) = (
        u32::try_from(parameters.width),
        u32::try_from(parameters.height),
    ) else {
        return Err(MediaError::Unsupported("its picture has no size".into()));
    };
    if width == 0 || height == 0 {
        return Err(MediaError::Unsupported("its picture has no size".into()));
    }
    let rate = rate(stream)
        .ok_or_else(|| MediaError::Unsupported("its picture has no rate of frames".into()))?;
    let bits = ffi::depth_of_number(parameters.format)
        .or_else(|| {
            u8::try_from(parameters.bits_per_raw_sample)
                .ok()
                .filter(|&bits| bits > 0)
        })
        .unwrap_or(8);
    Ok(VideoFacts {
        width,
        height,
        rate,
        codec: stream.parameters().id().name().to_owned(),
        bits,
        rotation: rotation(stream),
    })
}

/// The sound of a file.
fn sound(stream: &Stream) -> Result<AudioFacts, MediaError> {
    let parameters = ffi::parameters(stream);
    let rate = u32::try_from(parameters.sample_rate)
        .ok()
        .filter(|&rate| rate > 0)
        .ok_or_else(|| MediaError::Unsupported("its sound has no rate of samples".into()))?;
    let channels = u16::try_from(parameters.channels)
        .ok()
        .filter(|&channels| channels > 0)
        .ok_or_else(|| MediaError::Unsupported("its sound has no channels".into()))?;
    Ok(AudioFacts {
        rate,
        channels,
        codec: stream.parameters().id().name().to_owned(),
    })
}

/// How far the picture of a stream is turned clockwise to be shown upright, by its display
/// matrix: 0, 90, 180 or 270.
fn rotation(stream: &Stream) -> i16 {
    stream
        .side_data()
        .find(|data| data.kind() == ff::codec::packet::side_data::Type::DisplayMatrix)
        .and_then(|data| {
            let words: Vec<i32> = data
                .data()
                .as_chunks::<4>()
                .0
                .iter()
                .map(|word| i32::from_ne_bytes(*word))
                .collect();
            clockwise(words.get(..9)?)
        })
        .unwrap_or(0)
}

/// How far a display matrix turns a picture clockwise, to the nearest quarter turn.
///
/// The matrix turns it counterclockwise by `av_display_rotation_get`, whose reckoning this is:
/// `atan2(b / |(b, e)|, a / |(a, d)|)` of the matrix `[a b u; d e v; x y w]` is how far it turns
/// it clockwise. A matrix that mirrors the picture (whose determinant is below 0) is read as a
/// mirror left to right, as `av_display_matrix_flip` makes it by turning the signs of `a` and
/// `d`, and a turn: those signs are turned back, and the turn is given. None for a matrix that
/// does not turn a picture.
fn clockwise(matrix: &[i32]) -> Option<i16> {
    let (mut a, b, mut d, e) = (
        f64::from(matrix[0]),
        f64::from(matrix[1]),
        f64::from(matrix[3]),
        f64::from(matrix[4]),
    );
    if a * e - b * d < 0.0 {
        (a, d) = (-a, -d);
    }
    let (across, down) = (a.hypot(d), b.hypot(e));
    if across == 0.0 || down == 0.0 {
        return None;
    }
    let degrees = (b / down).atan2(a / across).to_degrees();
    // Within ±180 degrees, so a whole number of quarter turns between -2 and 2.
    #[allow(clippy::cast_possible_truncation)]
    let quarters = (degrees / 90.0).round() as i16;
    Some(quarters.rem_euclid(4) * 90)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONE: i32 = 1 << 16;

    #[test]
    fn a_matrix_that_ffmpeg_writes_for_a_display_rotation_of_90_turns_clockwise_by_270() {
        // `-display_rotation 90`, a quarter turn counterclockwise, as ffprobe shows it.
        let matrix = [0, -ONE, 0, ONE, 0, 0, 0, 0, 1 << 30];
        assert_eq!(clockwise(&matrix), Some(270));
    }

    #[test]
    fn the_other_matrices_turn_as_they_say() {
        assert_eq!(clockwise(&[ONE, 0, 0, 0, ONE, 0, 0, 0, 1 << 30]), Some(0));
        assert_eq!(clockwise(&[0, ONE, 0, -ONE, 0, 0, 0, 0, 1 << 30]), Some(90));
        assert_eq!(
            clockwise(&[-ONE, 0, 0, 0, -ONE, 0, 0, 0, 1 << 30]),
            Some(180)
        );
        // Mirrored left to right, and not turned: no turn, not a half turn.
        assert_eq!(clockwise(&[-ONE, 0, 0, 0, ONE, 0, 0, 0, 1 << 30]), Some(0));
        // Mirrored upside down: a mirror left to right and a half turn.
        assert_eq!(
            clockwise(&[ONE, 0, 0, 0, -ONE, 0, 0, 0, 1 << 30]),
            Some(180)
        );
        // Turned a quarter clockwise, then mirrored left to right.
        assert_eq!(clockwise(&[0, ONE, 0, ONE, 0, 0, 0, 0, 1 << 30]), Some(90));
        assert_eq!(clockwise(&[0; 9]), None);
    }
}
