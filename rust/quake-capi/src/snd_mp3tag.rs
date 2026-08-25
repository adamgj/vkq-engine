//! ID3/APE/Lyrics3/MusicMatch tag skipping for MP3 streams
//! (Quake/snd_mp3tag.c, Phase 4 M7). Called by the C decoder wrappers
//! (snd_mp3.c / snd_mpg123.c), which stay C behind their libraries
//! (ADR-014); the tag parsing itself handles untrusted input, hence the
//! Rust port. FS_* IO and fh.start/length manipulation mirror the C
//! byte for byte; `long` arithmetic uses c_long (the C is
//! platform-dependent there, and parity is per-platform per ADR-010).

use core::ffi::{c_int, c_long, c_void};

use quake_c_sys as sys;
use sys::snd_stream_t;

const SEEK_END: c_int = 2;

unsafe fn fs_seek(stream: *mut snd_stream_t, offset: c_long, whence: c_int) -> c_int {
    // SAFETY: forwarded FS contract
    unsafe { sys::FS_fseek(&mut (*stream).fh, offset as sys::qfileofs_t, whence) }
}

unsafe fn fs_read(stream: *mut snd_stream_t, buf: *mut c_void, size: usize, n: usize) -> usize {
    // SAFETY: forwarded FS contract
    unsafe { sys::FS_fread(buf, size, n, &mut (*stream).fh) }
}

fn is_id3v1(data: &[u8]) -> bool {
    // http://id3.org/ID3v1 :  3 bytes "TAG" identifier and 125 bytes tag data
    !(data.len() < 128 || &data[0..3] != b"TAG")
}

fn is_id3v2(data: &[u8]) -> bool {
    // ID3v2 header is 10 bytes: bytes 0-2 "ID3"
    if data.len() < 10 || &data[0..3] != b"ID3" {
        return false;
    }
    // bytes 3-4: version num, each byte always less than 0xff
    if data[3] == 0xff || data[4] == 0xff {
        return false;
    }
    // bytes 6-9: 32 bit 'synchsafe' integer
    if data[6] >= 0x80 || data[7] >= 0x80 || data[8] >= 0x80 || data[9] >= 0x80 {
        return false;
    }
    true
}

fn get_id3v2_len(data: &[u8], length: c_long) -> c_long {
    // size is a 'synchsafe' integer (see above)
    let mut size = ((data[6] as c_long) << 21)
        + ((data[7] as c_long) << 14)
        + ((data[8] as c_long) << 7)
        + data[9] as c_long;
    size += 10; // header size
                // bit 4 of flags: footer present
    if data[5] & 0x10 != 0 {
        size += 10;
    }
    // optional padding (always zeroes)
    while size < length && data[size as usize] == 0 {
        size += 1;
    }
    size
}

fn is_apetag(data: &[u8]) -> bool {
    if data.len() < 32 || &data[0..8] != b"APETAGEX" {
        return false;
    }
    let v = u32::from_le_bytes([data[8], data[9], data[10], data[11]]);
    if v != 2000 && v != 1000 {
        return false;
    }
    // reserved bits must be all zeroes
    if data[24..28] != [0; 4] || data[28..32] != [0; 4] {
        return false;
    }
    true
}

fn get_ape_len(data: &[u8]) -> c_long {
    let mut size = i32::from_le_bytes([data[12], data[13], data[14], data[15]]) as c_long;
    let version = u32::from_le_bytes([data[8], data[9], data[10], data[11]]);
    let flags = u32::from_le_bytes([data[20], data[21], data[22], data[23]]);
    if version == 2000 && (flags & (1 << 31)) != 0 {
        size += 32; // header present
    }
    size
}

fn is_lyrics3tag(data: &[u8]) -> c_int {
    if data.len() < 15 {
        return 0;
    }
    if &data[6..15] == b"LYRICS200" {
        return 2; // v2
    }
    if &data[6..15] == b"LYRICSEND" {
        return 1; // v1
    }
    0
}

