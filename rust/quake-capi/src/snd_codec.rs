//! The streaming-codec framework (Quake/snd_codec.c, Phase 4 M7, ADR-014).
//!
//! The registry works directly on `snd_codec_t` vtable structs -- the exact
//! mirror ADR-014 calls for: registration order is preserved (it is the
//! lookup-preference order), stream forwarding works unchanged, and the C
//! decoder wrappers (snd_mp3/snd_mpg123/snd_vorbis/snd_opus/snd_flac.c,
//! which stay C behind their libraries) plug in through the very statics
//! they already export. The WAV codec (and UMX, when built) are Rust
//! natives providing the same vtable shape.

use core::ffi::{c_char, c_int, c_uint, c_void};

use quake_c_sys as sys;
use sys::{qboolean, snd_codec_t, snd_stream_t};

pub const CODECTYPE_NONE: c_uint = 0;
pub const CODECTYPE_MOD: c_uint = 1 << 1;
pub const CODECTYPE_WAV: c_uint = 1 << 3;
pub const CODECTYPE_MP3: c_uint = 1 << 4;
pub const CODECTYPE_UMX: c_uint = 1 << 7;

// SAFETY invariant: the registry is built once in S_CodecInit and read on
// the main thread / under snd_mutex afterwards, like the C file-static.
static mut CODECS: *mut snd_codec_t = core::ptr::null_mut();

fn codecs() -> *mut snd_codec_t {
    // SAFETY: main-thread discipline (see CODECS)
    unsafe { CODECS }
}

/// S_CodecRegister (file-internal in C)
unsafe fn register(codec: *mut snd_codec_t) {
    // SAFETY: init-time only; prepends like the C
    unsafe {
        (*codec).next = CODECS;
        CODECS = codec;
    }
}

/// C: `void S_CodecInit (void)`
///
/// # Safety
/// Main thread, once per S_Init.
#[no_mangle]
pub unsafe extern "C" fn S_CodecInit() {
    // SAFETY: registration order matches snd_codec.c exactly ("in the
    // inverse order of codec choice preference"); each *_codec is linked
    // only when its USE_CODEC_* / cargo feature is on
    unsafe {
        CODECS = core::ptr::null_mut();

        #[cfg(feature = "codec-umx")]
        register(core::ptr::addr_of_mut!(crate::snd_umx::UMX_CODEC));
        #[cfg(feature = "codec-wave")]
        register(core::ptr::addr_of_mut!(crate::snd_wave::WAV_CODEC));
        #[cfg(feature = "codec-flac")]
        register(core::ptr::addr_of_mut!(sys::flac_codec));
        #[cfg(feature = "codec-mp3")]
        register(core::ptr::addr_of_mut!(sys::mp3_codec));
        #[cfg(feature = "codec-vorbis")]
        register(core::ptr::addr_of_mut!(sys::vorbis_codec));
        #[cfg(feature = "codec-opus")]
        register(core::ptr::addr_of_mut!(sys::opus_codec));

        let mut codec = CODECS;
        while !codec.is_null() {
            if let Some(init) = (*codec).initialize {
                init();
            }
            codec = (*codec).next;
        }
    }
}

/// C: `void S_CodecShutdown (void)`
///
/// # Safety
/// Main thread.
#[no_mangle]
pub unsafe extern "C" fn S_CodecShutdown() {
    // SAFETY: walks the registry like the C
    unsafe {
        let mut codec = CODECS;
        while !codec.is_null() {
            if let Some(shutdown) = (*codec).shutdown {
                shutdown();
            }
            codec = (*codec).next;
        }
        CODECS = core::ptr::null_mut();
    }
}

unsafe fn find_by_type(type_: c_uint) -> *mut snd_codec_t {
    // SAFETY: registry walk
    unsafe {
        let mut codec = codecs();
        while !codec.is_null() {
            if type_ == (*codec).type_ {
                break;
            }
            codec = (*codec).next;
        }
        codec
    }
}

fn eq_ignore_ascii(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b.iter())
            .all(|(x, y)| x.eq_ignore_ascii_case(y))
}

unsafe fn find_by_ext(ext: *const c_char) -> *mut snd_codec_t {
    // SAFETY: registry walk with q_strcasecmp semantics (ASCII)
    unsafe {
        let extb = core::ffi::CStr::from_ptr(ext).to_bytes();
        let mut codec = codecs();
        while !codec.is_null() {
            let ce = core::ffi::CStr::from_ptr((*codec).ext).to_bytes();
            if eq_ignore_ascii(extb, ce) {
                break;
            }
            codec = (*codec).next;
        }
        codec
    }
}

