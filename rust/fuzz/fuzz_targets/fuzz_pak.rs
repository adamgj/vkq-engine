//! PAK directory fuzzer: drives quake_fs::pak the way the FFI shim does.
//!
//! Input layout (mirroring dpackheader_t): bytes 0..4 = id, 4..8 = dirofs
//! (LE i32), 8..12 = dirlen (LE i32); everything after byte 12 plays the
//! directory bytes the shim would have read from disk at dirofs.
//!
//! Goal: no panics/aborts/OOM anywhere in the library.

#![no_main]

use libfuzzer_sys::fuzz_target;
use quake_fs::pak;

fuzz_target!(|data: &[u8]| {
    if data.len() < 12 {
        return;
    }
    let id: [u8; 4] = data[0..4].try_into().unwrap();
    let dirofs = i32::from_le_bytes(data[4..8].try_into().unwrap());
    let dirlen = i32::from_le_bytes(data[8..12].try_into().unwrap());

    let Ok(Ok(numpackfiles)) = pak::check_header(id, dirofs, dirlen) else {
        return;
    };

    let dir_bytes = &data[12..];

    // HARNESS CLAMP, not library logic: the shim only calls parse_entries
    // after a successful full read of dirlen bytes, so dir_bytes is always
    // >= numpackfiles * 64 there. A fuzz input is free to promise more
    // entries than it carries; mirror a caller with a short read by
    // truncating numpackfiles to what the buffer actually holds instead of
    // letting the harness index out of bounds.
    let numpackfiles = numpackfiles.min((dir_bytes.len() / pak::DPACKFILE_SIZE) as i32);

    // The CRC gate runs over the raw directory image exactly as read.
    let crc_len = (dirlen as usize).min(dir_bytes.len());
    let crc = pak::directory_crc(&dir_bytes[..crc_len]);
    let _ = pak::pak_is_modified(numpackfiles, crc);

    let entries = pak::parse_entries(dir_bytes, numpackfiles);
    assert_eq!(entries.len(), numpackfiles as usize);
    for e in &entries {
        // packfile_t names are always NUL-terminated within 64 bytes.
        assert_eq!(e.name[pak::PACKFILE_NAME_SIZE - 1], 0);
    }
});
