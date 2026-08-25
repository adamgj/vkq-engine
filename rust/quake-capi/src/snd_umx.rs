//! Unreal UMX container support (Quake/snd_umx.c, Phase 4 M7): parse the
//! upkg header/export/name tables, locate the embedded music object, then
//! hack the stream's start/length and forward it to the matching codec --
//! exactly the C flow. Built only under the `codec-umx` cargo feature,
//! mirroring USE_CODEC_UMX (off in every Meson config; the legacy Makefile
//! can enable it).

use core::ffi::{c_char, c_int, c_void};

use quake_c_sys as sys;
use sys::{fshandle_t, qboolean, snd_codec_t, snd_stream_t};

use crate::snd_codec::{CODECTYPE_MOD, CODECTYPE_MP3, CODECTYPE_UMX, CODECTYPE_WAV};

const SEEK_SET: c_int = 0;

const UPKG_HDR_TAG: u32 = 0x9e2a83c1;

const UMUSIC_IT: c_int = 0;
const UMUSIC_S3M: c_int = 1;
const UMUSIC_XM: c_int = 2;
const UMUSIC_MOD: c_int = 3;
const UMUSIC_WAV: c_int = 4;
const UMUSIC_MP2: c_int = 5;

static MUSTYPE: [&[u8]; 6] = [b"IT", b"S3M", b"XM", b"MOD", b"WAV", b"MP2"];
static MUSTYPE_NAMES: [&core::ffi::CStr; 6] = [c"IT", c"S3M", c"XM", c"MOD", c"WAV", c"MP2"];

/// the byte-swapped 64-byte upkg header prefix
#[derive(Default, Clone, Copy)]
struct UpkgHdr {
    tag: u32,
    file_version: i32,
    name_count: i32,
    name_offset: i32,
    export_count: i32,
    export_offset: i32,
    import_count: i32,
    import_offset: i32,
}

/// decode an FCompactIndex (signed char reads, exactly the C's)
fn get_fci(input: &[u8], pos: &mut c_int) -> i32 {
    let b = |i: usize| input[i] as i8;
    let mut size = 1;
    let mut a: i32 = (b(0) & 0x3f) as i32;

    if b(0) & 0x40 != 0 {
        size += 1;
        a |= ((b(1) & 0x7f) as i32) << 6;
        if (b(1) as u8) & 0x80 != 0 {
            size += 1;
            a |= ((b(2) & 0x7f) as i32) << 13;
            if (b(2) as u8) & 0x80 != 0 {
                size += 1;
                a |= ((b(3) & 0x7f) as i32) << 20;
                if (b(3) as u8) & 0x80 != 0 {
                    size += 1;
                    a |= ((b(4) & 0x3f) as i32) << 27;
                }
            }
        }
    }

    if (b(0) as u8) & 0x80 != 0 {
        a = -a;
    }

    *pos += size;
    a
}

unsafe fn fs_seek(f: *mut fshandle_t, ofs: i64, whence: c_int) -> c_int {
    // SAFETY: forwarded FS contract
    unsafe { sys::FS_fseek(f, ofs, whence) }
}

unsafe fn fs_read(f: *mut fshandle_t, buf: *mut c_void, size: usize, n: usize) -> usize {
    // SAFETY: forwarded FS contract
    unsafe { sys::FS_fread(buf, size, n, f) }
}

