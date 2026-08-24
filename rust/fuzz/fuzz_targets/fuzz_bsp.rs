//! Brush/BSP fuzzer (Phase 3 M7, D11 / AC4): the pure quake-formats::bsp
//! lump parsers and PVS/extents kernels the brush shims are built on. Each
//! parser's accept/reject is the funny-lump-size gate the engine's
//! `Sys_Error`/`Host_Error` fires on, so this fuzzes those decisions plus
//! the two value-driven fatals (CalcSurfaceExtents `> 2000`, PVS overrun).
//! The full C-via-FFI graph differential over the five dialects lives in
//! `quake-ctest`'s `bsp_differential` and the real-map `formats_corpus`
//! gate; this is coverage-guided exploration that must never panic.

#![no_main]

use libfuzzer_sys::fuzz_target;
use quake_formats::bsp::{extents, lumps, vis, Bsp2};

fn dialect(sel: u8) -> (Bsp2, bool, bool, bool) {
    // (leaf/node dialect, bsp2 record width, valve, q64)
    match sel % 5 {
        0 => (Bsp2::No, false, false, false), // BSP29
        1 => (Bsp2::No, false, true, false),  // Valve
        2 => (Bsp2::L1, true, false, false),  // 2PSB
        3 => (Bsp2::L2, true, false, false),  // BSP2
        _ => (Bsp2::No, false, false, true),  // Quake64
    }
}

fuzz_target!(|data: &[u8]| {
    if data.len() < 8 {
        return;
    }
    let (dia, bsp2, _valve, _q64) = dialect(data[0]);
    // numleafs reaches vis_row/parse_nodes from mod->numleafs, which the
    // shim sets from parse_leafs — capped at 32767 (LeafError::TooMany), so
    // it is always a small non-negative count there, never near i32::MAX
    // (where (numleafs + 31) would overflow in the pure row calc as it wraps
    // in the C int). The u16 mask keeps this in [0, 65535] — a superset of
    // the shim's real domain, still comfortably below the wrap hazard.
    let numleafs = i32::from(u16::from_le_bytes([data[1], data[2]]));
    // the rest of the input is one shared lump body, fed to every parser so
    // the funny-size gate is exercised at each record width
    let lump = &data[8..];

    // Reject <=> len % record_size != 0, for each sized lump. A parser that
    // returns Ok must have produced exactly len/record records.
    macro_rules! sized {
        ($parse:expr, $rec:expr) => {
            match $parse {
                Ok(v) => assert_eq!(v.len(), lump.len() / $rec),
                Err(_) => assert!(!lump.len().is_multiple_of($rec)),
            }
        };
    }
    let vrec = if bsp2 {
        lumps::DLEDGE_SIZE
    } else {
        lumps::DSEDGE_SIZE
    };
    let frec = if bsp2 {
        lumps::DLFACE_SIZE
    } else {
        lumps::DSFACE_SIZE
    };
    let crec = if bsp2 {
        lumps::DLCLIPNODE_SIZE
    } else {
        lumps::DSCLIPNODE_SIZE
    };

    sized!(lumps::parse_vertexes(lump), lumps::DVERTEX_SIZE);
    sized!(lumps::parse_edges(lump, bsp2), vrec);
    sized!(lumps::parse_surfedges(lump), 4);
    sized!(lumps::parse_planes(lump), lumps::DPLANE_SIZE);
    sized!(lumps::parse_texinfo(lump), lumps::TEXINFO_SIZE);
    sized!(lumps::parse_faces(lump, bsp2), frec);
    sized!(lumps::parse_clipnodes(lump, bsp2), crec);
    let _ = lumps::parse_nodes(lump, dia, numleafs);
    let _ = lumps::parse_leafs(lump, dia);

    // marksurfaces: FunnySize <=> len % rec, BadSurface <=> an index is out
    // of [0, numsurfaces); the shim replays whichever the C fired
    let numsurfaces = i32::from_le_bytes([data[4], data[5], data[6], data[7]]);
    let mark = lumps::parse_marksurfaces(lump, bsp2, numsurfaces);
    let mrec = if bsp2 { 4 } else { 2 };
    match mark.error {
        Some(lumps::MarkError::FunnySize) => assert!(!lump.len().is_multiple_of(mrec)),
        _ => assert!(lump.len().is_multiple_of(mrec)),
    }

    // CalcSurfaceExtents over fuzzed points/vecs: accept, or reject with a
    // single extent past the 2000 limit (unless the special flag suppresses
    // it). Read a few [f32;3] points and the two [f32;4] vecs out of `lump`.
    let f32s: Vec<f32> = lump
        .chunks_exact(4)
        .take(14)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    if f32s.len() == 14 {
        let vecs = [
            [f32s[0], f32s[1], f32s[2], f32s[3]],
            [f32s[4], f32s[5], f32s[6], f32s[7]],
        ];
        let points = [[f32s[8], f32s[9], f32s[10]], [f32s[11], f32s[12], f32s[13]]];
        for special in [false, true] {
            let _ = extents::calc_surface_extents(points.iter().copied(), &vecs, special);
        }
    }

    // PVS decompression: never write more than `row`, and a `None` input
    // fills exactly `row` all-visible bytes.
    let row = vis::vis_row(numleafs).max(0) as usize;
    if row <= (1 << 16) {
        let (out, status) = vis::decompress_vis(Some(lump), row);
        assert!(out.len() <= row);
        let (fill, fstatus) = vis::decompress_vis(None, row);
        assert_eq!(fill.len(), row);
        assert_eq!(fstatus, vis::VisStatus::Complete);
        let _ = status;
    }
});
