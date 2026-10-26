//! Every unsafe call of the crate: the devices of the graphics card and the decoders that use
//! them, the copying of a frame from the card, seeking, the index of a stream, and the fields of
//! libav that `ffmpeg-next` has no safe way to reach.

use std::ffi::{c_int, c_void, CStr, CString};

use ff::ffi::{
    av_buffer_ref, av_buffer_unref, av_frame_unref, av_hwdevice_ctx_create,
    av_hwdevice_find_type_by_name, av_hwframe_transfer_data, av_pix_fmt_desc_get, av_seek_frame,
    av_strerror, avcodec_default_get_format, avcodec_find_decoder, avcodec_get_hw_config,
    avformat_index_get_entries_count, avformat_index_get_entry_from_timestamp, AVBufferRef,
    AVCodecContext, AVDiscard, AVHWDeviceType, AVPixelFormat, AVSEEK_FLAG_BACKWARD,
    AV_CODEC_HW_CONFIG_METHOD_HW_DEVICE_CTX, AV_PIX_FMT_FLAG_HWACCEL, FF_THREAD_FRAME,
    FF_THREAD_SLICE,
};
use ffmpeg_next as ff;

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

    /// The format of the frames that a decoder of `codec` gives on this device, if it decodes
    /// on it.
    pub fn format_of(&self, codec: ff::codec::Id) -> Option<ff::format::Pixel> {
        // SAFETY: avcodec_find_decoder gives a decoder that lives as long as the program, or
        // null; avcodec_get_hw_config gives each of its configurations in turn, then null.
        unsafe {
            let decoder = avcodec_find_decoder(codec.into());
            if decoder.is_null() {
                return None;
            }
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
    pub fn attach(&self, context: &mut ff::codec::Context, format: ff::format::Pixel, extra: i32) {
        let format: AVPixelFormat = format.into();
        // SAFETY: the context is not open, so these may be set; the context takes a reference
        // of its own to the device, and lets it go when it is freed. `opaque` is not used by
        // ffmpeg-next, and carries the format for `choose_format`.
        unsafe {
            let context = context.as_mut_ptr();
            (*context).hw_device_ctx = av_buffer_ref(self.buffer);
            (*context).opaque = format as c_int as isize as *mut c_void;
            (*context).get_format = Some(choose_format);
            (*context).extra_hw_frames = extra;
        }
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

/// Copy a frame on a device into memory, as the format that the device keeps its frames in
/// (NV12 for 8 bits, P010 for 10). Where `into` holds a frame of the same size, copied there
/// before and not shared, it is written over, so that no new memory is touched; else it is
/// emptied first, and given new memory.
pub(crate) fn transfer(into: &mut ff::frame::Video, from: &ff::frame::Video) -> Result<(), String> {
    // SAFETY: both frames are frames of libav that this side owns. A frame that holds memory
    // and is the size of `from` was given it by an earlier copy (`into` is only ever filled
    // here), and is shared with no one, since what is copied is copied out of it again before
    // the next frame; av_hwframe_transfer_data writes into it, in its format, or fails. Else
    // `into` is emptied, and av_hwframe_transfer_data gives it buffers of its own.
    let code = unsafe {
        let (into, from) = (into.as_mut_ptr(), from.as_ptr());
        let reused = !(*into).buf[0].is_null()
            && ((*into).width, (*into).height) == ((*from).width, (*from).height)
            && av_hwframe_transfer_data(into, from, 0) >= 0;
        if reused {
            0
        } else {
            av_frame_unref(into);
            av_hwframe_transfer_data(into, from, 0)
        }
    };
    if code < 0 {
        return Err(format!(
            "the frame cannot be copied from the graphics card: {}",
            error_text(code)
        ));
    }
    Ok(())
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

/// How many entries the index of a stream has.
pub(crate) fn index_entries(input: &ff::format::context::Input, stream: usize) -> i32 {
    let Some(stream) = input.stream(stream) else {
        return 0;
    };
    // SAFETY: the stream is of an open context.
    unsafe { avformat_index_get_entries_count(stream.as_ptr()) }
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
        }
    }
}

extern "C" {
    /// `av_pix_fmt_desc_get`, which takes any number and gives null for one that is not a
    /// format: declared here to take the number as the integer that it is, so that no number
    /// is taken for a variant of `AVPixelFormat` that it is not.
    #[link_name = "av_pix_fmt_desc_get"]
    fn descriptor_of_number(format: c_int) -> *const ff::ffi::AVPixFmtDescriptor;
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

// SAFETY: a context of swscale is used by one thread at a time, which `&mut` sees to; it holds
// nothing of the thread that made it.
unsafe impl Send for Scaler {}
