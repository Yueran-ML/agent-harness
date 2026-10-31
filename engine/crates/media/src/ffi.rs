//! Every unsafe call of the crate: the devices of the graphics card and the decoders that use
//! them, the copying of a frame from the card into memory of a picture, converting a frame into
//! it and a picture into a frame, converting sound, references to frames, seeking, the index of
//! a stream, and the fields of libav that `ffmpeg-next` has no safe way to reach.

use std::ffi::{c_int, c_void, CStr, CString};

use ff::ffi::{
    av_buffer_create, av_buffer_ref, av_buffer_unref, av_channel_layout_check,
    av_channel_layout_compare, av_channel_layout_copy, av_channel_layout_default,
    av_channel_layout_uninit, av_frame_alloc, av_frame_free, av_frame_make_writable, av_frame_ref,
    av_frame_unref, av_hwdevice_ctx_create, av_hwdevice_find_type_by_name,
    av_hwframe_transfer_data, av_pix_fmt_desc_get, av_read_image_line2, av_seek_frame, av_strerror,
    avcodec_default_get_format, avcodec_get_hw_config, avformat_index_get_entries_count,
    avformat_index_get_entry, avformat_index_get_entry_from_timestamp, swr_alloc_set_opts2,
    swr_convert, swr_free, swr_get_out_samples, swr_init, sws_scale, AVBufferRef, AVChannelLayout,
    AVChannelOrder, AVCodecContext, AVColorRange, AVDiscard, AVHWDeviceType, AVHWFramesContext,
    AVPixelFormat, AVSampleFormat, SwrContext, AVINDEX_DISCARD_FRAME, AVSEEK_FLAG_BACKWARD,
    AVSEEK_FLAG_BYTE, AV_CODEC_HW_CONFIG_METHOD_HW_DEVICE_CTX, AV_PIX_FMT_FLAG_ALPHA,
    AV_PIX_FMT_FLAG_BAYER, AV_PIX_FMT_FLAG_BE, AV_PIX_FMT_FLAG_FLOAT, AV_PIX_FMT_FLAG_HWACCEL,
    AV_PIX_FMT_FLAG_PAL, AV_PIX_FMT_FLAG_PLANAR, AV_PIX_FMT_FLAG_RGB, FF_THREAD_FRAME,
    FF_THREAD_SLICE,
};
use ffmpeg_next as ff;

use crate::Picture;

/// The text of an error code of libav.
fn error_text(code: c_int) -> String {
    let mut text = [0 as std::ffi::c_char; 256];
    // SAFETY: the buffer is as long as it is said to be, and av_strerror ends what it writes
    // in it with a NUL, even when it knows no text for the code.
    unsafe {
        av_strerror(code, text.as_mut_ptr(), text.len());
        CStr::from_ptr(text.as_ptr()).to_string_lossy().into_owned()
    }
}

/// A device of the graphics card, as libav holds it.
pub(crate) struct Device {
    buffer: *mut AVBufferRef,
    kind: AVHWDeviceType,
}

// SAFETY: the device is a counted reference that is only ever let go and counted again, and
// libav counts its references atomically; the device itself may be used from any thread, as
// libav locks what it must.
unsafe impl Send for Device {}

impl Drop for Device {
    fn drop(&mut self) {
        // SAFETY: the reference was made by av_hwdevice_ctx_create and is let go only here.
        unsafe { av_buffer_unref(&raw mut self.buffer) };
    }
}

impl Device {
    /// Make a device of the kind that libav names `name` (`cuda`, `d3d11va`, `videotoolbox`),
    /// on the first card there is.
    pub fn new(name: &str) -> Result<Self, String> {
        let name = CString::new(name).map_err(|_| "a name with a NUL".to_owned())?;
        // SAFETY: the name is a string that ends in a NUL; the type it gives is only compared.
        let kind = unsafe { av_hwdevice_find_type_by_name(name.as_ptr()) };
        if kind == AVHWDeviceType::AV_HWDEVICE_TYPE_NONE {
            return Err("this build of libav has no such device".to_owned());
        }
        let mut buffer = std::ptr::null_mut();
        // SAFETY: `buffer` is where the reference is written; no name of a device and no
        // options are given, so libav takes the first device of the kind.
        let code = unsafe {
            av_hwdevice_ctx_create(
                &raw mut buffer,
                kind,
                std::ptr::null(),
                std::ptr::null_mut(),
                0,
            )
        };
        if code < 0 {
            return Err(format!("the device cannot be made: {}", error_text(code)));
        }
        Ok(Self { buffer, kind })
    }

    /// The format of the frames that `decoder` gives on this device, if it decodes on it.
    pub fn format_of(&self, decoder: &ff::Codec) -> Option<ff::format::Pixel> {
        // SAFETY: a decoder of libav lives as long as the program; avcodec_get_hw_config gives
        // each of its configurations in turn, then null.
        unsafe {
            let decoder = decoder.as_ptr();
            for index in 0.. {
                let config = avcodec_get_hw_config(decoder, index);
                if config.is_null() {
                    return None;
                }
                let methods = (*config).methods;
                if methods & AV_CODEC_HW_CONFIG_METHOD_HW_DEVICE_CTX as c_int != 0
                    && (*config).device_type == self.kind
                {
                    return Some((*config).pix_fmt.into());
                }
            }
            None
        }
    }

    /// Make a decoder that is not open yet decode on this device, to frames of `format`, and
    /// keep `extra` frames on the card beyond those that it needs, for those that are held.
    pub fn attach(
        &self,
        context: &mut ff::codec::Context,
        format: ff::format::Pixel,
        extra: i32,
    ) -> Result<(), String> {
        let format: AVPixelFormat = format.into();
        // SAFETY: the context is not open, so these may be set; the context takes a reference
        // of its own to the device, and lets it go when it is freed. `opaque` is not used by
        // ffmpeg-next, and carries the format for `choose_format`.
        unsafe {
            let reference = av_buffer_ref(self.buffer);
            if reference.is_null() {
                return Err("the device cannot be referred to: no memory".to_owned());
            }
            let context = context.as_mut_ptr();
            (*context).hw_device_ctx = reference;
            (*context).opaque = format as c_int as isize as *mut c_void;
            (*context).get_format = Some(choose_format);
            (*context).extra_hw_frames = extra;
        }
        Ok(())
    }
}