unsafe fn get_lyrics3v1_len(stream: *mut snd_stream_t) -> c_long {
    // SAFETY: FS reads bounded by len <= 5109 into buf
    unsafe {
        // needs manual search: http://id3.org/Lyrics3
        if (*stream).fh.length < 20 {
            return -1;
        }
        let mut len: c_long = if (*stream).fh.length > 5109 {
            5109
        } else {
            (*stream).fh.length as c_long
        };
        let mut buf = [0u8; 5104];
        fs_seek(stream, -len, SEEK_END);
        len -= 9; // exclude footer
        fs_read(stream, buf.as_mut_ptr().cast(), 1, len as usize);
        // strstr() won't work here.
        let mut i: c_long = len - 11;
        let mut p: usize = 0;
        while i >= 0 {
            if &buf[p..p + 11] == b"LYRICSBEGIN" {
                break;
            }
            i -= 1;
            p += 1;
        }
        if i < 0 {
            return -1;
        }
        len - p as c_long + 9 // footer
    }
}

fn get_lyrics3v2_len(data: &[u8], length: c_long) -> c_long {
    // 6 bytes before the end marker is size in decimal format
    if length != 6 {
        return 0;
    }
    // strtol(data, NULL, 10): leading spaces, optional sign, decimal digits
    let mut i = 0;
    while i < data.len()
        && (data[i] == b' '
            || data[i] == b'\t'
            || data[i] == b'\n'
            || data[i] == b'\r'
            || data[i] == 0x0b
            || data[i] == 0x0c)
    {
        i += 1;
    }
    let mut neg = false;
    if i < data.len() && (data[i] == b'+' || data[i] == b'-') {
        neg = data[i] == b'-';
        i += 1;
    }
    let mut v: c_long = 0;
    while i < data.len() && data[i].is_ascii_digit() {
        v = v.wrapping_mul(10).wrapping_add((data[i] - b'0') as c_long);
        i += 1;
    }
    if neg {
        v = -v;
    }
    v + 15
}

fn verify_lyrics3v2(data: &[u8]) -> bool {
    data.len() >= 11 && &data[0..11] == b"LYRICSBEGIN"
}

fn q_isdigit(c: u8) -> bool {
    c.is_ascii_digit()
}

fn is_musicmatch(data: &[u8]) -> bool {
    if data.len() < 48 {
        return false;
    }
    // sig: 19 bytes company name + 13 bytes space
    if &data[0..32] != b"Brava Software Inc.             " {
        return false;
    }
    // 4 bytes version: x.xx
    if !q_isdigit(data[32]) || data[33] != b'.' || !q_isdigit(data[34]) || !q_isdigit(data[35]) {
        return false;
    }
    // MMTAG_PARANOID: 12 bytes trailing space
    for &b in &data[36..48] {
        if b != b' ' {
            return false;
        }
    }
    true
}

unsafe fn get_musicmatch_len(stream: *mut snd_stream_t) -> c_long {
    // SAFETY: FS reads into fixed buffers, mirroring the C offsets exactly
    unsafe {
        const METASIZES: [c_long; 4] = [7868, 7936, 8004, 8132];
        const SYNCSTR: [u8; 10] = [b'1', b'8', b'2', b'7', b'3', b'6', b'4', b'5', 0, 0];
        let mut buf = [0u8; 256];

        fs_seek(stream, -68, SEEK_END);
        fs_read(stream, buf.as_mut_ptr().cast(), 1, 20);
        let imgext_ofs = i32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as c_int;
        let version_ofs = i32::from_le_bytes([buf[12], buf[13], buf[14], buf[15]]) as c_int;
        if version_ofs <= imgext_ofs {
            return -1;
        }
        if version_ofs <= 0 || imgext_ofs <= 0 {
            return -1;
        }
        // Try finding the version info section
        let mut len: c_long = 0;
        let mut found = false;
        for &metasize in METASIZES.iter() {
            // 48: footer, 20: offsets, 256: version info
            len = metasize + 48 + 20 + 256;
            if (*stream).fh.length < len as sys::qfilesize_t {
                return -1;
            }
            fs_seek(stream, -len, SEEK_END);
            fs_read(stream, buf.as_mut_ptr().cast(), 1, 256);
            // MMTAG_PARANOID: [30..255] must be 0x20
            if buf[30..256].iter().any(|&b| b != b' ') {
                continue;
            }
            if buf[0..10] == SYNCSTR {
                found = true;
                break;
            }
        }
        if !found {
            return -1; // no luck
        }
        // MMTAG_PARANOID: unused section (4 bytes of 0x00)
        fs_seek(stream, -(len + 4), SEEK_END);
        fs_read(stream, buf.as_mut_ptr().cast(), 1, 4);
        if buf[0..4] != [0; 4] {
            return -1;
        }
        len += (version_ofs - imgext_ofs) as c_long;
        if (*stream).fh.length < len as sys::qfilesize_t {
            return -1;
        }
        fs_seek(stream, -len, SEEK_END);
        fs_read(stream, buf.as_mut_ptr().cast(), 1, 8);
        let j = i32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
        if j < 0 {
            return -1;
        }
        // verify image size: without this, we may land at a wrong place
        if j as c_long + 12 != (version_ofs - imgext_ofs) as c_long {
            return -1;
        }
        // try finding the optional header
        if (*stream).fh.length < (len + 256) as sys::qfilesize_t {
            return len;
        }
        fs_seek(stream, -(len + 256), SEEK_END);
        fs_read(stream, buf.as_mut_ptr().cast(), 1, 256);
        if buf[0..10] != SYNCSTR {
            return len;
        }
        if buf[30..256].iter().any(|&b| b != b' ') {
            return len;
        }
        len + 256 // header is present
    }
}