unsafe fn open_with(
    filename: *const c_char,
    codec: *mut snd_codec_t,
    loop_: qboolean,
) -> *mut snd_stream_t {
    // SAFETY: mirrors the shared open sequence of the three OpenStream fns
    unsafe {
        let stream = S_CodecUtilOpen(filename, codec, loop_);
        if !stream.is_null() {
            if (*codec).codec_open.map(|f| f(stream)).unwrap_or(false) {
                (*stream).status = sys::stream_status_t_STREAM_PLAY;
            } else {
                let mut s = stream;
                S_CodecUtilClose(&mut s);
                return s; // NULL
            }
        }
        stream
    }
}

/// C: `snd_stream_t *S_CodecOpenStreamType (const char *filename, unsigned int type, qboolean loop)`
///
/// # Safety
/// `filename` NUL-terminated; main thread.
#[no_mangle]
pub unsafe extern "C" fn S_CodecOpenStreamType(
    filename: *const c_char,
    type_: c_uint,
    loop_: qboolean,
) -> *mut snd_stream_t {
    // SAFETY: messages match snd_codec.c byte for byte
    unsafe {
        if type_ == CODECTYPE_NONE {
            sys::Con_Printf(c"Bad type for %s\n".as_ptr(), filename);
            return core::ptr::null_mut();
        }
        let codec = find_by_type(type_);
        if codec.is_null() {
            sys::Con_Printf(c"Unknown type for %s\n".as_ptr(), filename);
            return core::ptr::null_mut();
        }
        open_with(filename, codec, loop_)
    }
}

/// C: `snd_stream_t *S_CodecOpenStreamExt (const char *filename, qboolean loop)`
///
/// # Safety
/// `filename` NUL-terminated; main thread.
#[no_mangle]
pub unsafe extern "C" fn S_CodecOpenStreamExt(
    filename: *const c_char,
    loop_: qboolean,
) -> *mut snd_stream_t {
    // SAFETY: mirrors the C incl. messages
    unsafe {
        let ext = sys::COM_FileGetExtension(filename);
        if *ext == 0 {
            sys::Con_Printf(c"No extension for %s\n".as_ptr(), filename);
            return core::ptr::null_mut();
        }
        let codec = find_by_ext(ext);
        if codec.is_null() {
            sys::Con_Printf(c"Unknown extension for %s\n".as_ptr(), filename);
            return core::ptr::null_mut();
        }
        open_with(filename, codec, loop_)
    }
}

/// C: `snd_stream_t *S_CodecOpenStreamAny (const char *filename, qboolean loop)`
///
/// # Safety
/// `filename` NUL-terminated; main thread.
#[no_mangle]
pub unsafe extern "C" fn S_CodecOpenStreamAny(
    filename: *const c_char,
    loop_: qboolean,
) -> *mut snd_stream_t {
    // SAFETY: mirrors the C incl. the try-all-extensions path
    unsafe {
        let ext = sys::COM_FileGetExtension(filename);
        if *ext == 0 {
            // try all available
            let nameb = core::ffi::CStr::from_ptr(filename).to_bytes();
            let mut codec = codecs();
            while !codec.is_null() {
                // q_snprintf (tmp, MAX_QPATH, "%s.%s", filename, codec->ext)
                let extb = core::ffi::CStr::from_ptr((*codec).ext).to_bytes();
                let mut tmp = [0u8; 64];
                for (n, &b) in nameb
                    .iter()
                    .chain(b".".iter())
                    .chain(extb.iter())
                    .take(63)
                    .enumerate()
                {
                    tmp[n] = b;
                }
                let stream = S_CodecUtilOpen(tmp.as_ptr().cast(), codec, loop_);
                if !stream.is_null() {
                    if (*codec).codec_open.map(|f| f(stream)).unwrap_or(false) {
                        (*stream).status = sys::stream_status_t_STREAM_PLAY;
                        return stream;
                    }
                    let mut s = stream;
                    S_CodecUtilClose(&mut s);
                }
                codec = (*codec).next;
            }
            core::ptr::null_mut()
        } else {
            // use the name as is
            let codec = find_by_ext(ext);
            if codec.is_null() {
                sys::Con_Printf(c"Unknown extension for %s\n".as_ptr(), filename);
                return core::ptr::null_mut();
            }
            open_with(filename, codec, loop_)
        }
    }
}

/// C: `qboolean S_CodecForwardStream (snd_stream_t *stream, unsigned int type)`
///
/// # Safety
/// `stream` valid.
#[no_mangle]
pub unsafe extern "C" fn S_CodecForwardStream(
    stream: *mut snd_stream_t,
    type_: c_uint,
) -> qboolean {
    // SAFETY: mirrors the C forwarding
    unsafe {
        let codec = find_by_type(type_);
        if codec.is_null() {
            return false;
        }
        (*stream).codec = codec;
        (*codec).codec_open.map(|f| f(stream)).unwrap_or(false)
    }
}