/// The format that a decoder chooses among those that it offers: the one of the device that it
/// was given, if it is offered, else the first that is not of a device, as libav would choose.
unsafe extern "C" fn choose_format(
    context: *mut AVCodecContext,
    offered: *const AVPixelFormat,
) -> AVPixelFormat {
    // SAFETY: libav gives the context that `attach` set up, whose `opaque` holds a format, and
    // a list of formats that ends in AV_PIX_FMT_NONE (-1); the formats are read as integers, so
    // that no value is taken for a variant that is not one.
    unsafe {
        let wanted = c_int::try_from((*context).opaque as isize).unwrap_or(-2);
        let numbers = offered.cast::<c_int>();
        let mut index = 0;
        while *numbers.add(index) != -1 {
            if *numbers.add(index) == wanted {
                return *offered.add(index);
            }
            index += 1;
        }
        avcodec_default_get_format(context, offered)
    }
}

/// Set how many threads a decoder that is not open yet decodes with, 0 for as many as libav
/// thinks best, and whether they decode frames or slices of a frame at once or slices only.
pub(crate) fn set_threads(context: &mut ff::codec::Context, count: i32, frames_too: bool) {
    // SAFETY: the context is not open, so its threads may be set.
    unsafe {
        let context = context.as_mut_ptr();
        (*context).thread_count = count;
        (*context).thread_type = if frames_too {
            FF_THREAD_FRAME | FF_THREAD_SLICE
        } else {
            FF_THREAD_SLICE
        };
    }
}

/// Whether frames of `format` are on a device, not in memory.
pub(crate) fn on_device(format: ff::format::Pixel) -> bool {
    // SAFETY: av_pix_fmt_desc_get gives a descriptor that lives as long as the program, or null.
    unsafe {
        let descriptor = av_pix_fmt_desc_get(format.into());
        !descriptor.is_null() && (*descriptor).flags & AV_PIX_FMT_FLAG_HWACCEL as u64 != 0
    }
}

/// The bits of the first component of a pixel of `format`: of luma, for a format of Y'CbCr.
pub(crate) fn depth(format: ff::format::Pixel) -> Option<u8> {
    // SAFETY: as in `on_device`.
    unsafe { depth_of(av_pix_fmt_desc_get(format.into())) }
}

/// The format in memory of the frames of the device that `frame` is on, as they are copied
/// out of it (NV12 for 8 bits, P010 for 10 bits, others for 4:4:4 or 12 bits).
pub(crate) fn memory_format(frame: &ff::frame::Video) -> Option<ff::format::Pixel> {
    // SAFETY: a frame on a device holds a reference to the frames context of the device, whose
    // data is an AVHWFramesContext; it is only read.
    unsafe {
        let frames = (*frame.as_ptr()).hw_frames_ctx;
        if frames.is_null() {
            return None;
        }
        // The data of a frames context is an AVHWFramesContext that libav allocated, aligned
        // for it.
        #[allow(clippy::cast_ptr_alignment)]
        let context = (*frames).data.cast::<AVHWFramesContext>();
        Some((*context).sw_format.into())
    }
}

/// Copy a frame on a device into memory, as its [`memory_format`]. Where `into` holds a frame
/// of that format and size, copied there before and shared with no one, it is written over, so
/// that no new memory is touched; else it is emptied first, and given new memory.
pub(crate) fn transfer(into: &mut ff::frame::Video, from: &ff::frame::Video) -> Result<(), String> {
    let format = memory_format(from).ok_or("the frame is not on a device")?;
    let reuse =
        into.format() == format && (into.width(), into.height()) == (from.width(), from.height());
    // SAFETY: both frames are frames of libav that this side owns. A frame of the format and
    // size that the device copies out was given its memory by an earlier copy (`into` is only
    // ever filled here), and is shared with no one, since what is copied is copied out of it
    // again before the next frame: av_hwframe_transfer_data writes the planes of that format
    // into it. Else `into` is emptied, and av_hwframe_transfer_data gives it buffers of its
    // own, of that format.
    let code = unsafe {
        let (into, from) = (into.as_mut_ptr(), from.as_ptr());
        if !reuse || (*into).buf[0].is_null() {
            av_frame_unref(into);
        }
        av_hwframe_transfer_data(into, from, 0)
    };
    if code < 0 {
        return Err(format!(
            "the frame cannot be copied from the graphics card: {}",
            error_text(code)
        ));
    }
    Ok(())
}

/// Planes of memory to be written: luma and chroma, each with the bytes from a row to the next.
pub(crate) struct Planes<'a> {
    pub luma: &'a mut [u8],
    pub luma_stride: usize,
    pub chroma: &'a mut [u8],
    pub chroma_stride: usize,
}

impl Planes<'_> {
    fn strides(&self) -> Result<(c_int, c_int), String> {
        let stride = |bytes: usize| c_int::try_from(bytes).map_err(|_| "a row too long".to_owned());
        Ok((stride(self.luma_stride)?, stride(self.chroma_stride)?))
    }
}

/// Does nothing: the memory of a buffer that `transfer_into` makes is a plane of a picture,
/// which the picture frees.
unsafe extern "C" fn keep(_opaque: *mut c_void, _data: *mut u8) {}

/// Copy a frame on a device straight into planes of memory, in its [`memory_format`], which is
/// of two planes laid out as the planes are (NV12, P010, or those of 4:2:2 and 4:4:4 of 16-bit
/// words: P210 to P416): one copy, with no frame in between. The planes are long enough for the size of
/// the frame at their strides, which are at least as long as a row.
pub(crate) fn transfer_into(
    from: &ff::frame::Video,
    planes: &mut Planes<'_>,
) -> Result<(), String> {
    let format = memory_format(from).ok_or("the frame is not on a device")?;
    let (luma_stride, chroma_stride) = planes.strides()?;
    // SAFETY: a frame is made that describes the planes: of the format and size of `from`, the
    // planes at their strides, which are long enough for its rows. Each plane is held by a
    // buffer that frees nothing, so the frame refers to memory that outlives it (the frame is
    // freed before this returns, and the planes are borrowed until then); a frame that has
    // memory is written into by av_hwframe_transfer_data, before it returns, and never kept.
    let code = unsafe {
        let mut frame = av_frame_alloc();
        if frame.is_null() {
            return Err("no memory for a frame".to_owned());
        }
        let luma = av_buffer_create(
            planes.luma.as_mut_ptr(),
            planes.luma.len(),
            Some(keep),
            std::ptr::null_mut(),
            0,
        );
        let chroma = av_buffer_create(
            planes.chroma.as_mut_ptr(),
            planes.chroma.len(),
            Some(keep),
            std::ptr::null_mut(),
            0,
        );
        (*frame).buf[0] = luma;
        (*frame).buf[1] = chroma;
        let code = if luma.is_null() || chroma.is_null() {
            ff::ffi::AVERROR(ff::ffi::ENOMEM)
        } else {
            let from = from.as_ptr();
            let pixel: AVPixelFormat = format.into();
            (*frame).format = pixel as c_int;
            (*frame).width = (*from).width;
            (*frame).height = (*from).height;
            (*frame).data[0] = planes.luma.as_mut_ptr();
            (*frame).data[1] = planes.chroma.as_mut_ptr();
            (*frame).linesize[0] = luma_stride;
            (*frame).linesize[1] = chroma_stride;
            av_hwframe_transfer_data(frame, from, 0)
        };
        av_frame_free(&raw mut frame);
        code
    };
    if code < 0 {
        return Err(format!(
            "the frame cannot be copied from the graphics card: {}",
            error_text(code)
        ));
    }
    Ok(())
}

