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
///   sound the best stream of sound, as libav ranks them.
/// - Its duration is that of the picture if it has one, else that of the sound, in the time
///   base of that stream and from its start (see the crate's doc). Where the stream does not
///   say, the duration of the whole file is taken.
/// - The width and height of the picture are as they are stored, before it is turned; its rate
///   is as the crate's doc says; its bits are those of luma; its codec is named as ffprobe
///   names it (`h264`, `hevc`, `prores`); its rotation is how far it is turned clockwise to be
///   shown upright, from its display matrix, to the nearest of 0, 90, 180 and 270.
///
/// # Errors
/// The path is not UTF-8; the file is not there, cannot be read, or is not media; it holds
/// neither picture nor sound; or what it holds cannot be used: a picture or sound that this
/// build of libav cannot decode, no duration, a picture without a size or a rate.
pub fn probe(path: &Path) -> Result<Facts, MediaError> {
    let input = open(path)?;
    let video = video_stream(&input).and_then(|index| input.stream(index));
    let audio = input
        .streams()
        .best(Type::Audio)
        .filter(|stream| ff::decoder::find(stream.parameters().id()).is_some());
    let timed = match (&video, &audio) {
        (Some(video), _) => video,
        (None, Some(audio)) => audio,
        (None, None) => return Err(MediaError::NoStreams),
    };
    let duration = duration(&input, timed)?;
    let video = video.map(|stream| picture(&stream)).transpose()?;
    let audio = audio.map(|stream| sound(&stream)).transpose()?;
    Ok(Facts {
        duration,
        video,
        audio,
    })
}

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
    let input = ff::format::input_with_dictionary(&url, options)
        .map_err(|error| MediaError::Open(error.to_string()))?;
    // The demuxers of pictures drawn in text take any text for one.
    if TEXT_ART.contains(&input.format().name()) {
        return Err(MediaError::Open("it is text, not media".into()));
    }
    Ok(input)
}

/// The demuxers of libav that read pictures drawn in text (ANSI art, Binary Text, `XBin`, `iCE`
/// Draw, Artworx), which take any text for one.
const TEXT_ART: [&str; 5] = ["tty", "bin", "xbin", "idf", "adf"];

/// The index of the picture of a file: the best stream of pictures that libav can decode and
/// that is not the cover of an album.
pub(crate) fn video_stream(input: &Input) -> Option<usize> {
    let usable = |stream: &Stream| {
        stream.parameters().medium() == Type::Video
            && !stream.disposition().contains(Disposition::ATTACHED_PIC)
            && ff::decoder::find(stream.parameters().id()).is_some()
    };
    input
        .streams()
        .best(Type::Video)
        .filter(usable)
        .or_else(|| input.streams().find(usable))
        .map(|stream| stream.index())
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
fn duration(input: &Input, stream: &Stream) -> Result<Time, MediaError> {
    let (num, den) = base(stream)?;
    let scale =
        u32::try_from(den).map_err(|_| MediaError::Unsupported("a time base too fine".into()))?;
    let ticks = if stream.duration() > 0 {
        i128::from(stream.duration()) * i128::from(num)
    } else if input.duration() > 0 {
        // Microseconds, rounded to the nearest tick.
        let over = i128::from(input.duration()) * i128::from(den);
        let under = i128::from(ff::ffi::AV_TIME_BASE);
        (2 * over + under).div_euclid(2 * under)
    } else {
        return Err(MediaError::Unsupported(
            "it does not say how long it lasts".into(),
        ));
    };
    let ticks =
        i64::try_from(ticks).map_err(|_| MediaError::Unsupported("it lasts too long".into()))?;
    Ok(Time { ticks, scale })
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
/// it clockwise. None for a matrix that does not turn a picture.
fn clockwise(matrix: &[i32]) -> Option<i16> {
    let (a, b, d, e) = (
        f64::from(matrix[0]),
        f64::from(matrix[1]),
        f64::from(matrix[3]),
        f64::from(matrix[4]),
    );
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
        // Mirrored and turned: the turn is still read.
        assert_eq!(
            clockwise(&[-ONE, 0, 0, 0, ONE, 0, 0, 0, 1 << 30]),
            Some(180)
        );
        assert_eq!(clockwise(&[0; 9]), None);
    }
}
