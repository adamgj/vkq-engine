//! The WAV streaming-music codec (Quake/snd_wave.c, Phase 4 M7) as a Rust
//! native behind the same `snd_codec_t` vtable. IO stays the C's: raw stdio
//! reads on the stream's FILE plus Sys_fseek/Sys_ftell, with FS_rewind for
//! the rewind entry.

use core::ffi::{c_int, c_void};

use quake_c_sys as sys;
use sys::{qboolean, snd_codec_t, snd_stream_t, FILE};

use crate::snd_codec::CODECTYPE_WAV;

const SEEK_CUR: c_int = 1;

unsafe fn fread(buf: *mut c_void, size: usize, n: usize, f: *mut FILE) -> usize {
    // SAFETY: forwarded stdio contract
    unsafe { sys::stdio::fread(buf, size, n, f) }
}

unsafe fn fget_le32(f: *mut FILE) -> i32 {
    let mut v: i32 = 0;
    // SAFETY: 4-byte read into v; short read returns 0 like the C
    unsafe {
        if fread(core::ptr::addr_of_mut!(v).cast(), 1, 4, f) != 4 {
            return 0;
        }
    }
    v.to_le() // LittleLong: identity on LE targets
}

unsafe fn fget_le16(f: *mut FILE) -> i16 {
    let mut v: i16 = 0;
    // SAFETY: 2-byte read into v; short read returns 0 like the C
    unsafe {
        if fread(core::ptr::addr_of_mut!(v).cast(), 1, 2, f) != 2 {
            return 0;
        }
    }
    v.to_le()
}

/// WAV_ReadChunkInfo: returns the chunk data length or -1
unsafe fn read_chunk_info(f: *mut FILE, name: &mut [u8; 4]) -> i32 {
    // SAFETY: 4-byte id read + length field
    unsafe {
        if fread(name.as_mut_ptr().cast(), 1, 4, f) != 4 {
            return -1;
        }
        let len = fget_le32(f);
        if len < 0 {
            sys::Con_Printf(c"WAV: Negative chunk length\n".as_ptr());
            return -1;
        }
        len
    }
}

/// WAV_FindRIFFChunk: length of the chunk's data, or -1 if not found
unsafe fn find_riff_chunk(f: *mut FILE, chunk: &[u8; 4]) -> i32 {
    // SAFETY: sequential reads/seeks like the C
    unsafe {
        let mut name = [0u8; 4];
        loop {
            let len = read_chunk_info(f, &mut name);
            if len < 0 {
                return -1;
            }
            if &name == chunk {
                return len;
            }
            let skip = (len + 1) & !1; // pad by 2
            sys::Sys_fseek(f, skip as sys::qfileofs_t, SEEK_CUR);
        }
    }
}