/// Convert a frame in memory with a scaler made for its format and size, straight into planes
/// of NV12 or P010 of its size: one copy, with no frame in between. The planes are long enough
/// for the size of the frame at their strides.
pub(crate) fn scale_into(
    scaler: &mut Scaler,
    from: &ff::frame::Video,
    planes: &mut Planes<'_>,
) -> Result<(), String> {
    let (luma_stride, chroma_stride) = planes.strides()?;
    let strides = [luma_stride, chroma_stride, 0, 0];
    let mut data = [
        planes.luma.as_mut_ptr(),
        planes.chroma.as_mut_ptr(),
        std::ptr::null_mut(),
        std::ptr::null_mut(),
    ];
    // SAFETY: the scaler was made for the format and size of `from`, whose planes it reads;
    // it writes the two planes of its output format, of the same size, at the strides given,
    // which the planes are long enough for.
    let rows = unsafe {
        let from = from.as_ptr();
        sws_scale(
            scaler.0.as_mut_ptr(),
            (*from).data.as_ptr().cast(),
            (*from).linesize.as_ptr(),
            0,
            (*from).height,
            data.as_mut_ptr(),
            strides.as_ptr(),
        )
    };
    if rows < 0 {
        return Err(format!(
            "the frame cannot be converted: {}",
            error_text(rows)
        ));
    }
    Ok(())
}

/// Another reference to a frame, which shares its memory, on a device or not.
pub(crate) fn reference(frame: &ff::frame::Video) -> Result<ff::frame::Video, String> {
    let mut copy = ff::frame::Video::empty();
    // SAFETY: `copy` is an empty frame of this side; av_frame_ref makes it refer to the buffers
    // of `frame`, counting them, and copies its fields.
    let code = unsafe { av_frame_ref(copy.as_mut_ptr(), frame.as_ptr()) };
    if code < 0 {
        return Err(format!("the frame cannot be kept: {}", error_text(code)));
    }
    Ok(copy)
}

/// Seek `stream` to its key frame at or before `timestamp`, or, if `backward` is false, at or
/// after it.
pub(crate) fn seek(
    input: &mut ff::format::context::Input,
    stream: usize,
    timestamp: i64,
    backward: bool,
) -> Result<(), String> {
    let stream = c_int::try_from(stream).map_err(|_| "no such stream".to_owned())?;
    let flags = if backward { AVSEEK_FLAG_BACKWARD } else { 0 };
    // SAFETY: the context is open, and the stream is one of it.
    let code = unsafe { av_seek_frame(input.as_mut_ptr(), stream, timestamp, flags) };
    if code < 0 {
        return Err(format!("seeking failed: {}", error_text(code)));
    }
    Ok(())
}

/// The timestamp of the last key frame at or before `timestamp` in the index of a stream, if
/// its index has one. Some formats index the time a frame is decoded rather than shown, which
/// is a little earlier where frames are reordered.
pub(crate) fn key_frame_before(
    input: &ff::format::context::Input,
    stream: usize,
    timestamp: i64,
) -> Option<i64> {
    let stream = input.stream(stream)?;
    // SAFETY: the stream is of an open context; the entry that is given lives until the next
    // packet is read, and only its timestamp is read before then.
    unsafe {
        let entry = avformat_index_get_entry_from_timestamp(
            stream.as_ptr().cast_mut(),
            timestamp,
            AVSEEK_FLAG_BACKWARD,
        );
        if entry.is_null() {
            None
        } else {
            Some((*entry).timestamp)
        }
    }
}

/// Whether a packet is to be dropped once decoded: before the start of an edit list.
pub(crate) fn discarded(packet: &ff::Packet) -> bool {
    // SAFETY: the packet is valid for as long as it is borrowed; only its flags are read.
    unsafe { (*ff::packet::Ref::as_ptr(packet)).flags & ff::ffi::AV_PKT_FLAG_DISCARD != 0 }
}

/// How many entries the index of a stream has.
pub(crate) fn index_entries(input: &ff::format::context::Input, stream: usize) -> i32 {
    let Some(stream) = input.stream(stream) else {
        return 0;
    };
    // SAFETY: the stream is of an open context.
    unsafe { avformat_index_get_entries_count(stream.as_ptr()) }
}

/// How many entries of the index of a stream are frames that are kept once decoded: those that
/// are not before the start (or after the end) of an edit list.
pub(crate) fn kept_index_entries(input: &ff::format::context::Input, stream: usize) -> u64 {
    let Some(stream) = input.stream(stream) else {
        return 0;
    };
    // SAFETY: the stream is of an open context, and nothing is read from it meanwhile; each
    // entry that is given lives until then, and only its flags are read.
    unsafe {
        let pointer = stream.as_ptr().cast_mut();
        let count = avformat_index_get_entries_count(pointer);
        (0..count)
            .filter(|&index| {
                let entry = avformat_index_get_entry(pointer, index);
                !entry.is_null() && (*entry).flags() & AVINDEX_DISCARD_FRAME == 0
            })
            .count() as u64
    }
}

/// Seek to byte `position` of the file, where the demuxer can: for the formats whose seeking by
/// time finds no key frame (MPEG-TS, MPEG-PS), the sure way back to their start.
pub(crate) fn seek_to_byte(
    input: &mut ff::format::context::Input,
    position: i64,
) -> Result<(), String> {
    // SAFETY: the context is open; seeking by bytes takes no stream.
    let code = unsafe {
        av_seek_frame(
            input.as_mut_ptr(),
            -1,
            position,
            AVSEEK_FLAG_BYTE | AVSEEK_FLAG_BACKWARD,
        )
    };
    if code < 0 {
        return Err(format!("seeking failed: {}", error_text(code)));
    }
    Ok(())
}

/// Have the demuxer drop the packets of every stream but `keep`, unread where it can.
pub(crate) fn discard_others(input: &mut ff::format::context::Input, keep: usize) {
    let count = input.nb_streams() as usize;
    // SAFETY: the context is open, and `streams` has `nb_streams` of them; `discard` may be
    // set between reads.
    unsafe {
        let context = input.as_mut_ptr();
        for index in 0..count {
            if index != keep {
                let stream = *(*context).streams.add(index);
                (*stream).discard = AVDiscard::AVDISCARD_ALL;
            }
        }
    }
}

