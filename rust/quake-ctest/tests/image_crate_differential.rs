//! Differential tests for the Image_DecodeSTB seam (Phase 3 M8, AC11): the
//! Rust sniff-and-dispatch shim vs the original streaming stb decoder
//! (image_stb.c compiled as c_ref_Image_DecodeSTB). Both sides open the same
//! fixture files through their own filesystem and the full observable state
//! is compared: returned buffer bytes (w*h*4 RGBA), out-dimensions
//! (including the failure paths that write them), console log, com_filesize
//! and the open-handle delta.
//!
//! The Rust shim bulk-reads the resource and decodes from memory (routing
//! crate-undecoded formats through the shared C Image_DecodeSTBMem), while
//! the oracle streams through the Sys_File callbacks — so every case here
//! also gates the memory-vs-callback equivalence of stb itself on that
//! input.

use quake_ctest::drivers as drv;
use quake_ctest::fs as ctfs; // also links the cc-built c_ref_* archive
use quake_ctest::fs::Side;
use std::sync::Once;

static SETUP: Once = Once::new();

/// Shared fixture dir mounted as a searchpath (root = tmp, gamedir
/// "stbgame") on both sides. Caller must hold [`ctfs::lock`].
fn file_dir() -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("quake-ctest-stb-{}", std::process::id()));
    let dir = root.join("stbgame");
    SETUP.call_once(|| {
        std::fs::create_dir_all(dir.join("gfx")).unwrap();
        for side in ctfs::BOTH {
            ctfs::setup(side, &[&root], 0, c"stbgame");
        }
    });
    dir
}

fn stb_buf_len(w: i32, h: i32) -> usize {
    ((w as u32).wrapping_mul(h as u32) as usize).wrapping_mul(4)
}

/// Runs both sides over `name` and asserts complete outcome equality
/// (data bytes, dims, con log, file size, handle balance).
fn compare_both(name: &std::ffi::CStr) -> drv::ImageOutcome {
    // SAFETY: fixture mounted by the caller via file_dir(); the STB seam
    // soft-fails on both sides
    let c = unsafe { drv::image_decode_side(Side::C, name, drv::ImageFormat::Stb, stb_buf_len) };
    // SAFETY: as above
    let r = unsafe { drv::image_decode_side(Side::Rust, name, drv::ImageFormat::Stb, stb_buf_len) };
    assert_eq!(c, r, "C vs Rust Image_DecodeSTB of {name:?}");
    assert_eq!(c.open_handles, 0, "decoder must close the handle");
    assert_eq!(c.error, None, "the STB seam never Sys_Errors");
    c
}

/// Repo root (this crate lives at rust/quake-ctest).
fn repo_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap()
        .to_path_buf()
}

// ---------------------------------------------------------------------------
// Real in-repo PNGs: gfx/conback.png and gfx/p_mods.png ship in the embedded
// vkquake.pak and are decoded on every engine launch; mainmenu2.png is the
// third in-tree PNG. These are the standing real-asset legs of AC11.

#[test]
fn vq_pak_pngs_decode_identically() {
    let _guard = ctfs::lock();
    let dir = file_dir();
    let src = repo_root().join("Misc/vq_pak/gfx");
    for name in ["conback.png", "mainmenu2.png", "p_mods.png"] {
        std::fs::copy(src.join(name), dir.join("gfx").join(name)).unwrap();
    }
    let out = compare_both(c"gfx/conback.png");
    assert!(out.data.is_some(), "conback.png must decode");
    assert!(out.width > 0 && out.height > 0);
    let out = compare_both(c"gfx/mainmenu2.png");
    assert!(out.data.is_some(), "mainmenu2.png must decode");
    let out = compare_both(c"gfx/p_mods.png");
    assert!(out.data.is_some(), "p_mods.png must decode");
}

// ---------------------------------------------------------------------------
// Sniff parity: the shim's ported probe chain must classify exactly like
// stbi__load_main. A wrong classification changes which decoder runs and
// therefore the observable failure reason (or worse, the pixels).

fn write_and_compare(rel: &str, cname: &std::ffi::CStr, bytes: &[u8]) -> drv::ImageOutcome {
    let dir = file_dir();
    std::fs::write(dir.join(rel), bytes).unwrap();
    compare_both(cname)
}

