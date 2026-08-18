//! PAK directory parsing and the id1 modification gate (COM_LoadPackFile).
//!
//! The file IO, `Mem_*` allocation and searchpath threading stay in the FFI
//! shim; this module holds the decisions: header validation (in C's exact
//! check order), the directory CRC16 gate against the retail pak0.pak
//! versions, and entry extraction with q_strlcpy's truncation behavior.

use quake_util::crc;

/// C: `MAX_FILES_IN_PACK` (pakfile.h)
pub const MAX_FILES_IN_PACK: usize = 2048;
/// C: `PAK0_COUNT` / `PAK0_CRC_V1xx` (common_fs.c)
pub const PAK0_COUNT: i32 = 339;
pub const PAK0_CRC_V100: u16 = 13900;
pub const PAK0_CRC_V101: u16 = 62751;
pub const PAK0_CRC_V106: u16 = 32981;

/// On-disk directory entry size: `sizeof (dpackfile_t)` = 56 + 4 + 4.
pub const DPACKFILE_SIZE: usize = 64;
/// `dpackfile_t.name` field width.
pub const DPACKFILE_NAME_SIZE: usize = 56;
/// `packfile_t.name` width (MAX_QPATH): q_strlcpy truncates to this - 1.
pub const PACKFILE_NAME_SIZE: usize = 64;

/// Fatal outcomes of COM_LoadPackFile's header checks (Sys_Error in C).
#[derive(Debug, PartialEq, Eq)]
pub enum PakError {
    /// C: `Sys_Error ("%s is not a packfile", packfile)`
    NotAPackfile,
    /// C: `Sys_Error ("Invalid packfile %s (dirlen: %i, dirofs: %i)", ...)`
    InvalidDirectory { dirlen: i32, dirofs: i32 },
    /// C: `Sys_Error ("%s has %i files", packfile, numpackfiles)`
    TooManyFiles(i32),
}

/// Non-fatal: `Sys_Printf ("WARNING: %s has no files, ignored\n", ...)`,
/// the pak is skipped and its handle closed.
#[derive(Debug, PartialEq, Eq)]
pub struct PakEmpty;

/// Header check in COM_LoadPackFile's exact order: magic, negative
/// dirlen/dirofs, empty (ignored), file-count cap. Returns numpackfiles.
pub fn check_header(
    id: [u8; 4],
    dirofs: i32,
    dirlen: i32,
) -> Result<Result<i32, PakEmpty>, PakError> {
    if id != *b"PACK" {
        return Err(PakError::NotAPackfile);
    }
    // C computes numpackfiles before the sign check but only uses it after
    let numpackfiles = dirlen / DPACKFILE_SIZE as i32;
    if dirlen < 0 || dirofs < 0 {
        return Err(PakError::InvalidDirectory { dirlen, dirofs });
    }
    if numpackfiles == 0 {
        return Ok(Err(PakEmpty));
    }
    if numpackfiles > MAX_FILES_IN_PACK as i32 {
        return Err(PakError::TooManyFiles(numpackfiles));
    }
    Ok(Ok(numpackfiles))
}

/// The com_modified decision: anything but a retail id1 pak0.pak directory
/// (entry count and directory CRC16) marks the install modified.
pub fn pak_is_modified(numpackfiles: i32, dir_crc: u16) -> bool {
    numpackfiles != PAK0_COUNT || !matches!(dir_crc, PAK0_CRC_V106 | PAK0_CRC_V101 | PAK0_CRC_V100)
}

/// CRC16 over the raw directory bytes exactly as read from disk
/// (header.dirlen bytes).
pub fn directory_crc(dir_bytes: &[u8]) -> u16 {
    crc::crc_block(dir_bytes)
}

/// A parsed directory entry (packfile_t image).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackEntry {
    /// NUL-terminated within PACKFILE_NAME_SIZE, exactly the bytes
    /// q_strlcpy would have produced.
    pub name: [u8; PACKFILE_NAME_SIZE],
    pub filepos: i32,
    pub filelen: i32,
}