/// What the parameters of a stream say, of what `ffmpeg-next` cannot read without opening a
/// decoder.
pub(crate) struct Parameters {
    pub width: i32,
    pub height: i32,
    /// The format of a pixel, as libav numbers it, or -1.
    pub format: c_int,
    pub sample_rate: i32,
    pub channels: i32,
    pub bits_per_raw_sample: i32,
    /// Samples in each frame of sound, where every frame has as many; or 0.
    pub frame_size: i32,
    /// Samples of sound to be decoded and dropped after a seek, as Opus needs; or 0.
    pub seek_preroll: i32,
    /// How many frames a picture is shown after it is decoded, where frames are reordered (as
    /// B-frames are); or 0.
    pub video_delay: i32,
}

/// What the parameters of a stream say.
pub(crate) fn parameters(stream: &ff::format::stream::Stream) -> Parameters {
    // SAFETY: the stream is of an open context, and has parameters.
    unsafe {
        let parameters = (*stream.as_ptr()).codecpar;
        Parameters {
            width: (*parameters).width,
            height: (*parameters).height,
            format: (*parameters).format,
            sample_rate: (*parameters).sample_rate,
            channels: (*parameters).ch_layout.nb_channels,
            bits_per_raw_sample: (*parameters).bits_per_raw_sample,
            frame_size: (*parameters).frame_size,
            seek_preroll: (*parameters).seek_preroll,
            video_delay: (*parameters).video_delay,
        }
    }
}

/// The numbers that a stream or a frame gives for its colours, as libav numbers them: the colour
/// primaries, the transfer, the matrix (`AVColorSpace`), the range and where chroma lies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ColorCodes {
    pub primaries: c_int,
    pub transfer: c_int,
    pub matrix: c_int,
    pub range: c_int,
    pub location: c_int,
}

impl ColorCodes {
    /// Nothing said: each number libav's "unspecified".
    #[cfg_attr(not(test), allow(dead_code))]
    pub const UNSAID: ColorCodes = ColorCodes {
        primaries: 2,
        transfer: 2,
        matrix: 2,
        range: 0,
        location: 0,
    };

    /// What these say, and where they say nothing, what `then` says: of a stream before its
    /// frames, as MXF says the colours of `DNxHR` whose frames are said by its decoder to be
    /// of BT.709 whatever they are, and as [`crate::probe`] reads them. Unspecified (2) and
    /// reserved (0) primaries and transfers, an unspecified (2) matrix, an unspecified (0)
    /// range or siting say nothing; a matrix of 0 (R'G'B') does.
    #[must_use]
    pub fn before(self, then: ColorCodes) -> ColorCodes {
        let said = |value: c_int, nothing: &[c_int], other: c_int| {
            if nothing.contains(&value) {
                other
            } else {
                value
            }
        };
        ColorCodes {
            primaries: said(self.primaries, &[0, 2], then.primaries),
            transfer: said(self.transfer, &[0, 2], then.transfer),
            matrix: said(self.matrix, &[2], then.matrix),
            range: said(self.range, &[0], then.range),
            location: said(self.location, &[0], then.location),
        }
    }
}

/// Read a field of libav that is an enumeration as the integer that it is, so that a number
/// that is no variant of the enumeration of the bindings is not taken for one.
///
/// # Safety
/// `field` points to a field of an enumeration of `repr(i32)`, or of `c_int`, that may be read.
unsafe fn number<T>(field: *const T) -> c_int {
    debug_assert_eq!(std::mem::size_of::<T>(), std::mem::size_of::<c_int>());
    // SAFETY: as the caller says; an enumeration of the bindings is of `repr(i32)`.
    unsafe { field.cast::<c_int>().read() }
}

/// What the parameters of a stream say of its colours.
pub(crate) fn stream_colors(stream: &ff::format::stream::Stream) -> ColorCodes {
    // SAFETY: the stream is of an open context, and has parameters; each field is an
    // enumeration of `repr(i32)` that is only read.
    unsafe {
        let parameters = (*stream.as_ptr()).codecpar;
        ColorCodes {
            primaries: number(&raw const (*parameters).color_primaries),
            transfer: number(&raw const (*parameters).color_trc),
            matrix: number(&raw const (*parameters).color_space),
            range: number(&raw const (*parameters).color_range),
            location: number(&raw const (*parameters).chroma_location),
        }
    }
}

/// What a decoded frame says of its colours.
pub(crate) fn frame_colors(frame: &ff::frame::Video) -> ColorCodes {
    // SAFETY: the frame is a frame of libav; each field is an enumeration of `repr(i32)` that
    // is only read.
    unsafe {
        let frame = frame.as_ptr();
        ColorCodes {
            primaries: number(&raw const (*frame).color_primaries),
            transfer: number(&raw const (*frame).color_trc),
            matrix: number(&raw const (*frame).colorspace),
            range: number(&raw const (*frame).color_range),
            location: number(&raw const (*frame).chroma_location),
        }
    }
}

/// How the pixels of a format of libav are laid out, as far as the pictures of the crate need.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "each a fact of the descriptor of libav"
)]
pub(crate) struct Layout {
    /// Its components are R, G and B, not Y', Cb and Cr.
    pub rgb: bool,
    /// How far chroma is halved across and down, as powers of 2: (1, 1) for 4:2:0, (1, 0) for
    /// 4:2:2, (0, 0) for 4:4:4.
    pub log2_chroma: (u32, u32),
    /// The bits of its first component.
    pub depth: u32,
    /// Whether it is Y', Cb and Cr each in a plane of its own (planes 0, 1 and 2), each sample
    /// a byte (8 bits) or a little-endian word of 16 bits that holds its bits at the bottom, all
    /// of the same depth: so that its planes may be read as they are.
    pub planar_yuv: bool,
    /// Whether it is planar Y'CbCr ([`Layout::planar_yuv`]) with alpha in a fourth plane of its
    /// own (plane 3), of the depth and sample size of luma: the `yuva` formats.
    pub alpha_plane: bool,
    /// Whether its pixels are R'G'B', or grey (one value for R', G' and B'), or indices of a
    /// palette of R'G'B'A, with or without alpha, of components that libav reads one at a time
    /// (`av_read_image_line2`): so that they are taken as R'G'B'A as they are, never made
    /// Y'CbCr. Not of a Bayer mosaic, which is to be demosaiced first.
    pub rgb_like: bool,
}

