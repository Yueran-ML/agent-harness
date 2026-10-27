//! Every unsafe call of the crate: the devices of the graphics card and the decoders that use
//! them, the copying of a frame from the card into memory of a picture, converting a frame into
//! it, converting sound, references to frames, seeking, the index of a stream, and the fields of
//! libav that `ffmpeg-next` has no safe way to reach.

use std::ffi::{c_int, c_void, CStr, CString};

use ff::ffi::{
    av_buffer_create, av_buffer_ref, av_buffer_unref, av_channel_layout_check,
    av_channel_layout_compare, av_channel_layout_copy, av_channel_layout_default,
    av_channel_layout_uninit, av_frame_alloc, av_frame_free, av_frame_ref, av_frame_unref,
    av_hwdevice_ctx_create, av_hwdevice_find_type_by_name, av_hwframe_transfer_data,
    av_pix_fmt_desc_get, av_seek_frame, av_strerror, avcodec_default_get_format,
    avcodec_get_hw_config, avformat_index_get_entries_count,
    avformat_index_get_entry_from_timestamp, swr_alloc_set_opts2, swr_convert, swr_free,
    swr_get_out_samples, swr_init, sws_scale, AVBufferRef, AVChannelLayout, AVChannelOrder,
    AVCodecContext, AVDiscard, AVHWDeviceType, AVHWFramesContext, AVPixelFormat, AVSampleFormat,
    SwrContext, AVSEEK_FLAG_BACKWARD, AVSEEK_FLAG_BYTE, AV_CODEC_HW_CONFIG_METHOD_HW_DEVICE_CTX,
    AV_PIX_FMT_FLAG_HWACCEL, FF_THREAD_FRAME, FF_THREAD_SLICE,
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
/// NV12 or P010: one copy, with no frame in between. The planes are long enough for the size of
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

/// How many entries the index of a stream has.
pub(crate) fn index_entries(input: &ff::format::context::Input, stream: usize) -> i32 {
    let Some(stream) = input.stream(stream) else {
        return 0;
    };
    // SAFETY: the stream is of an open context.
    unsafe { avformat_index_get_entries_count(stream.as_ptr()) }
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

    /// Convert a frame, or, given none, what the converter holds back, and add the samples to
    /// `out`, interleaved.
    pub fn convert(
        &mut self,
        frame: Option<&ff::frame::Audio>,
        out: &mut Vec<f32>,
    ) -> Result<(), String> {
        let samples = match frame {
            Some(frame) => {
                c_int::try_from(frame.samples()).map_err(|_| "a frame too long".to_owned())?
            }
            None => 0,
        };
        // SAFETY: the context is set up. The frame, if there is one, is of the format, rate
        // and layout that it takes (`takes`), and its `extended_data` points to a plane of
        // `samples` samples for each channel, or one plane of all of them interleaved. `out`
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
            let input = match frame {
                Some(frame) => (*frame.as_ptr())
                    .extended_data
                    .cast::<*const u8>()
                    .cast_const(),
                None => std::ptr::null(),
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
