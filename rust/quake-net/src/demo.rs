//! Demo file format (cl_demo.c wire half, Phase 5 M4): the forcetrack
//! header line and the `[i32le length][3 x f32le viewangles][payload]`
//! record framing. The raw stdio (fread/fwrite/fflush/fseek on
//! `cls.demofile`, which may sit inside a pak) stays C-side; this module
//! owns the byte format, so recorded files are byte-identical to the C
//! original's and playback accepts exactly what C accepted.

/// `[i32 length][3 x f32 viewangles]`, all little-endian
pub const RECORD_HEADER_SIZE: usize = 16;

/// quakedef.h MAX_MSGLEN: the playback bound `Sys_Error`s above this
pub const MAX_MSGLEN: i32 = quake_types::net::MAX_MSGLEN as i32;

/// Offset of `Sys_fseek (cls.demofile, -17, SEEK_END)` when resuming a
/// recording: 4 (length) + 12 (viewangles) + 1 (the single svc_disconnect
/// byte CL_Stop_f appends) -- the trailing disconnect record is overwritten.
pub const RESUME_RECORD_SEEK_END_OFFSET: i64 = -17;

/// CL_WriteDemoMessage's prefix for one record
pub fn record_header(cursize: i32, viewangles: [f32; 3]) -> [u8; RECORD_HEADER_SIZE] {
    let mut h = [0u8; RECORD_HEADER_SIZE];
    h[0..4].copy_from_slice(&cursize.to_le_bytes());
    for (i, a) in viewangles.iter().enumerate() {
        h[4 + i * 4..8 + i * 4].copy_from_slice(&a.to_le_bytes());
    }
    h
}

/// CL_GetDemoMessage's header decode. `None` is the "Demo message \>
/// MAX_MSGLEN" `Sys_Error` path; a negative length passes here like in C
/// (the following C-side fread then fails and stops playback).
pub fn parse_record_header(h: &[u8; RECORD_HEADER_SIZE]) -> Option<(i32, [f32; 3])> {
    let cursize = i32::from_le_bytes(h[0..4].try_into().unwrap());
    if cursize > MAX_MSGLEN {
        return None;
    }
    let mut angles = [0f32; 3];
    for (i, a) in angles.iter_mut().enumerate() {
        *a = f32::from_le_bytes(h[4 + i * 4..8 + i * 4].try_into().unwrap());
    }
    Some((cursize, angles))
}

/// The `fprintf (cls.demofile, "%i\n", cls.forcetrack)` header line
pub fn forcetrack_line(track: i32) -> Vec<u8> {
    format!("{track}\n").into_bytes()
}

/// CL_PlayDemo_f's header parse: `fscanf ("%i")` then an explicit
/// `fgetc () == '\n'` (a plain "%i\n" would also eat a following space --
/// see the O.S. comment in cl_demo.c). Returns (track, bytes consumed
/// including the newline).
///
/// fscanf's `%i` is strtol base 0: leading C whitespace skipped, optional
/// sign, `0x` hex / leading-`0` octal / decimal. COMPAT: out-of-range
/// values are undefined in C; glibc/macOS strtol saturates to
/// LONG_MAX/LONG_MIN before the int truncation, mirrored here by
/// accumulating in the sign's own direction with i64 saturation. No
/// engine-written demo hits these corners (the writer emits plain decimal);
/// the reachable domain is differentially tested against libc
/// (net_demo_differential.rs).
pub fn parse_forcetrack(buf: &[u8]) -> Option<(i32, usize)> {
    let mut i = 0;
    while i < buf.len() && matches!(buf[i], b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r') {
        i += 1;
    }
    let neg = match buf.get(i) {
        Some(b'-') => {
            i += 1;
            true
        }
        Some(b'+') => {
            i += 1;
            false
        }
        _ => false,
    };
    let mut base: i64 = 10;
    let mut digits = 0usize;
    if buf.get(i) == Some(&b'0') {
        if matches!(buf.get(i + 1), Some(b'x') | Some(b'X'))
            && buf.get(i + 2).is_some_and(u8::is_ascii_hexdigit)
        {
            base = 16;
            i += 2;
        } else {
            base = 8;
            // the leading 0 itself is a digit
        }
    }
    let mut val: i64 = 0;
    while let Some(&c) = buf.get(i) {
        let d = match (c, base) {
            (b'0'..=b'9', _) if i64::from(c - b'0') < base => i64::from(c - b'0'),
            (b'a'..=b'f', 16) => i64::from(c - b'a') + 10,
            (b'A'..=b'F', 16) => i64::from(c - b'A') + 10,
            _ => break,
        };
        // accumulate in the sign's direction so overflow saturates to
        // LONG_MIN/LONG_MAX like strtol, not to -LONG_MAX
        val = if neg {
            val.saturating_mul(base).saturating_sub(d)
        } else {
            val.saturating_mul(base).saturating_add(d)
        };
        digits += 1;
        i += 1;
    }
    if digits == 0 {
        return None;
    }
    let track = val as i32;
    // the explicit fgetc check: the very next byte must be the newline
    if buf.get(i) != Some(&b'\n') {
        return None;
    }
    Some((track, i + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_header_roundtrip() {
        let h = record_header(1234, [1.5, -90.0, 359.75]);
        assert_eq!(h.len(), 16);
        let (len, ang) = parse_record_header(&h).unwrap();
        assert_eq!(len, 1234);
        assert_eq!(ang, [1.5, -90.0, 359.75]);
    }

    #[test]
    fn oversize_rejected_negative_passes() {
        let h = record_header(MAX_MSGLEN + 1, [0.0; 3]);
        assert!(parse_record_header(&h).is_none());
        let h = record_header(-5, [0.0; 3]);
        assert_eq!(parse_record_header(&h).unwrap().0, -5);
    }

    #[test]
    fn forcetrack_roundtrip_and_quirks() {
        for t in [-1, 0, 2, 11, i32::MAX] {
            let line = forcetrack_line(t);
            let (v, n) = parse_forcetrack(&line).unwrap();
            assert_eq!((v, n), (t, line.len()));
        }
        // fscanf skips leading whitespace, %i takes base-0 forms
        assert_eq!(parse_forcetrack(b"  \t-3\nxx"), Some((-3, 6)));
        assert_eq!(parse_forcetrack(b"0x10\n"), Some((16, 5)));
        assert_eq!(parse_forcetrack(b"010\n"), Some((8, 4)));
        assert_eq!(parse_forcetrack(b"0\n"), Some((0, 2)));
        // the explicit fgetc-newline check: trailing junk before '\n' fails
        assert_eq!(parse_forcetrack(b"5 \n"), None);
        assert_eq!(parse_forcetrack(b"5x\n"), None);
        assert_eq!(parse_forcetrack(b"\n"), None);
        assert_eq!(parse_forcetrack(b""), None);
        assert_eq!(parse_forcetrack(b"5"), None);
    }
}