unsafe fn probe_id3v1(stream: *mut snd_stream_t, buf: &mut [u8; 128], atend: bool) -> c_int {
    // SAFETY: FS reads into buf, fh.length bookkeeping like the C
    unsafe {
        if (*stream).fh.length >= 128 {
            fs_seek(stream, -128, SEEK_END);
            if fs_read(stream, buf.as_mut_ptr().cast(), 1, 128) != 128 {
                return -1;
            }
            if is_id3v1(&buf[..]) {
                if !atend {
                    // possible false positive?
                    if is_musicmatch(&buf[128 - 48..])
                        || is_apetag(&buf[128 - 32..])
                        || is_lyrics3tag(&buf[128 - 15..]) != 0
                    {
                        return 0;
                    }
                }
                (*stream).fh.length -= 128;
                sys::Con_DPrintf(
                    c"MP3: skipped %ld bytes ID3v1 tag\n".as_ptr(),
                    128 as c_long,
                );
                return 1;
            }
        }
        0
    }
}

unsafe fn probe_mmtag(stream: *mut snd_stream_t, buf: &mut [u8; 128]) -> c_int {
    // SAFETY: as above
    unsafe {
        if (*stream).fh.length >= 68 {
            fs_seek(stream, -48, SEEK_END);
            if fs_read(stream, buf.as_mut_ptr().cast(), 1, 48) != 48 {
                return -1;
            }
            if is_musicmatch(&buf[..48]) {
                let len = get_musicmatch_len(stream);
                if len < 0 {
                    return -1;
                }
                if len as sys::qfilesize_t >= (*stream).fh.length {
                    return -1;
                }
                (*stream).fh.length -= len as sys::qfilesize_t;
                sys::Con_DPrintf(c"MP3: skipped %ld bytes MusicMatch tag\n".as_ptr(), len);
                return 1;
            }
        }
        0
    }
}

unsafe fn probe_apetag(stream: *mut snd_stream_t, buf: &mut [u8; 128]) -> c_int {
    // SAFETY: as above
    unsafe {
        if (*stream).fh.length >= 32 {
            fs_seek(stream, -32, SEEK_END);
            if fs_read(stream, buf.as_mut_ptr().cast(), 1, 32) != 32 {
                return -1;
            }
            if is_apetag(&buf[..32]) {
                let len = get_ape_len(&buf[..32]);
                if len as sys::qfilesize_t >= (*stream).fh.length {
                    return -1;
                }
                (*stream).fh.length -= len as sys::qfilesize_t;
                sys::Con_DPrintf(c"MP3: skipped %ld bytes APE tag\n".as_ptr(), len);
                return 1;
            }
        }
        0
    }
}