/// How the pixels of `format` are laid out; none for a format that libav does not describe.
pub(crate) fn layout(format: ff::format::Pixel) -> Option<Layout> {
    // SAFETY: as in `on_device`; the descriptor is only read.
    unsafe {
        let descriptor = av_pix_fmt_desc_get(format.into());
        if descriptor.is_null() || (*descriptor).nb_components == 0 {
            return None;
        }
        let descriptor = &*descriptor;
        let flag = |bit: c_int| descriptor.flags & u64::from(bit.unsigned_abs()) != 0;
        let rgb = flag(AV_PIX_FMT_FLAG_RGB);
        let depth = u32::try_from(descriptor.comp[0].depth).ok()?;
        let step = if depth > 8 { 2 } else { 1 };
        let planar_yuv = !rgb
            && !flag(AV_PIX_FMT_FLAG_BE)
            && flag(AV_PIX_FMT_FLAG_PLANAR)
            && descriptor.nb_components >= 3
            && (0..3).all(|index: usize| {
                let component = &descriptor.comp[index];
                component.plane == c_int::try_from(index).unwrap_or(-1)
                    && component.step == step
                    && component.offset == 0
                    && component.shift == 0
                    && u32::try_from(component.depth).ok() == Some(depth)
            })
            && depth <= 16;
        let alpha = &descriptor.comp[3];
        let alpha_plane = planar_yuv
            && flag(AV_PIX_FMT_FLAG_ALPHA)
            && descriptor.nb_components == 4
            && alpha.plane == 3
            && alpha.step == step
            && alpha.offset == 0
            && alpha.shift == 0
            && u32::try_from(alpha.depth).ok() == Some(depth);
        let rgb_like = !flag(AV_PIX_FMT_FLAG_BAYER)
            && !flag(AV_PIX_FMT_FLAG_HWACCEL)
            && (rgb || flag(AV_PIX_FMT_FLAG_PAL) || descriptor.nb_components <= 2);
        Some(Layout {
            rgb,
            log2_chroma: (
                u32::from(descriptor.log2_chroma_w),
                u32::from(descriptor.log2_chroma_h),
            ),
            depth,
            planar_yuv,
            alpha_plane,
            rgb_like,
        })
    }
}

/// The components of the pixels of a format that [`Layout::rgb_like`] says are R'G'B', grey or
/// indices of a palette, as [`read_row`] reads them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "each a flag of the descriptor of libav"
)]
pub(crate) struct Components {
    /// How many there are: R, G, B and maybe A (in that order, whatever planes or bytes they
    /// are in); grey and maybe A; or the index of a palette.
    pub count: usize,
    /// The bits of each (32 of a float, 16 of a half float).
    pub depths: [u32; 4],
    /// Whether they are R, G and B, not grey.
    pub rgb: bool,
    /// Whether the last is alpha (of a palette: whether its entries have alpha).
    pub alpha: bool,
    /// Whether they are floats (of 32 bits, or halves of 16), not integers.
    pub float: bool,
    /// Whether the first is an index of the palette that the frame carries.
    pub palette: bool,
}

/// The components of the pixels of `format`, where it is a format that libav describes.
pub(crate) fn components(format: ff::format::Pixel) -> Option<Components> {
    // SAFETY: as in `layout`.
    unsafe {
        let descriptor = av_pix_fmt_desc_get(format.into());
        if descriptor.is_null() || (*descriptor).nb_components == 0 {
            return None;
        }
        let descriptor = &*descriptor;
        let flag = |bit: c_int| descriptor.flags & u64::from(bit.unsigned_abs()) != 0;
        let mut depths = [0; 4];
        for (depth, component) in depths.iter_mut().zip(&descriptor.comp) {
            *depth = u32::try_from(component.depth).unwrap_or(0);
        }
        Some(Components {
            count: usize::from(descriptor.nb_components).min(4),
            depths,
            rgb: flag(AV_PIX_FMT_FLAG_RGB),
            alpha: flag(AV_PIX_FMT_FLAG_ALPHA),
            float: flag(AV_PIX_FMT_FLAG_FLOAT),
            palette: flag(AV_PIX_FMT_FLAG_PAL),
        })
    }
}

/// Read the values of `component` in row `y` of a frame in memory into `into`, one for each of
/// the first pixels of the row (as many as `into` holds), as libav reads them
/// (`av_read_image_line2`): an integer with the bits of the component at the bottom, the bits
/// of a float as they are, an index of a palette as it is.
///
/// # Panics
/// `y` is not a row of the frame, `component` is not one of its format, or `into` is longer
/// than a row.
pub(crate) fn read_row(frame: &ff::frame::Video, component: usize, y: u32, into: &mut [u32]) {
    let format = frame.format();
    let count = components(format).map_or(0, |described| described.count);
    assert!(
        component < count && y < frame.height() && into.len() <= frame.width() as usize,
        "row {y} of component {component} of a frame of {}×{}",
        frame.width(),
        frame.height()
    );
    // SAFETY: the frame is in memory (a format of a device has no components), of its own
    // format and size, so its planes hold every row of every component; the row, the component
    // and the pixels read are its own (checked above), and `into` has room for the values read,
    // of 4 bytes each. Nothing is written but `into`.
    unsafe {
        let raw = frame.as_ptr();
        let mut data: [*const u8; 4] = [std::ptr::null(); 4];
        for (to, from) in data.iter_mut().zip(&(*raw).data) {
            *to = from.cast_const();
        }
        av_read_image_line2(
            into.as_mut_ptr().cast(),
            data.as_mut_ptr(),
            (*raw).linesize.as_ptr(),
            av_pix_fmt_desc_get(format.into()),
            0,
            c_int::try_from(y).unwrap_or(0),
            c_int::try_from(component).unwrap_or(0),
            c_int::try_from(into.len()).unwrap_or(0),
            0,
            4,
        );
    }
}

/// The palette of a frame of a format of a palette: 256 entries of A, R, G and B, a byte each
/// from the top, as libav holds them.
///
/// # Panics
/// The format of the frame has no palette.
pub(crate) fn palette(frame: &ff::frame::Video) -> [u32; 256] {
    assert!(
        components(frame.format()).is_some_and(|described| described.palette),
        "a frame without a palette"
    );
    let mut entries = [0_u32; 256];
    // SAFETY: a frame of a format of a palette holds its palette in plane 1: 256 entries of 4
    // bytes in the native order of bytes, as libav lays them; they are only read.
    unsafe {
        let plane = (*frame.as_ptr()).data[1].cast_const();
        if !plane.is_null() {
            for (at, entry) in entries.iter_mut().enumerate() {
                *entry = plane.add(4 * at).cast::<u32>().read_unaligned();
            }
        }
    }
    entries
}

/// What a decoded frame says of its alpha, as libav numbers it (`AVAlphaMode`): 0 nothing, 1
/// premultiplied, 2 straight.
pub(crate) fn alpha_mode(frame: &ff::frame::Video) -> c_int {
    // SAFETY: as in `frame_colors`.
    unsafe { number(&raw const (*frame.as_ptr()).alpha_mode) }
}

