//! Pure decision logic of `Quake/wad.c` (WAD2 loading). The stateful/I-O
//! orchestration (engine globals, COM_*/FS_* calls, in-place buffer edits)
//! lives in quake-capi; these functions reproduce the exact quirks:
//! lenient repair-not-reject bounds handling (including the C's
//! `q_max (0, size - filepos)` after `filepos = 0`), wrapped-int overflow
//! comparisons, and W_CleanupName's zero-padding without a terminating NUL
//! for 16-character names.

/// `W_CleanupName`: lowercase (ASCII), stop at NUL, zero-pad to exactly 16.
/// `input` is the name bytes up to the first NUL (at most 16).
pub fn cleanup_name(input: &[u8]) -> [u8; 16] {
    let mut out = [0u8; 16];
    for (i, o) in out.iter_mut().enumerate() {
        let Some(&c) = input.get(i) else { break };
        if c == 0 {
            break;
        }
        *o = if c.is_ascii_uppercase() {
            c + (b'a' - b'A')
        } else {
            c
        };
    }
    out
}

/// W_LoadWadFile's magic check: literal 'W','A','D','2' only (W_AddWadFile
/// separately accepts WAD3).
pub fn wad2_id_ok(identification: &[u8; 4]) -> bool {
    identification == b"WAD2"
}