/// WAV_ReadRIFFHeader
unsafe fn read_riff_header(stream: &mut snd_stream_t) -> bool {
    // SAFETY: mirrors the C reads and messages exactly
    unsafe {
        let f = stream.fh.file;
        let name = stream.name.as_ptr();
        let info = &mut stream.info;

        let mut dump = [0u8; 12];
        if fread(dump.as_mut_ptr().cast(), 1, 12, f) < 12
            || &dump[0..4] != b"RIFF"
            || &dump[8..12] != b"WAVE"
        {
            sys::Con_Printf(c"%s is missing RIFF/WAVE chunks\n".as_ptr(), name);
            return false;
        }

        // Scan for the format chunk
        let mut fmtlen = find_riff_chunk(f, b"fmt ");
        if fmtlen < 0 {
            sys::Con_Printf(c"%s is missing fmt chunk\n".as_ptr(), name);
            return false;
        }

        // Save the parameters
        let wav_format = fget_le16(f) as i32;
        if wav_format != 1 {
            // WAV_FORMAT_PCM
            sys::Con_Printf(c"%s is not Microsoft PCM format\n".as_ptr(), name);
            return false;
        }

        info.channels = fget_le16(f) as c_int;
        info.rate = fget_le32(f);
        fget_le32(f);
        fget_le16(f);
        info.bits = fget_le16(f) as c_int;

        if info.bits != 8 && info.bits != 16 {
            sys::Con_Printf(c"%s is not 8 or 16 bit\n".as_ptr(), name);
            return false;
        }

        info.width = info.bits / 8;
        info.dataofs = 0;

        // Skip the rest of the format chunk if required
        if fmtlen > 16 {
            fmtlen -= 16;
            sys::Sys_fseek(f, fmtlen as sys::qfileofs_t, SEEK_CUR);
        }

        // Scan for the data chunk
        info.size = find_riff_chunk(f, b"data");
        if info.size < 0 {
            sys::Con_Printf(c"%s is missing data chunk\n".as_ptr(), name);
            return false;
        }

        if info.channels != 1 && info.channels != 2 {
            sys::Con_Printf(
                c"Unsupported number of channels %d in %s\n".as_ptr(),
                info.channels,
                name,
            );
            return false;
        }
        info.samples = (info.size / info.width) / info.channels;
        if info.samples == 0 {
            sys::Con_Printf(c"%s has zero samples\n".as_ptr(), name);
            return false;
        }

        true
    }
}

unsafe extern "C" fn wav_open(stream: *mut snd_stream_t) -> qboolean {
    // SAFETY: mirrors S_WAV_CodecOpenStream (note the C truncates start to
    // long -- 64-bit on the unix targets, matching qfileofs_t there)
    unsafe {
        let start = (*stream).fh.start;

        if !read_riff_header(&mut *stream) {
            return false;
        }

        (*stream).fh.start = sys::Sys_ftell((*stream).fh.file); // reset to data position
        if (*stream).fh.start - start + (*stream).info.size as sys::qfileofs_t
            > (*stream).fh.length
        {
            sys::Con_Printf(c"%s data size mismatch\n".as_ptr(), (*stream).name.as_ptr());
            return false;
        }

        true
    }
}

/// exported non-static in the C for historical reasons; kept callable
unsafe extern "C" fn wav_read(stream: *mut snd_stream_t, bytes: c_int, buffer: *mut c_void) -> c_int {
    // SAFETY: mirrors S_WAV_CodecReadStream
    unsafe {
        let stream = &mut *stream;
        let remaining = stream.info.size - stream.fh.pos as c_int;

        if remaining <= 0 {
            return 0;
        }
        let bytes = bytes.min(remaining);
        stream.fh.pos += bytes as sys::qfileofs_t;
        if fread(buffer, 1, bytes as usize, stream.fh.file) != bytes as usize {
            return 0;
        }
        if stream.info.width == 2 {
            // LittleShort pass over the samples: identity on LE targets
        }
        bytes
    }
}

unsafe extern "C" fn wav_close(stream: *mut snd_stream_t) {
    // SAFETY: mirrors S_WAV_CodecCloseStream
    unsafe {
        let mut s = stream;
        crate::snd_codec::S_CodecUtilClose(&mut s);
    }
}

unsafe extern "C" fn wav_rewind(stream: *mut snd_stream_t) -> c_int {
    // SAFETY: FS_rewind on the embedded handle, like the C
    unsafe {
        sys::FS_rewind(&mut (*stream).fh);
    }
    0
}

unsafe extern "C" fn wav_initialize() -> qboolean {
    true
}

unsafe extern "C" fn wav_shutdown() {}

pub static mut WAV_CODEC: snd_codec_t = snd_codec_t {
    type_: CODECTYPE_WAV,
    initialized: true, // always available
    ext: c"wav".as_ptr(),
    initialize: Some(wav_initialize),
    shutdown: Some(wav_shutdown),
    codec_open: Some(wav_open),
    codec_read: Some(wav_read),
    codec_rewind: Some(wav_rewind),
    codec_jump: None,
    codec_close: Some(wav_close),
    next: core::ptr::null_mut(),
};