extern "C" {
    /// `av_pix_fmt_desc_get`, which takes any number and gives null for one that is not a
    /// format: declared here to take the number as the integer that it is, so that no number
    /// is taken for a variant of `AVPixelFormat` that it is not.
    #[link_name = "av_pix_fmt_desc_get"]
    fn descriptor_of_number(format: c_int) -> *const ff::ffi::AVPixFmtDescriptor;
}

/// The format that libav numbers `format`, if it is one.
pub(crate) fn pixel_of_number(format: c_int) -> Option<ff::format::Pixel> {
    // SAFETY: the function takes any number. A number that has a descriptor is a format of
    // libav, so a variant of `AVPixelFormat` (of `repr(i32)`), whose bindings hold every format.
    unsafe {
        if descriptor_of_number(format).is_null() {
            return None;
        }
        Some(std::mem::transmute::<c_int, AVPixelFormat>(format).into())
    }
}

/// The bits of luma of the format that libav numbers `format`.
pub(crate) fn depth_of_number(format: c_int) -> Option<u8> {
    // SAFETY: the function takes any number; the descriptor lives as long as the program.
    unsafe { depth_of(descriptor_of_number(format)) }
}

/// The bits of the first component of a pixel that `descriptor` describes.
///
/// # Safety
/// `descriptor` is null or a descriptor of libav.
unsafe fn depth_of(descriptor: *const ff::ffi::AVPixFmtDescriptor) -> Option<u8> {
    // SAFETY: as the caller says.
    unsafe {
        if descriptor.is_null() || (*descriptor).nb_components == 0 {
            return None;
        }
        u8::try_from((*descriptor).comp[0].depth)
            .ok()
            .filter(|&bits| bits > 0)
    }
}

/// A converter of frames between formats, of swscale.
pub(crate) struct Scaler(pub ff::software::scaling::Context);

impl Scaler {
    /// The same limited BT.709 YUV interpretation as the first-phase preview, to full-range
    /// RGB. swscale otherwise defaults to BT.601, which makes thumbnails visibly disagree.
    pub fn thumbnail_colors(&mut self) -> Result<(), String> {
        // SAFETY: the context is live and exclusively borrowed; the four coefficients are
        // a static libswscale table. Contrast and saturation are unity in 16.16 fixed point.
        let code = unsafe {
            let coefficients = ff::ffi::sws_getCoefficients(ff::ffi::SWS_CS_ITU709);
            ff::ffi::sws_setColorspaceDetails(
                self.0.as_mut_ptr(),
                coefficients,
                0,
                coefficients,
                1,
                0,
                1 << 16,
                1 << 16,
            )
        };
        if code < 0 {
            Err(error_text(code))
        } else {
            Ok(())
        }
    }
}

impl Scaler {
    /// Make R'G'B' into Y'CbCr by the matrix of BT.709, of limited range, rather than by
    /// BT.601, which swscale takes where it is not told.
    pub fn rgb_to_bt709(&mut self) -> Result<(), String> {
        // SAFETY: as in `thumbnail_colors`; R'G'B' is of full range, Y'CbCr of limited.
        let code = unsafe {
            let coefficients = ff::ffi::sws_getCoefficients(ff::ffi::SWS_CS_ITU709);
            ff::ffi::sws_setColorspaceDetails(
                self.0.as_mut_ptr(),
                coefficients,
                1,
                coefficients,
                0,
                0,
                1 << 16,
                1 << 16,
            )
        };
        if code < 0 {
            Err(error_text(code))
        } else {
            Ok(())
        }
    }
}

// SAFETY: a context of swscale is used by one thread at a time, which `&mut` sees to; it holds
// nothing of the thread that made it.
unsafe impl Send for Scaler {}

/// A converter of sound, of swresample: from the format, rate and channels of the frames of a
/// decoder to 32-bit floats at another rate, interleaved, in the default layout of another
/// number of channels (stereo for 2), mixing the channels down or up as swresample does.
pub(crate) struct Resampler {
    context: *mut SwrContext,
    /// What it takes: the format of a sample, the rate, and the layout of the channels.
    format: ff::format::Sample,
    rate: i32,
    layout: AVChannelLayout,
    channels: usize,
}

// SAFETY: the context and the layout are used by one thread at a time, which `&mut` sees to;
// they hold nothing of the thread that made them.
unsafe impl Send for Resampler {}

impl Drop for Resampler {
    fn drop(&mut self) {
        // SAFETY: the context was made by swr_alloc_set_opts2, the layout by
        // av_channel_layout_copy or av_channel_layout_default, and each is let go only here.
        unsafe {
            swr_free(&raw mut self.context);
            av_channel_layout_uninit(&raw mut self.layout);
        }
    }
}

/// The layout of the channels of a frame, or the default layout of as many channels where the
/// frame's says only how many there are, or is not valid.
///
/// # Safety
/// `frame` is a frame of sound of libav.
unsafe fn layout_of(frame: *const ff::ffi::AVFrame) -> Result<AVChannelLayout, String> {
    // SAFETY: as the caller says; a layout that is zeroed is an empty one, which
    // av_channel_layout_copy and av_channel_layout_default write over.
    unsafe {
        let given = &raw const (*frame).ch_layout;
        let mut layout: AVChannelLayout = std::mem::zeroed();
        let count = (*given).nb_channels;
        if count <= 0 {
            return Err("its sound has no channels".to_owned());
        }
        if (*given).order == AVChannelOrder::AV_CHANNEL_ORDER_UNSPEC
            || av_channel_layout_check(given) == 0
        {
            av_channel_layout_default(&raw mut layout, count);
        } else if av_channel_layout_copy(&raw mut layout, given) < 0 {
            return Err("the layout of its channels cannot be copied".to_owned());
        }
        Ok(layout)
    }
}