/// W_LoadWadFile's header bounds check:
/// `infotableofs < 0 || infotableofs + numlumps * sizeof(lumpinfo_t) >
/// (size_t)com_filesize` — the sum is size_t arithmetic, so negative operands
/// sign-extend to huge values and trip the check.
pub fn header_extends_beyond(infotableofs: i32, numlumps: i32, file_len: i64) -> bool {
    if infotableofs < 0 {
        return true;
    }
    // sizeof (lumpinfo_t), taken from the mirror whose layout quake-types
    // asserts against the C struct, so the two cannot drift
    const LUMPINFO_SIZE: u64 = core::mem::size_of::<quake_types::wad::LumpInfo>() as u64;
    let total = (infotableofs as i64 as u64)
        .wrapping_add((numlumps as i64 as u64).wrapping_mul(LUMPINFO_SIZE));
    total > file_len as u64
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LumpProblem {
    /// `"... begins %lld bytes beyond end"` — filepos was zeroed
    BeginsBeyond { over: i64 },
    /// `"... extends %lld bytes beyond end (lump size: %u)"`
    ExtendsBeyond { over: i64, size: i32 },
}

/// The shared lump repair of both wad loaders, over already-byteswapped
/// values. Mutates `filepos`/`size` exactly like the C and reports which
/// warning (if any) the caller should print. Int additions wrap like the
/// C's (technically-UB, practically-wrapping) 32-bit arithmetic.
pub fn repair_lump(
    filepos: &mut i32,
    size: &mut i32,
    disksize: i32,
    file_len: i64,
) -> Option<LumpProblem> {
    if (filepos.wrapping_add(*size) as i64) > file_len
        && (filepos.wrapping_add(disksize) as i64) <= file_len
    {
        *size = disksize;
    }
    if *filepos < 0 || *size < 0 || (filepos.wrapping_add(*size) as i64) > file_len {
        if (*filepos as i64) > file_len || *size < 0 {
            let over = *filepos as i64 - file_len;
            *filepos = 0;
            // C: `size = q_max (0, size - filepos)` after zeroing filepos
            *size = (*size).max(0);
            Some(LumpProblem::BeginsBeyond { over })
        } else {
            let over = (filepos.wrapping_add(*size)) as i64 - file_len;
            let reported_size = *size;
            *size = (size.wrapping_sub(*filepos)).max(0);
            Some(LumpProblem::ExtendsBeyond {
                over,
                size: reported_size,
            })
        }
    } else {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AddWadVerdict {
    /// accepted, with the (byteswapped) id preserved in wad->id
    Ok,
    /// `"%s is not a valid WAD"`
    BadId,
    /// `"%s is not a valid WAD (%i lumps, %i info table offset)"`
    BadCounts,
    /// `"WAD file %s has no lumps, ignored"`
    Empty,
}

/// W_AddWadFile's header verdict; `id` is the little-endian id word (WAD2 or
/// WAD3 accepted here, unlike W_LoadWadFile).
pub fn check_add_wad_header(id: i32, numlumps: i32, infotableofs: i32) -> AddWadVerdict {
    const WADID: i32 = i32::from_le_bytes(*b"WAD2");
    const WADID_VALVE: i32 = i32::from_le_bytes(*b"WAD3");
    if id != WADID && id != WADID_VALVE {
        return AddWadVerdict::BadId;
    }
    if numlumps < 0 || infotableofs < 0 {
        return AddWadVerdict::BadCounts;
    }
    if numlumps == 0 {
        return AddWadVerdict::Empty;
    }
    AddWadVerdict::Ok
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleanup_name_semantics() {
        assert_eq!(&cleanup_name(b"SKY1"), b"sky1\0\0\0\0\0\0\0\0\0\0\0\0");
        // 16-char names get no terminating NUL
        assert_eq!(&cleanup_name(b"ABCDEFGHIJKLMNOP"), b"abcdefghijklmnop");
        assert_eq!(&cleanup_name(b"{FOO_bar-9"), b"{foo_bar-9\0\0\0\0\0\0");
        // non-ASCII bytes pass through
        assert_eq!(cleanup_name(&[0xc4, 0]).first(), Some(&0xc4));
    }

    #[test]
    fn repair_cases() {
        // healthy lump untouched
        let (mut fp, mut sz) = (16, 100);
        assert_eq!(repair_lump(&mut fp, &mut sz, 100, 1000), None);
        assert_eq!((fp, sz), (16, 100));

        // size overrun falls back to disksize when that fits
        let (mut fp, mut sz) = (16, 100000);
        assert_eq!(repair_lump(&mut fp, &mut sz, 100, 1000), None);
        assert_eq!((fp, sz), (16, 100));

        // begins beyond: filepos zeroed, size kept non-negative
        let (mut fp, mut sz) = (2000, 50);
        assert!(matches!(
            repair_lump(&mut fp, &mut sz, 50, 1000),
            Some(LumpProblem::BeginsBeyond { over: 1000 })
        ));
        assert_eq!((fp, sz), (0, 50));

        // extends beyond: size clamped by the C's odd size-minus-filepos
        let (mut fp, mut sz) = (900, 500);
        assert!(matches!(
            repair_lump(&mut fp, &mut sz, 500, 1000),
            Some(LumpProblem::ExtendsBeyond {
                over: 400,
                size: 500
            })
        ));
        assert_eq!((fp, sz), (900, 0));

        // negative size
        let (mut fp, mut sz) = (10, -5);
        assert!(matches!(
            repair_lump(&mut fp, &mut sz, -5, 1000),
            Some(LumpProblem::BeginsBeyond { .. })
        ));
        assert_eq!((fp, sz), (0, 0));
    }

    #[test]
    fn header_checks() {
        assert!(wad2_id_ok(b"WAD2"));
        assert!(!wad2_id_ok(b"WAD3"));
        assert!(header_extends_beyond(-1, 0, 1000));
        assert!(header_extends_beyond(12, 100, 1000));
        assert!(!header_extends_beyond(12, 30, 1000));
        assert!(header_extends_beyond(12, -1, 100000)); // negative lumps -> huge size_t

        assert_eq!(
            check_add_wad_header(i32::from_le_bytes(*b"WAD3"), 5, 12),
            AddWadVerdict::Ok
        );
        assert_eq!(
            check_add_wad_header(i32::from_le_bytes(*b"PACK"), 5, 12),
            AddWadVerdict::BadId
        );
        assert_eq!(
            check_add_wad_header(i32::from_le_bytes(*b"WAD2"), -1, 12),
            AddWadVerdict::BadCounts
        );
        assert_eq!(
            check_add_wad_header(i32::from_le_bytes(*b"WAD2"), 0, 12),
            AddWadVerdict::Empty
        );
    }
}
