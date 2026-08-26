//! Pure predicates and length computations from the MP3 tag skipper
//! (`Quake/snd_mp3tag.c`, Phase 4). These are the parts that inspect
//! attacker-controlled bytes with no IO, split out of the `quake-capi` shim
//! so they can be fuzzed directly (ADR-019 section 7) -- the same
//! "fuzz the pure predicates, not the c_ref oracle" design the Phase 3
//! format targets use, since the C original's `Sys_Error` cannot be trapped
//! across a Rust frame.
//!
//! The IO-driven probes (`probe_*`, `get_lyrics3v1_len`,
//! `get_musicmatch_len`) stay in the shim: they seek and read through
//! `fshandle_t`.
//!
//! `long` in the C is `c_long` here, so the arithmetic keeps the C's
//! platform width (32-bit on Windows, 64-bit elsewhere) exactly.

use core::ffi::{c_int, c_long};

pub fn is_id3v1(data: &[u8]) -> bool {
    // http://id3.org/ID3v1 :  3 bytes "TAG" identifier and 125 bytes tag data
    !(data.len() < 128 || &data[0..3] != b"TAG")
}

pub fn is_id3v2(data: &[u8]) -> bool {
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

pub fn get_id3v2_len(data: &[u8], length: c_long) -> c_long {
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

pub fn is_apetag(data: &[u8]) -> bool {
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

pub fn get_ape_len(data: &[u8]) -> c_long {
    let mut size = i32::from_le_bytes([data[12], data[13], data[14], data[15]]) as c_long;
    let version = u32::from_le_bytes([data[8], data[9], data[10], data[11]]);
    let flags = u32::from_le_bytes([data[20], data[21], data[22], data[23]]);
    if version == 2000 && (flags & (1 << 31)) != 0 {
        size += 32; // header present
    }
    size
}

pub fn is_lyrics3tag(data: &[u8]) -> c_int {
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

pub fn get_lyrics3v2_len(data: &[u8], length: c_long) -> c_long {
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

pub fn verify_lyrics3v2(data: &[u8]) -> bool {
    data.len() >= 11 && &data[0..11] == b"LYRICSBEGIN"
}

fn q_isdigit(c: u8) -> bool {
    c.is_ascii_digit()
}

pub fn is_musicmatch(data: &[u8]) -> bool {
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