unsafe fn get_objtype(f: *mut fshandle_t, ofs: i32, mut type_: c_int) -> c_int {
    // SAFETY: fixed-size reads at file offsets, mirroring the C probes
    unsafe {
        let mut sig: [u8; 16];
        loop {
            sig = [0; 16];
            fs_seek(f, ofs as i64, SEEK_SET);
            fs_read(f, sig.as_mut_ptr().cast(), 16, 1);
            if type_ == UMUSIC_IT {
                return if &sig[0..4] == b"IMPM" { UMUSIC_IT } else { -1 };
            }
            if type_ == UMUSIC_XM {
                if &sig[0..16] != b"Extended Module:" {
                    return -1;
                }
                fs_read(f, sig.as_mut_ptr().cast(), 16, 1);
                if sig[0] != b' ' {
                    return -1;
                }
                fs_read(f, sig.as_mut_ptr().cast(), 16, 1);
                if sig[5] != 0x1a {
                    return -1;
                }
                return UMUSIC_XM;
            }
            if type_ == UMUSIC_MP2 {
                let u = ((sig[0] as u16) << 8 | sig[1] as u16) & 0xFFFE;
                return if u == 0xFFFC || u == 0xFFF4 {
                    UMUSIC_MP2
                } else {
                    -1
                };
            }
            if type_ == UMUSIC_WAV {
                return if &sig[0..4] == b"RIFF" && &sig[8..12] == b"WAVE" {
                    UMUSIC_WAV
                } else {
                    -1
                };
            }

            fs_seek(f, ofs as i64 + 44, SEEK_SET);
            fs_read(f, sig.as_mut_ptr().cast(), 4, 1);
            if type_ == UMUSIC_S3M {
                if &sig[0..4] == b"SCRM" {
                    return UMUSIC_S3M;
                }
                // SpaceMarines.umx / Starseek.umx report "s3m" but are "it"
                type_ = UMUSIC_IT;
                continue;
            }

            fs_seek(f, ofs as i64 + 1080, SEEK_SET);
            fs_read(f, sig.as_mut_ptr().cast(), 4, 1);
            if type_ == UMUSIC_MOD {
                return if &sig[0..4] == b"M.K." || &sig[0..4] == b"M!K!" {
                    UMUSIC_MOD
                } else {
                    -1
                };
            }

            return -1;
        }
    }
}

unsafe fn read_export(
    f: *mut fshandle_t,
    hdr: &UpkgHdr,
    ofs: &mut i32,
    objsize: &mut i32,
) -> c_int {
    // SAFETY: 40-byte read at *ofs
    unsafe {
        let mut buf = [0u8; 40];
        fs_seek(f, *ofs as i64, SEEK_SET);
        if fs_read(f, buf.as_mut_ptr().cast(), 4, 10) < 10 {
            return -1;
        }

        let mut idx: c_int = 0;
        if hdr.file_version < 40 {
            idx += 8;
        }
        if hdr.file_version < 60 {
            idx += 16;
        }
        get_fci(&buf[idx as usize..], &mut idx); // skip junk
        let t;
        {
            let start = idx as usize;
            let mut local = idx;
            t = get_fci(&buf[start..], &mut local); // type_name
            idx = local;
        }
        if hdr.file_version > 61 {
            idx += 4; // skip export size
        }
        {
            let start = idx as usize;
            let mut local = idx;
            *objsize = get_fci(&buf[start..], &mut local);
            idx = local;
        }
        *ofs += idx;

        t
    }
}

unsafe fn read_typname(f: *mut fshandle_t, hdr: &UpkgHdr, idx: c_int, out: &mut [u8; 64]) -> c_int {
    // SAFETY: bounded name-table reads
    unsafe {
        if idx >= hdr.name_count {
            return -1;
        }
        let mut buf = [0u8; 64];
        let mut l: i64 = 0;
        for _ in 0..=idx {
            if fs_seek(f, hdr.name_offset as i64 + l, SEEK_SET) < 0 {
                return -1;
            }
            if fs_read(f, buf.as_mut_ptr().cast(), 1, 63) == 0 {
                return -1;
            }
            if hdr.file_version >= 64 {
                let s = buf[0] as i8; // numchars *including* terminator
                if s <= 0 {
                    return -1;
                }
                l += s as i64 + 5;
            } else {
                let n = buf.iter().position(|&b| b == 0).unwrap_or(63);
                l += n as i64 + 5;
            }
        }

        // strcpy (out, version >= 64 ? &buf[1] : buf)
        let src: &[u8] = if hdr.file_version >= 64 {
            &buf[1..]
        } else {
            &buf[..]
        };
        let n = src.iter().position(|&b| b == 0).unwrap_or(src.len() - 1);
        out[..n].copy_from_slice(&src[..n]);
        out[n] = 0;
        0
    }
}

