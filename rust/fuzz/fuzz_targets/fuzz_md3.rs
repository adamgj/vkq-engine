//! MD3 fuzzer (Phase 3 M7, D11 / AC6): the pure quake-formats MD3 header and
//! surface decisions the `Mod_LoadMD3Model` shim uses — the version gate,
//! the frame/surface-count fatals, and the per-surface ident/framecount
//! gates. The C-via-FFI graph differential (with the GLMesh upload payload
//! parity that is the real MD3 evidence) lives in `md3_differential`; there
//! are no `.md3` assets in the local depot, so the corpus gate carries none.

#![no_main]

use libfuzzer_sys::fuzz_target;
use quake_formats::md3;

fuzz_target!(|data: &[u8]| {
    if data.len() < md3::MD3_HEADER_SIZE {
        return;
    }
    // shim reads version off the smallest prefix, then the counts
    let version = md3::parse_version(&data[..md3::OFS_HDR_VERSION + 4]);
    let counts = md3::parse_header_counts(&data[..md3::MD3_HEADER_COUNTS_PREFIX]);

    // The shim's Sys_Error gates, in order: version, too-many-frames,
    // no-surfaces, too-many-surfaces. Classify each purely.
    let version_ok = version == md3::MD3_VERSION;
    let frames_ok = counts.num_frames <= md3::MAXALIASFRAMES;
    let surfaces_ok = counts.num_surfaces != 0 && counts.num_surfaces <= md3::MAX_SURFACES;
    let header_accepts = version_ok && frames_ok && surfaces_ok;

    // prefix-stability: the shim reads counts/offsets off short prefixes of
    // the header; parsing the same fields from the full header must agree
    let offsets = md3::parse_header_offsets(&data[..md3::MD3_HEADER_OFFSETS_PREFIX]);
    assert_eq!(
        offsets,
        md3::parse_header_offsets(&data[..md3::MD3_HEADER_SIZE])
    );
    assert_eq!(
        counts,
        md3::parse_header_counts(&data[..md3::MD3_HEADER_SIZE])
    );
    // the header decision itself has no second pure implementation to diff
    // against in-process; its reject parity is carried by md3_differential's
    // fatal-parity cases (version/frames/surfaces, in shim order)
    let _ = header_accepts;

    // Per-surface gates over the first surface block, if the input reaches
    // one: ident must equal IDMD3HEADER and numFrames must match the header.
    if data.len() >= md3::MD3_HEADER_SIZE + md3::MD3_SURFACE_SIZE {
        let surf = &data[md3::MD3_HEADER_SIZE..md3::MD3_HEADER_SIZE + md3::MD3_SURFACE_SIZE];
        let sident = md3::parse_surface_ident(&surf[..md3::MD3_SURFACE_IDENT_PREFIX]);
        let snf = md3::parse_surface_numframes(&surf[..md3::MD3_SURFACE_NUMFRAMES_PREFIX]);
        let sh = md3::parse_surface_header(&surf[..md3::MD3_SURFACE_SIZE]);
        assert_eq!(sh.ident, sident);
        assert_eq!(sh.num_frames, snf);
        // record parsers over fuzzed surface bytes must not panic, and the
        // triangle-index narrowing must match a direct re-read of the file
        // i32s (the u16 cast is the C's own narrowing, kept bug-for-bug)
        let tri = md3::parse_triangle_indexes(&surf[..12]);
        for (j, &idx) in tri.iter().enumerate() {
            let raw = i32::from_le_bytes(surf[j * 4..j * 4 + 4].try_into().unwrap());
            assert_eq!(idx, raw as u16);
        }
        let _ = md3::parse_st_native(&surf[..8]);
    }
});
