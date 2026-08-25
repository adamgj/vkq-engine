//! MDL (alias model) fuzzer (Phase 3 M7, D11 / AC5): the pure quake-formats
//! MDL header decision the `Mod_ParseAliasModel` shim uses — the version
//! gate and the `validate` diagnostic list whose first fatal is the shim's
//! `Sys_Error`. The full C-via-FFI graph differential over synthetic and
//! real .mdl files lives in `alias_differential` and `formats_corpus`.

#![no_main]

use libfuzzer_sys::fuzz_target;
use quake_formats::mdl;

fuzz_target!(|data: &[u8]| {
    if data.len() < mdl::MDL_T_SIZE {
        return;
    }
    // the shim reads `version` off the smallest prefix first, then the full
    // header; mirror that so the fuzzer explores the version gate on inputs
    // that stop there
    let version = mdl::parse_version(&data[..mdl::OFS_VERSION + 4]);
    let h = mdl::parse_header(&data[..mdl::MDL_T_SIZE]);
    assert_eq!(h.version, version);

    let diags = mdl::validate(&h);

    // At most one fatal, and it is always the last element: `validate`
    // returns early on the first fatal, so nothing follows it.
    let fatal_positions: Vec<usize> = diags
        .iter()
        .enumerate()
        .filter(|(_, d)| d.is_fatal())
        .map(|(i, _)| i)
        .collect();
    assert!(fatal_positions.len() <= 1);
    if let Some(&p) = fatal_positions.first() {
        assert_eq!(p, diags.len() - 1);
    }

    // The record parsers over any 4/12-byte-aligned prefix must not panic.
    if data.len() >= mdl::MDL_T_SIZE + mdl::STVERT_T_SIZE {
        let off = mdl::MDL_T_SIZE;
        let _ = mdl::parse_stvert(&data[off..off + mdl::STVERT_T_SIZE]);
    }
    if data.len() >= mdl::MDL_T_SIZE + mdl::DTRIANGLE_T_SIZE {
        let off = mdl::MDL_T_SIZE;
        let _ = mdl::parse_triangle(&data[off..off + mdl::DTRIANGLE_T_SIZE]);
    }
});