unsafe fn probe_umx(f: *mut fshandle_t, hdr: &UpkgHdr, ofs: &mut i32, objsize: &mut i32) -> c_int {
    // SAFETY: bounded export/name table reads, mirroring the C exactly
    unsafe {
        let fsiz = sys::FS_filelength(f) as i64;

        if hdr.name_offset as i64 >= fsiz
            || hdr.export_offset as i64 >= fsiz
            || hdr.import_offset as i64 >= fsiz
        {
            sys::Con_DPrintf(c"Illegal values in header.\n".as_ptr());
            return -1;
        }

        // parse the exports table for the first music object
        let mut buf = [0u8; 64];
        fs_seek(f, hdr.export_offset as i64, SEEK_SET);
        fs_read(f, buf.as_mut_ptr().cast(), 1, 64);

        let mut idx: c_int = 0;
        get_fci(&buf[idx as usize..], &mut idx); // skip class_index
        {
            let start = idx as usize;
            let mut local = idx;
            get_fci(&buf[start..], &mut local); // skip super_index
            idx = local;
        }
        if hdr.file_version >= 60 {
            idx += 4; // skip int32 package_index
        }
        {
            let start = idx as usize;
            let mut local = idx;
            get_fci(&buf[start..], &mut local); // skip object_name
            idx = local;
        }
        idx += 4; // skip int32 object_flags

        let s;
        {
            let start = idx as usize;
            let mut local = idx;
            s = get_fci(&buf[start..], &mut local); // serial_size
            idx = local;
        }
        if s <= 0 {
            return -1;
        }
        let mut pos;
        {
            let start = idx as usize;
            let mut local = idx;
            pos = get_fci(&buf[start..], &mut local); // serial_offset
        }
        if pos < 0 || pos as i64 > fsiz - 40 {
            return -1;
        }

        let mut size = s;
        let t = read_export(f, hdr, &mut pos, &mut size);
        if t < 0 {
            return -1;
        }
        if size <= 0 || size as i64 > fsiz - pos as i64 {
            return -1;
        }

        let mut name = [0u8; 64];
        if read_typname(f, hdr, t, &mut name) < 0 {
            return -1;
        }
        let n = name.iter().position(|&b| b == 0).unwrap_or(64);
        let mut mtype: c_int = -1;
        for (i, m) in MUSTYPE.iter().enumerate() {
            if name[..n].eq_ignore_ascii_case(m) {
                mtype = i as c_int;
                break;
            }
        }
        if mtype < 0 {
            return -1;
        }
        let mtype = get_objtype(f, pos, mtype);
        if mtype < 0 {
            return -1;
        }

        *ofs = pos;
        *objsize = size;
        mtype
    }
}

unsafe fn probe_header(f: *mut fshandle_t, hdr: &mut UpkgHdr) -> i32 {
    // SAFETY: 64-byte header read
    unsafe {
        let mut raw = [0u8; 64];
        if fs_read(f, raw.as_mut_ptr().cast(), 1, 64) < 64 {
            return -1;
        }
        let le32 = |i: usize| i32::from_le_bytes([raw[i], raw[i + 1], raw[i + 2], raw[i + 3]]);
        hdr.tag = le32(0) as u32;
        hdr.file_version = le32(4);
        hdr.name_count = le32(12);
        hdr.name_offset = le32(16);
        hdr.export_count = le32(20);
        hdr.export_offset = le32(24);
        hdr.import_count = le32(28);
        hdr.import_offset = le32(32);

        if hdr.tag != UPKG_HDR_TAG {
            sys::Con_DPrintf(c"Unknown header tag 0x%x\n".as_ptr(), hdr.tag);
            return -1;
        }
        if hdr.name_count < 0
            || hdr.export_count < 0
            || hdr.import_count < 0
            || hdr.name_offset < 36
            || hdr.export_offset < 36
            || hdr.import_offset < 36
        {
            sys::Con_DPrintf(c"Illegal values in header.\n".as_ptr());
            return -1;
        }
        0
    }
}