/// C: `void S_CodecCloseStream (snd_stream_t *stream)`
///
/// # Safety
/// `stream` valid and open.
#[no_mangle]
pub unsafe extern "C" fn S_CodecCloseStream(stream: *mut snd_stream_t) {
    // SAFETY: vtable dispatch like the C
    unsafe {
        (*stream).status = sys::stream_status_t_STREAM_NONE;
        if let Some(close) = (*(*stream).codec).codec_close {
            close(stream);
        }
    }
}

/// C: `int S_CodecRewindStream (snd_stream_t *stream)`
///
/// # Safety
/// `stream` valid and open.
#[no_mangle]
pub unsafe extern "C" fn S_CodecRewindStream(stream: *mut snd_stream_t) -> c_int {
    // SAFETY: vtable dispatch
    unsafe {
        (*(*stream).codec)
            .codec_rewind
            .map(|f| f(stream))
            .unwrap_or(-1)
    }
}

/// C: `int S_CodecJumpToOrder (snd_stream_t *stream, int to)`
///
/// # Safety
/// `stream` valid and open.
#[no_mangle]
pub unsafe extern "C" fn S_CodecJumpToOrder(stream: *mut snd_stream_t, to: c_int) -> c_int {
    // SAFETY: vtable dispatch; NULL jump returns -1 like the C
    unsafe {
        match (*(*stream).codec).codec_jump {
            Some(jump) => jump(stream, to),
            None => -1,
        }
    }
}

/// C: `int S_CodecReadStream (snd_stream_t *stream, int bytes, void *buffer)`
///
/// # Safety
/// `buffer` valid for `bytes`.
#[no_mangle]
pub unsafe extern "C" fn S_CodecReadStream(
    stream: *mut snd_stream_t,
    bytes: c_int,
    buffer: *mut c_void,
) -> c_int {
    // SAFETY: vtable dispatch
    unsafe {
        (*(*stream).codec)
            .codec_read
            .map(|f| f(stream, bytes, buffer))
            .unwrap_or(0)
    }
}

/// C: `snd_stream_t *S_CodecUtilOpen (const char *filename, snd_codec_t *codec, qboolean loop)`
///
/// # Safety
/// `filename` NUL-terminated.
#[no_mangle]
pub unsafe extern "C" fn S_CodecUtilOpen(
    filename: *const c_char,
    codec: *mut snd_codec_t,
    loop_: qboolean,
) -> *mut snd_stream_t {
    // SAFETY: mirrors the C open (Mem_Alloc zeroes; ADR-013 ownership)
    unsafe {
        let mut handle: *mut sys::FILE = core::ptr::null_mut();
        let length = sys::COM_FOpenFile(filename, &mut handle, core::ptr::null_mut());
        let pak = sys::COM_ThreadFileFromPak() != 0;
        if length == -1 {
            sys::Con_DPrintf(c"Couldn't open %s\n".as_ptr(), filename);
            return core::ptr::null_mut();
        }

        let stream: *mut snd_stream_t = sys::Mem_Alloc(core::mem::size_of::<snd_stream_t>()).cast();
        (*stream).codec = codec;
        (*stream).loop_ = loop_;
        (*stream).fh.file = handle;
        (*stream).fh.start = sys::Sys_ftell(handle);
        (*stream).fh.pos = 0;
        (*stream).fh.length = length;
        (*stream).fh.pak = pak;
        (*stream).pak = pak;
        // q_strlcpy (stream->name, filename, MAX_QPATH)
        let nameb = core::ffi::CStr::from_ptr(filename).to_bytes();
        let n = nameb.len().min(63);
        for (i, &b) in nameb[..n].iter().enumerate() {
            (*stream).name[i] = b as c_char;
        }
        (*stream).name[n] = 0;
        stream
    }
}

/// C: `void S_CodecUtilClose (snd_stream_t **stream)`
///
/// # Safety
/// `stream` points at a valid open stream pointer.
#[no_mangle]
pub unsafe extern "C" fn S_CodecUtilClose(stream: *mut *mut snd_stream_t) {
    // SAFETY: mirrors the C close/free/null sequence
    unsafe {
        sys::stdio::fclose((**stream).fh.file);
        sys::Mem_Free((*stream).cast());
        *stream = core::ptr::null_mut();
    }
}

/// C: `int S_CodecIsAvailable (unsigned int type)`
///
/// # Safety
/// Main thread.
#[no_mangle]
pub unsafe extern "C" fn S_CodecIsAvailable(type_: c_uint) -> c_int {
    // SAFETY: registry walk
    unsafe {
        let mut codec = codecs();
        while !codec.is_null() {
            if type_ == (*codec).type_ {
                return (*codec).initialized as c_int;
            }
            codec = (*codec).next;
        }
        -1
    }
}