impl Resampler {
    /// A converter of frames of the format, rate and layout of `frame` to `rate` and
    /// `channels`.
    pub fn new(frame: &ff::frame::Audio, rate: u32, channels: u16) -> Result<Resampler, String> {
        let format = frame.format();
        if matches!(format, ff::format::Sample::None) {
            return Err("its sound has no format of samples".to_owned());
        }
        let out_rate = c_int::try_from(rate).map_err(|_| "a rate too high".to_owned())?;
        // SAFETY: the frame is a frame of sound of libav. The layouts are made by libav, and
        // swr_alloc_set_opts2 copies them; the context it gives is set up by swr_init, or
        // freed.
        unsafe {
            let raw = frame.as_ptr();
            let in_rate = (*raw).sample_rate;
            if in_rate <= 0 {
                return Err("its sound has no rate".to_owned());
            }
            let mut layout = layout_of(raw)?;
            let mut out_layout: AVChannelLayout = std::mem::zeroed();
            av_channel_layout_default(&raw mut out_layout, c_int::from(channels));
            let mut context = std::ptr::null_mut();
            let code = swr_alloc_set_opts2(
                &raw mut context,
                &raw const out_layout,
                AVSampleFormat::AV_SAMPLE_FMT_FLT,
                out_rate,
                &raw const layout,
                format.into(),
                in_rate,
                0,
                std::ptr::null_mut(),
            );
            av_channel_layout_uninit(&raw mut out_layout);
            let code = if code < 0 { code } else { swr_init(context) };
            if code < 0 {
                swr_free(&raw mut context);
                av_channel_layout_uninit(&raw mut layout);
                return Err(format!(
                    "its sound cannot be converted: {}",
                    error_text(code)
                ));
            }
            Ok(Resampler {
                context,
                format,
                rate: in_rate,
                layout,
                channels: usize::from(channels),
            })
        }
    }

    /// Whether frames of the format, rate and layout of `frame` are what it takes.
    pub fn takes(&self, frame: &ff::frame::Audio) -> bool {
        // SAFETY: the frame is a frame of sound of libav, and the layouts are valid.
        unsafe {
            let raw = frame.as_ptr();
            if frame.format() != self.format || (*raw).sample_rate != self.rate {
                return false;
            }
            match layout_of(raw) {
                Ok(mut layout) => {
                    let same =
                        av_channel_layout_compare(&raw const layout, &raw const self.layout) == 0;
                    av_channel_layout_uninit(&raw mut layout);
                    same
                }
                Err(_) => false,
            }
        }
    }

    /// Convert a frame, passing over its first `skip` samples (fewer than it has), or, given
    /// none, what the converter holds back, and add the samples to `out`, interleaved.
    pub fn convert(
        &mut self,
        frame: Option<(&ff::frame::Audio, usize)>,
        out: &mut Vec<f32>,
    ) -> Result<(), String> {
        let samples = match frame {
            Some((frame, skip)) => c_int::try_from(frame.samples().saturating_sub(skip))
                .map_err(|_| "a frame too long".to_owned())?,
            None => 0,
        };
        // The planes from the first sample taken: each plane of a planar format holds a
        // channel, the one plane of a packed format all of them.
        let mut planes_in: Vec<*const u8> = Vec::new();
        if let Some((frame, skip)) = frame {
            let format = frame.format();
            let (count, step) = if format.is_planar() {
                (usize::from(frame.channels()), format.bytes())
            } else {
                (1, format.bytes() * usize::from(frame.channels()))
            };
            // SAFETY: `extended_data` points to a pointer for each plane of the frame (one for
            // each channel where it is planar, else one), each to `samples` samples of `step`
            // bytes; `skip` is fewer than its samples, so the pointers stay within.
            unsafe {
                let data = (*frame.as_ptr()).extended_data;
                for plane in 0..count {
                    planes_in.push((*data.add(plane)).add(skip * step).cast_const());
                }
            }
        }
        // SAFETY: the context is set up. The frame, if there is one, is of the format, rate
        // and layout that it takes (`takes`), and `planes_in` points to a plane of `samples`
        // samples for each channel, or one plane of all of them interleaved. `out`
        // is given room for as many samples as swr_get_out_samples says may come, each of
        // `channels` floats, and is cut to those that came.
        unsafe {
            let room = swr_get_out_samples(self.context, samples);
            if room < 0 {
                return Err(format!(
                    "the sound cannot be converted: {}",
                    error_text(room)
                ));
            }
            let room_samples = usize::try_from(room).unwrap_or(0);
            let before = out.len();
            out.resize(before + room_samples * self.channels, 0.0);
            let planes = [out.as_mut_ptr().add(before).cast::<u8>()];
            let input = if planes_in.is_empty() {
                std::ptr::null()
            } else {
                planes_in.as_ptr()
            };
            let made = swr_convert(self.context, planes.as_ptr(), room, input, samples);
            if made < 0 {
                out.truncate(before);
                return Err(format!(
                    "the sound cannot be converted: {}",
                    error_text(made)
                ));
            }
            out.truncate(before + usize::try_from(made).unwrap_or(0) * self.channels);
        }
        Ok(())
    }
}

/// Make a frame that is about to be written into its own: where an encoder still holds its
/// memory (as one that keeps frames to look ahead may), it is given new memory, so that what
/// the encoder holds is not written over.
pub(crate) fn writable(frame: &mut ff::Frame) -> Result<(), String> {
    // SAFETY: the frame is a frame of libav whose buffers were made by av_frame_get_buffer;
    // av_frame_make_writable copies it into buffers of its own where they are shared.
    let code = unsafe { av_frame_make_writable(frame.as_mut_ptr()) };
    if code < 0 {
        return Err(format!("no memory for a frame: {}", error_text(code)));
    }
    Ok(())
}

/// Have an encoder that is not open yet say that its frames are BT.709 of limited range, as
/// the pictures of the crate are taken to be.
pub(crate) fn tag(context: &mut ff::codec::Context, tags: &crate::encode::ColorTags) {
    use ff::ffi::{
        av_frame_side_data_new, AVColorPrimaries as P, AVColorSpace as M,
        AVColorTransferCharacteristic as T, AVFrameSideDataType,
    };
    // H.273's numbers, as libav numbers them too; one it has no name for is unspecified.
    let primaries = match tags.primaries {
        1 => P::AVCOL_PRI_BT709,
        4 => P::AVCOL_PRI_BT470M,
        5 => P::AVCOL_PRI_BT470BG,
        6 => P::AVCOL_PRI_SMPTE170M,
        7 => P::AVCOL_PRI_SMPTE240M,
        9 => P::AVCOL_PRI_BT2020,
        11 => P::AVCOL_PRI_SMPTE431,
        12 => P::AVCOL_PRI_SMPTE432,
        _ => P::AVCOL_PRI_UNSPECIFIED,
    };
    let transfer = match tags.transfer {
        1 => T::AVCOL_TRC_BT709,
        4 => T::AVCOL_TRC_GAMMA22,
        5 => T::AVCOL_TRC_GAMMA28,
        6 => T::AVCOL_TRC_SMPTE170M,
        8 => T::AVCOL_TRC_LINEAR,
        13 => T::AVCOL_TRC_IEC61966_2_1,
        14 => T::AVCOL_TRC_BT2020_10,
        16 => T::AVCOL_TRC_SMPTE2084,
        18 => T::AVCOL_TRC_ARIB_STD_B67,
        _ => T::AVCOL_TRC_UNSPECIFIED,
    };
    let matrix = match tags.matrix {
        1 => M::AVCOL_SPC_BT709,
        5 => M::AVCOL_SPC_BT470BG,
        6 => M::AVCOL_SPC_SMPTE170M,
        9 => M::AVCOL_SPC_BT2020_NCL,
        _ => M::AVCOL_SPC_UNSPECIFIED,
    };
    // SAFETY: the context is not open, so these may be set; the side data is made by libav in
    // the context's own list, which it frees with the context.
    unsafe {
        let context = context.as_mut_ptr();
        (*context).color_primaries = primaries;
        (*context).color_trc = transfer;
        (*context).colorspace = matrix;
        (*context).color_range = if tags.full_range {
            AVColorRange::AVCOL_RANGE_JPEG
        } else {
            AVColorRange::AVCOL_RANGE_MPEG
        };
        if let Some(mastering) = &tags.mastering {
            let bytes = crate::encode::mastering_bytes_of(mastering);
            let made = av_frame_side_data_new(
                &raw mut (*context).decoded_side_data,
                &raw mut (*context).nb_decoded_side_data,
                AVFrameSideDataType::AV_FRAME_DATA_MASTERING_DISPLAY_METADATA,
                bytes.len(),
                0,
            );
            if !made.is_null() && (*made).size >= bytes.len() {
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), (*made).data, bytes.len());
            }
        }
    }
}