/// Parse `numpackfiles` entries from the directory bytes.
///
/// q_strlcpy copies from the entry's name field until a NUL, truncated to 63
/// chars: an unterminated 56-byte name deliberately keeps reading into the
/// entry's filepos/filelen bytes, like the C. The C would continue past the
/// directory into stale static-buffer memory; we stop at the slice end (the
/// only divergence, unreachable with a well-formed directory).
pub fn parse_entries(dir_bytes: &[u8], numpackfiles: i32) -> Vec<PackEntry> {
    let mut entries = Vec::with_capacity(numpackfiles as usize);
    for i in 0..numpackfiles as usize {
        let base = i * DPACKFILE_SIZE;
        let mut name = [0u8; PACKFILE_NAME_SIZE];
        let avail = &dir_bytes[base..];
        for (j, dst) in name.iter_mut().take(PACKFILE_NAME_SIZE - 1).enumerate() {
            match avail.get(j) {
                Some(&b) if b != 0 => *dst = b,
                _ => break,
            }
        }
        let filepos = i32::from_le_bytes(dir_bytes[base + 56..base + 60].try_into().unwrap());
        let filelen = i32::from_le_bytes(dir_bytes[base + 60..base + 64].try_into().unwrap());
        entries.push(PackEntry {
            name,
            filepos,
            filelen,
        });
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir_entry(name: &[u8], filepos: i32, filelen: i32) -> [u8; 64] {
        let mut e = [0u8; 64];
        e[..name.len()].copy_from_slice(name);
        e[56..60].copy_from_slice(&filepos.to_le_bytes());
        e[60..64].copy_from_slice(&filelen.to_le_bytes());
        e
    }

    #[test]
    fn header_checks_in_c_order() {
        assert_eq!(check_header(*b"PAK0", 0, 64), Err(PakError::NotAPackfile));
        // bad magic wins over negative sizes
        assert_eq!(check_header(*b"XXXX", -1, -1), Err(PakError::NotAPackfile));
        assert_eq!(
            check_header(*b"PACK", -12, 64),
            Err(PakError::InvalidDirectory {
                dirlen: 64,
                dirofs: -12
            })
        );
        assert_eq!(
            check_header(*b"PACK", 12, -64),
            Err(PakError::InvalidDirectory {
                dirlen: -64,
                dirofs: 12
            })
        );
        assert_eq!(check_header(*b"PACK", 12, 0), Ok(Err(PakEmpty)));
        // < one entry of dirlen truncates to zero files, like C int division
        assert_eq!(check_header(*b"PACK", 12, 63), Ok(Err(PakEmpty)));
        assert_eq!(check_header(*b"PACK", 12, 64), Ok(Ok(1)));
        assert_eq!(check_header(*b"PACK", 12, 2048 * 64), Ok(Ok(2048)));
        assert_eq!(
            check_header(*b"PACK", 12, 2049 * 64),
            Err(PakError::TooManyFiles(2049))
        );
    }

    #[test]
    fn modification_gate() {
        assert!(!pak_is_modified(PAK0_COUNT, PAK0_CRC_V106));
        assert!(!pak_is_modified(PAK0_COUNT, PAK0_CRC_V101));
        assert!(!pak_is_modified(PAK0_COUNT, PAK0_CRC_V100));
        assert!(pak_is_modified(PAK0_COUNT, 12345));
        assert!(pak_is_modified(338, PAK0_CRC_V106));
    }

    #[test]
    fn entry_parse_and_name_truncation() {
        let mut dir = Vec::new();
        dir.extend_from_slice(&dir_entry(b"maps/e1m1.bsp", 100, 200));
        // unterminated 56-byte name bleeds into filepos bytes, like q_strlcpy
        // reading past the field
        let unterminated = dir_entry(&[b'A'; 56], 0x42424242, 7);
        dir.extend_from_slice(&unterminated);
        let entries = parse_entries(&dir, 2);

        assert_eq!(&entries[0].name[..14], b"maps/e1m1.bsp\0");
        assert_eq!(entries[0].filepos, 100);
        assert_eq!(entries[0].filelen, 200);

        // 56 'A's, then 4 'B's (0x42) from filepos, then filelen's first
        // bytes (7, 0, 0) stop the copy at the NUL after truncating to 63
        assert_eq!(&entries[1].name[..56], &[b'A'; 56][..]);
        assert_eq!(&entries[1].name[56..60], b"BBBB");
        assert_eq!(entries[1].name[60], 7);
        assert_eq!(entries[1].name[61], 0);
        assert_eq!(entries[1].filepos, 0x42424242);
        assert_eq!(entries[1].filelen, 7);
    }

    #[test]
    fn retail_crc_gate_uses_crc16_block() {
        // pin the CRC implementation to the engine's CRC16 (CCITT, init 0xffff)
        assert_eq!(directory_crc(b""), crc::crc_block(b""));
        assert!(pak_is_modified(PAK0_COUNT, directory_crc(&[0u8; 64])));
    }
}