unsafe fn probe_lyrics3(stream: *mut snd_stream_t, buf: &mut [u8; 128]) -> c_int {
    // SAFETY: as above
    unsafe {
        if (*stream).fh.length >= 15 {
            fs_seek(stream, -15, SEEK_END);
            if fs_read(stream, buf.as_mut_ptr().cast(), 1, 15) != 15 {
                return -1;
            }
            let tag = is_lyrics3tag(&buf[..15]);
            if tag == 2 {
                let len = get_lyrics3v2_len(&buf[..6], 6);
                if len as sys::qfilesize_t >= (*stream).fh.length {
                    return -1;
                }
                if len < 15 {
                    return -1;
                }
                fs_seek(stream, -len, SEEK_END);
                if fs_read(stream, buf.as_mut_ptr().cast(), 1, 11) != 11 {
                    return -1;
                }
                if !verify_lyrics3v2(&buf[..11]) {
                    return -1;
                }
                (*stream).fh.length -= len as sys::qfilesize_t;
                sys::Con_DPrintf(c"MP3: skipped %ld bytes Lyrics3 tag\n".as_ptr(), len);
                return 1;
            } else if tag == 1 {
                let len = get_lyrics3v1_len(stream);
                if len < 0 {
                    return -1;
                }
                (*stream).fh.length -= len as sys::qfilesize_t;
                sys::Con_DPrintf(c"MP3: skipped %ld bytes Lyrics3 tag\n".as_ptr(), len);
                return 1;
            }
        }
        0
    }
}

/// C: `int mp3_skiptags (snd_stream_t *stream)` -- called by the C mp3
/// decoder wrappers.
///
/// # Safety
/// `stream` valid and open.
#[no_mangle]
pub unsafe extern "C" fn mp3_skiptags(stream: *mut snd_stream_t) -> c_int {
    // SAFETY: whole-function mirror of the C incl. its failsafe epilogue
    unsafe {
        let mut buf = [0u8; 128];
        let mut rc: c_int = -1;
        // failsafe
        let oldlength = (*stream).fh.length;
        let oldstart = (*stream).fh.start;

        'fail: {
            let readsize = fs_read(stream, buf.as_mut_ptr().cast(), 1, 128);
            if readsize == 0 || sys::FS_ferror(&mut (*stream).fh) != 0 {
                break 'fail;
            }

            // ID3v2 tag is at the start
            if is_id3v2(&buf[..readsize]) {
                let len = get_id3v2_len(&buf[..], readsize as c_long);
                if len as sys::qfilesize_t >= (*stream).fh.length {
                    break 'fail;
                }
                (*stream).fh.start += len as sys::qfileofs_t;
                (*stream).fh.length -= len as sys::qfilesize_t;
                sys::Con_DPrintf(c"MP3: skipped %ld bytes ID3v2 tag\n".as_ptr(), len);
            }
            // APE tag _might_ be at the start
            else if is_apetag(&buf[..readsize]) {
                let len = get_ape_len(&buf[..]);
                if len as sys::qfilesize_t >= (*stream).fh.length {
                    break 'fail;
                }
                (*stream).fh.start += len as sys::qfileofs_t;
                (*stream).fh.length -= len as sys::qfilesize_t;
                sys::Con_DPrintf(c"MP3: skipped %ld bytes APE tag\n".as_ptr(), len);
            }

            // it's not impossible that _old_ MusicMatch tag places itself
            // after ID3v1
            let mut c_mm = probe_mmtag(stream, &mut buf);
            if c_mm < 0 {
                break 'fail;
            }
            // ID3v1 tag is at the end
            let c_id3 = probe_id3v1(stream, &mut buf, c_mm == 0);
            if c_id3 < 0 {
                break 'fail;
            }
            let _ = c_id3;
            // we do not know the order of ape or lyrics3 or musicmatch tags
            let mut c_ape = 0;
            let mut c_lyr = 0;
            loop {
                if c_lyr == 0 {
                    c_lyr = probe_lyrics3(stream, &mut buf);
                    if c_lyr < 0 {
                        break 'fail;
                    }
                    if c_lyr != 0 {
                        continue;
                    }
                }
                if c_mm == 0 {
                    c_mm = probe_mmtag(stream, &mut buf);
                    if c_mm < 0 {
                        break 'fail;
                    }
                    if c_mm != 0 {
                        continue;
                    }
                }
                if c_ape == 0 {
                    c_ape = probe_apetag(stream, &mut buf);
                    if c_ape < 0 {
                        break 'fail;
                    }
                    if c_ape != 0 {
                        continue;
                    }
                }
                break;
            }

            rc = if (*stream).fh.length > 0 { 0 } else { -1 };
        }

        if rc < 0 {
            (*stream).fh.start = oldstart;
            (*stream).fh.length = oldlength;
        }
        sys::FS_rewind(&mut (*stream).fh);
        rc
    }
}