/// Side data of a stream, as its bytes.
pub(crate) enum SideData {
    /// An `AVMasteringDisplayMetadata`.
    Mastering(Vec<u8>),
    /// An `AVContentLightMetadata`.
    ContentLight(Vec<u8>),
}

/// Add side data to the parameters of stream `stream` of a file, before its index is written
/// (which `QuickTime` and MP4 write when the file is finished).
pub(crate) fn add_stream_side_data(
    output: &mut ff::format::context::Output,
    stream: usize,
    data: SideData,
) -> Result<(), String> {
    use ff::ffi::{av_free, av_malloc, av_packet_side_data_add, AVPacketSideDataType};
    let (kind, bytes) = match data {
        SideData::Mastering(bytes) => (
            AVPacketSideDataType::AV_PKT_DATA_MASTERING_DISPLAY_METADATA,
            bytes,
        ),
        SideData::ContentLight(bytes) => {
            (AVPacketSideDataType::AV_PKT_DATA_CONTENT_LIGHT_LEVEL, bytes)
        }
    };
    if stream >= output.nb_streams() as usize {
        return Err(format!("there is no stream {stream}"));
    }
    // SAFETY: the context has `nb_streams` streams, each with parameters; the buffer is made by
    // av_malloc, as av_packet_side_data_add takes it (keeping it where it adds it, freed here
    // where it does not); any side data of the same kind is replaced.
    unsafe {
        let stream = *(*output.as_mut_ptr()).streams.add(stream);
        let parameters = (*stream).codecpar;
        let buffer = av_malloc(bytes.len()).cast::<u8>();
        if buffer.is_null() {
            return Err("no memory for side data".into());
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), buffer, bytes.len());
        let added = av_packet_side_data_add(
            &raw mut (*parameters).coded_side_data,
            &raw mut (*parameters).nb_coded_side_data,
            kind,
            buffer.cast::<c_void>(),
            bytes.len(),
            0,
        );
        if added.is_null() {
            av_free(buffer.cast::<c_void>());
            return Err("libav cannot keep side data".into());
        }
    }
    Ok(())
}

/// Set the tag of the codec of a stream of a file whose header is not written yet, as `hvc1`
/// for HEVC, which the players of Apple ask for.
pub(crate) fn set_codec_tag(output: &mut ff::format::context::Output, stream: usize, tag: [u8; 4]) {
    if stream >= output.nb_streams() as usize {
        return;
    }
    // SAFETY: the context has `nb_streams` streams, each with parameters; the header is not
    // written, so the tag may be set.
    unsafe {
        let stream = *(*output.as_mut_ptr()).streams.add(stream);
        (*(*stream).codecpar).codec_tag = u32::from_le_bytes(tag);
    }
}

/// The format of a picture: NV12 for 8 bits, P010 for 10.
pub(crate) fn format_of(picture: &Picture) -> ff::format::Pixel {
    if picture.bits > 8 {
        ff::format::Pixel::P010LE
    } else {
        ff::format::Pixel::NV12
    }
}

/// Convert a picture with a scaler made for its format and size into a frame of the format and
/// size that the scaler makes, which has memory of its own to be written.
pub(crate) fn scale_from(
    scaler: &mut Scaler,
    picture: &Picture,
    into: &mut ff::frame::Video,
) -> Result<(), String> {
    let (input, output) = (scaler.0.input(), scaler.0.output());
    let format = format_of(picture);
    if (input.format, input.width, input.height) != (format, picture.width, picture.height)
        || (output.format, output.width, output.height)
            != (into.format(), into.width(), into.height())
    {
        return Err("the converter was made for pictures of another shape".to_owned());
    }
    let bytes = if picture.bits > 8 { 2 } else { 1 };
    let row = (picture.width as usize).div_ceil(2) * 2 * bytes;
    let rows = picture.height as usize;
    if picture.luma_stride < row
        || picture.chroma_stride < row
        || picture.luma.len() < picture.luma_stride * rows
        || picture.chroma.len() < picture.chroma_stride * rows.div_ceil(2)
    {
        return Err("the planes of the picture are shorter than its size".to_owned());
    }
    // SAFETY: the frame is a frame of libav; only whether it has memory is read.
    if unsafe { into.is_empty() } {
        return Err("the frame has no memory".to_owned());
    }
    let stride = |bytes: usize| c_int::try_from(bytes).map_err(|_| "a row too long".to_owned());
    let strides = [
        stride(picture.luma_stride)?,
        stride(picture.chroma_stride)?,
        0,
        0,
    ];
    let data = [
        picture.luma.as_ptr(),
        picture.chroma.as_ptr(),
        std::ptr::null(),
        std::ptr::null(),
    ];
    let height = c_int::try_from(picture.height).map_err(|_| "a picture too high".to_owned())?;
    // SAFETY: the scaler was made for the format and size of the picture, whose two planes are
    // as long as its rows at their strides (checked above); it writes the planes of `into`,
    // which has memory of its format and size, at its strides.
    let rows = unsafe {
        let into = into.as_mut_ptr();
        sws_scale(
            scaler.0.as_mut_ptr(),
            data.as_ptr(),
            strides.as_ptr(),
            0,
            height,
            (*into).data.as_ptr(),
            (*into).linesize.as_ptr(),
        )
    };
    if rows < 0 {
        return Err(format!(
            "the picture cannot be converted: {}",
            error_text(rows)
        ));
    }
    Ok(())
}
