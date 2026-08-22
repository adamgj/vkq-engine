//! Mod_LoadLighting: .lit validation and the lightmap expansions.

/// Outcome of validating an external .lit file body (after the path-priority
/// check, which is shim-side).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LitCheck {
    /// C: `Con_Printf ("Corrupt .lit file (old version?), ignoring\n")`
    NotQlit,
    /// C: `Con_Printf ("Unknown .lit file version (%d)\n", i)`
    BadVersion(i32),
    /// C: `Con_Printf ("Outdated .lit file (%s should be %u bytes, not
    /// %lld)\n", ...)` — `expected` is the C int `8 + l->filelen * 3`
    /// (printed with %u)
    WrongSize { expected: i32 },
    /// Use `data[8 .. 8 + filelen * 3]`
    Ok,
}

/// The QLIT header check of Mod_LoadLighting. `data` is the loaded .lit
/// file, `filelen` the BSP lighting lump length, `com_filesize` the .lit
/// file size as reported by COM_LoadFile.
pub fn check_lit(data: &[u8], filelen: i32, com_filesize: i64) -> LitCheck {
    if data.len() < 8 || &data[0..4] != b"QLIT" {
        return LitCheck::NotQlit;
    }
    let version = i32::from_le_bytes([data[4], data[5], data[6], data[7]]);
    if version != 1 {
        return LitCheck::BadVersion(version);
    }
    let expected = 8i32.wrapping_add(filelen.wrapping_mul(3));
    if i64::from(expected) == com_filesize {
        LitCheck::Ok
    } else {
        LitCheck::WrongSize { expected }
    }
}

/// The white-lighting expansion: each sample byte becomes an RGB triple.
/// (The C in-place copy-to-tail trick produces the same bytes.)
pub fn expand_white(samples: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(samples.len() * 3);
    for &d in samples {
        out.extend_from_slice(&[d, d, d]);
    }
    out
}

/// The Quake64 16-bit RGB unpack (RRRRRGGG GGBBBBBB). C iterates
/// `filelen / 2` pairs, so a trailing odd byte is dropped.
pub fn expand_q64(lump: &[u8]) -> Vec<u8> {
    let pairs = lump.len() / 2;
    let mut out = Vec::with_capacity(pairs * 3);
    for p in lump[..pairs * 2].chunks_exact(2) {
        let (b0, b1) = (p[0], p[1]);
        out.push(b0 & 0xf8);
        out.push(((b0 & 0x07) << 5) + ((b1 & 0xc0) >> 5));
        out.push((b1 & 0x3f) << 2);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lit(version: i32, samples: usize) -> Vec<u8> {
        let mut d = b"QLIT".to_vec();
        d.extend_from_slice(&version.to_le_bytes());
        d.extend(std::iter::repeat_n(0xabu8, samples));
        d
    }

    #[test]
    fn lit_header_paths() {
        assert_eq!(check_lit(b"BLIT\x01\x00\x00\x00", 0, 8), LitCheck::NotQlit);
        assert_eq!(check_lit(b"QLI", 0, 3), LitCheck::NotQlit);
        assert_eq!(check_lit(&lit(2, 0), 0, 8), LitCheck::BadVersion(2));
        let d = lit(1, 12);
        assert_eq!(check_lit(&d, 4, 20), LitCheck::Ok);
        assert_eq!(check_lit(&d, 4, 21), LitCheck::WrongSize { expected: 20 });
    }

    #[test]
    fn white_expansion() {
        assert_eq!(
            expand_white(&[0, 128, 255]),
            vec![0, 0, 0, 128, 128, 128, 255, 255, 255]
        );
    }

    #[test]
    fn q64_unpack() {
        // 0xff 0xff -> 0xf8, (0x07<<5)+(0xc0>>5) = 0xe0+0x06 = 0xe6, 0x3f<<2 = 0xfc
        assert_eq!(expand_q64(&[0xff, 0xff]), vec![0xf8, 0xe6, 0xfc]);
        assert_eq!(expand_q64(&[0x00, 0x00]), vec![0, 0, 0]);
        // odd trailing byte dropped
        assert_eq!(expand_q64(&[0xff, 0xff, 0x12]).len(), 3);
    }
}
