//! WAD2/WAD3 fuzzer: drives the quake_fs::wad decision logic over an
//! arbitrary WAD image the way the quake-capi loaders do.
//!
//! Input layout (mirroring wadinfo_t): bytes 0..4 = identification,
//! 4..8 = numlumps (LE i32), 8..12 = infotableofs (LE i32); the whole
//! input is the "file" whose length feeds the bounds checks, and lumpinfo_t
//! records (32 bytes each) are read from infotableofs like the C does after
//! its header check passes.

#![no_main]

use libfuzzer_sys::fuzz_target;
use quake_fs::wad;

const LUMPINFO_SIZE: usize = core::mem::size_of::<quake_types::wad::LumpInfo>();

fuzz_target!(|data: &[u8]| {
    if data.len() < 12 {
        return;
    }
    let id: [u8; 4] = data[0..4].try_into().unwrap();
    let numlumps = i32::from_le_bytes(data[4..8].try_into().unwrap());
    let infotableofs = i32::from_le_bytes(data[8..12].try_into().unwrap());
    let file_len = data.len() as i64;

    let _ = wad::wad2_id_ok(&id);
    let _ = wad::check_add_wad_header(i32::from_le_bytes(id), numlumps, infotableofs);

    if wad::header_extends_beyond(infotableofs, numlumps, file_len) {
        return;
    }
    // C parity subtlety: a negative numlumps sign-extends huge in the size_t
    // sum, which USUALLY trips the check — but numlumps == -1 (and other
    // small negatives) can wrap the sum past 2^64 and pass it with an
    // out-of-file infotableofs. The C never reads the table in that case
    // because its `for (i = 0; i < numlumps; i++)` is a signed loop that runs
    // zero times; mirror that here instead of casting numlumps to usize.
    // With numlumps > 0 the sum cannot wrap (<= 2^31 + 2^36), so a passing
    // check genuinely guarantees infotableofs + numlumps * 32 <= file_len
    // and the slice below is in bounds.
    if numlumps <= 0 {
        return;
    }
    let table = &data[infotableofs as usize..];
    for lump in table.chunks_exact(LUMPINFO_SIZE).take(numlumps as usize) {
        let mut filepos = i32::from_le_bytes(lump[0..4].try_into().unwrap());
        let disksize = i32::from_le_bytes(lump[4..8].try_into().unwrap());
        let mut size = i32::from_le_bytes(lump[8..12].try_into().unwrap());
        let verdict = wad::repair_lump(&mut filepos, &mut size, disksize, file_len);
        // Post-repair invariants of the C: size never stays negative, and a
        // lump reported healthy has non-negative filepos. Nothing stronger:
        // the ExtendsBeyond path can keep a negative filepos, and the
        // wrapped-i32 sum check can bless overflowing filepos+size — both
        // deliberate C parity, so neither is asserted away.
        assert!(size >= 0);
        if verdict.is_none() {
            assert!(filepos >= 0);
        }

        // W_CleanupName over the 16-byte name field (stops at NUL)
        let name = &lump[16..32];
        let nul = name.iter().position(|&b| b == 0).unwrap_or(16);
        let _ = wad::cleanup_name(&name[..nul]);
    }
});