#[test]
fn sniff_unknown_types_warn_identically() {
    let _guard = ctfs::lock();
    // garbage: no probe matches -> "couldn't load ... (unknown image type)"
    let out = write_and_compare(
        "gfx/junk.png",
        c"gfx/junk.png",
        b"this is not an image at all",
    );
    assert_eq!(out.data, None);
    assert_eq!(
        out.con_log.len(),
        1,
        "exactly one warning: {:?}",
        out.con_log
    );
    assert!(
        out.con_log[0].contains("unknown image type"),
        "stb short reason expected: {:?}",
        out.con_log
    );
    // empty file
    let out = write_and_compare("gfx/empty.png", c"gfx/empty.png", b"");
    assert_eq!(out.data, None);
    // 7 of the 8 PNG signature bytes
    let out = write_and_compare(
        "gfx/sig7.png",
        c"gfx/sig7.png",
        &[137, 80, 78, 71, 13, 10, 26],
    );
    assert_eq!(out.data, None);
    // a lone 0xFF (EOF inside the JPEG fill run) and a non-SOI marker
    let out = write_and_compare("gfx/ff.png", c"gfx/ff.png", &[0xFF]);
    assert_eq!(out.data, None);
    let out = write_and_compare("gfx/eoi.png", c"gfx/eoi.png", &[0xFF, 0xD9]);
    assert_eq!(out.data, None);
}

#[test]
fn sniff_accepted_but_corrupt_bodies_warn_identically() {
    let _guard = ctfs::lock();
    // PNG signature + garbage chunk: png probe accepts, decode fails
    let mut png = vec![137, 80, 78, 71, 13, 10, 26, 10];
    png.extend_from_slice(b"garbage after the signature");
    let out = write_and_compare("gfx/badpng.png", c"gfx/badpng.png", &png);
    assert_eq!(out.data, None);
    assert_eq!(out.con_log.len(), 1);
    // bare SOI (with fill bytes): jpeg probe accepts, decode fails
    let out = write_and_compare("gfx/soi.jpg", c"gfx/soi.jpg", &[0xFF, 0xFF, 0xFF, 0xD8]);
    assert_eq!(out.data, None);
    // tga-plausible header with a truncated body: tga probe accepts; stb's
    // raw path zero-fills the tail on both sides
    let mut tga = vec![0u8; 18];
    tga[2] = 2; // uncompressed true-color
    tga[12] = 2; // width 2
    tga[14] = 2; // height 2
    tga[16] = 24; // bpp
    tga.extend_from_slice(&[10, 20, 30]); // 1 of 4 pixels
    let out = write_and_compare("gfx/trunc.tga", c"gfx/trunc.tga", &tga);
    assert_eq!((out.width, out.height), (2, 2));
    assert!(out.data.is_some(), "stb accepts truncated raw TGA");
}

#[test]
fn sniff_order_jpeg_fails_tga_passes() {
    let _guard = ctfs::lock();
    // first byte 0x00 keeps the jpeg probe out; the rest is a valid TGA
    // header (type 2), so classification order is observable via the result
    let mut tga = vec![0u8; 18];
    tga[2] = 2;
    tga[12] = 1;
    tga[14] = 1;
    tga[16] = 32;
    tga.extend_from_slice(&[1, 2, 3, 4]);
    let out = write_and_compare("gfx/ord.tga", c"gfx/ord.tga", &tga);
    assert_eq!((out.width, out.height), (1, 1));
    assert!(out.data.is_some());
}

// ---------------------------------------------------------------------------
// Well-formed decodes through the seam (currently the stb fallback for all
// three formats; each crate leg re-runs these plus its own matrix).

#[test]
fn small_tga_decodes_identically() {
    let _guard = ctfs::lock();
    // 2x2 24bpp bottom-origin (descriptor bit 5 clear -> stb flips)
    let mut tga = vec![0u8; 18];
    tga[2] = 2;
    tga[12] = 2;
    tga[14] = 2;
    tga[16] = 24;
    // BGR pixels, rows bottom-up
    tga.extend_from_slice(&[255, 0, 0, 0, 255, 0, 0, 0, 255, 10, 20, 30]);
    let out = write_and_compare("gfx/small.tga", c"gfx/small.tga", &tga);
    assert_eq!((out.width, out.height), (2, 2));
    let data = out.data.expect("valid TGA decodes");
    // stb converts BGR->RGB and flips vertically: file row 0 (blue, green)
    // becomes output row 1
    assert_eq!(&data[0..4], &[255, 0, 0, 255]); // file row 1 px 0 (B=0,G=0,R=255)
    assert_eq!(&data[12..16], &[0, 255, 0, 255]); // file row 0 px 1
}