unsafe fn process_upkg(f: *mut fshandle_t, ofs: &mut i32, objsize: &mut i32) -> c_int {
    let mut header = UpkgHdr::default();
    // SAFETY: forwarded
    unsafe {
        if probe_header(f, &mut header) < 0 {
            return -1;
        }
        probe_umx(f, &header, ofs, objsize)
    }
}

unsafe extern "C" fn umx_initialize() -> qboolean {
    true
}

unsafe extern "C" fn umx_shutdown() {}

unsafe extern "C" fn umx_open(stream: *mut snd_stream_t) -> qboolean {
    // SAFETY: mirrors S_UMX_CodecOpenStream incl. the fh.start/length hack
    unsafe {
        let mut ofs: i32 = 0;
        let mut size: i32 = 0;

        let type_ = process_upkg(&mut (*stream).fh, &mut ofs, &mut size);
        if type_ < 0 {
            sys::Con_DPrintf(c"%s: unrecognized umx\n".as_ptr(), (*stream).name.as_ptr());
            return false;
        }

        sys::Con_DPrintf(
            c"%s: %s data @ 0x%x, %d bytes\n".as_ptr(),
            (*stream).name.as_ptr(),
            MUSTYPE_NAMES[type_ as usize].as_ptr(),
            ofs,
            size,
        );
        // hack the fshandle_t start pos and length members so that only the
        // relevant data is accessed from now on
        (*stream).fh.start += ofs as sys::qfileofs_t;
        (*stream).fh.length = size as sys::qfilesize_t;
        fs_seek(&mut (*stream).fh, 0, SEEK_SET);

        match type_ {
            t if t == UMUSIC_IT || t == UMUSIC_S3M || t == UMUSIC_XM || t == UMUSIC_MOD => {
                crate::snd_codec::S_CodecForwardStream(stream, CODECTYPE_MOD)
            }
            t if t == UMUSIC_WAV => crate::snd_codec::S_CodecForwardStream(stream, CODECTYPE_WAV),
            t if t == UMUSIC_MP2 => crate::snd_codec::S_CodecForwardStream(stream, CODECTYPE_MP3),
            _ => false,
        }
    }
}

unsafe extern "C" fn umx_read(
    _stream: *mut snd_stream_t,
    _bytes: c_int,
    _buffer: *mut c_void,
) -> c_int {
    -1
}

unsafe extern "C" fn umx_close(stream: *mut snd_stream_t) {
    // SAFETY: forwarded close
    unsafe {
        let mut s = stream;
        crate::snd_codec::S_CodecUtilClose(&mut s);
    }
}

unsafe extern "C" fn umx_rewind(_stream: *mut snd_stream_t) -> c_int {
    -1
}

pub static mut UMX_CODEC: snd_codec_t = snd_codec_t {
    type_: CODECTYPE_UMX,
    initialized: true, // always available
    ext: {
        const E: &core::ffi::CStr = c"umx";
        E.as_ptr()
    },
    initialize: Some(umx_initialize),
    shutdown: Some(umx_shutdown),
    codec_open: Some(umx_open),
    codec_read: Some(umx_read),
    codec_rewind: Some(umx_rewind),
    codec_jump: None,
    codec_close: Some(umx_close),
    next: core::ptr::null_mut(),
};

// keep c_char referenced for parity with the C signature surface
#[allow(unused)]
fn _sig(_: *const c_char) {}
