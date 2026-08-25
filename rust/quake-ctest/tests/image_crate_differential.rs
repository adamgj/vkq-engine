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

// ---------------------------------------------------------------------------
// TGA matrix (M8 step 3): the hand-ported quake-image::tga vs streaming stb,
// over every layout the loader has branches for. Each case funnels through
// compare_both, so pixels, dims (success and failure write points), con log
// and handle balance are all asserted.

use quake_ctest::image_fixture::{build_tga, lcg_bytes, tga_rle_stream, TgaHeader};

fn tga_case(name_num: &mut u32, bytes: &[u8]) -> drv::ImageOutcome {
    let rel = format!("gfx/m{}.tga", *name_num);
    *name_num += 1;
    let dir = file_dir();
    std::fs::write(dir.join(&rel), bytes).unwrap();
    let cname = std::ffi::CString::new(rel).unwrap();
    compare_both(&cname)
}

#[test]
fn tga_matrix_raw_and_rle() {
    let _guard = ctfs::lock();
    let mut n = 100;
    let (w, h) = (5u16, 4u16);
    for image_type in [2u8, 3, 10, 11] {
        let bpps: &[u8] = if image_type % 8 == 3 {
            &[8, 16]
        } else {
            // bpp 8 with a "truecolor" type is a legal-quirk grey decode
            &[8, 15, 16, 24, 32]
        };
        for &bpp in bpps {
            for descriptor in [0u8, 0x20] {
                let px_size = match bpp {
                    8 => 1usize,
                    15 | 16 => 2,
                    24 => 3,
                    _ => 4,
                };
                let pixels = usize::from(w) * usize::from(h);
                let payload = if image_type >= 8 {
                    tga_rle_stream(px_size, pixels, u32::from(bpp) * 7 + u32::from(descriptor))
                } else {
                    lcg_bytes(
                        u32::from(bpp) * 13 + u32::from(descriptor),
                        pixels * px_size,
                    )
                };
                let hdr = TgaHeader {
                    image_type,
                    width: w,
                    height: h,
                    bpp,
                    descriptor,
                    ..Default::default()
                };
                let out = tga_case(&mut n, &build_tga(&hdr, &payload));
                assert_eq!(
                    (out.width, out.height),
                    (i32::from(w), i32::from(h)),
                    "type {image_type} bpp {bpp} desc {descriptor:#x}"
                );
                assert!(
                    out.data.is_some(),
                    "type {image_type} bpp {bpp} desc {descriptor:#x} must decode"
                );
            }
        }
    }
}

#[test]
fn tga_matrix_indexed() {
    let _guard = ctfs::lock();
    let mut n = 200;
    let (w, h) = (5u16, 4u16);
    let palette_len = 6u16;
    for image_type in [1u8, 9] {
        for palette_bits in [8u8, 15, 16, 24, 32] {
            for index_bpp in [8u8, 16] {
                let entry_size = match palette_bits {
                    8 => 1usize,
                    15 | 16 => 2,
                    24 => 3,
                    _ => 4,
                };
                let idx_size = usize::from(index_bpp) / 8;
                let mut payload = Vec::new();
                payload.extend_from_slice(&lcg_bytes(3, usize::from(palette_len) * entry_size));
                let pixels = usize::from(w) * usize::from(h);
                if image_type == 9 {
                    payload.extend_from_slice(&tga_rle_stream(
                        idx_size,
                        pixels,
                        u32::from(palette_bits),
                    ));
                } else {
                    // index stream with deliberate out-of-range values
                    // (>= palette_len -> entry 0)
                    for i in 0..pixels {
                        let idx = (i % 9) as u16; // 6..8 are out of range
                        if idx_size == 1 {
                            payload.push(idx as u8);
                        } else {
                            payload.extend_from_slice(&idx.to_le_bytes());
                        }
                    }
                }
                let hdr = TgaHeader {
                    colormap_type: 1,
                    image_type,
                    palette_len,
                    palette_bits,
                    width: w,
                    height: h,
                    bpp: index_bpp,
                    descriptor: 0,
                    ..Default::default()
                };
                let out = tga_case(&mut n, &build_tga(&hdr, &payload));
                assert!(
                    out.data.is_some(),
                    "indexed type {image_type} pal {palette_bits} idx {index_bpp} must decode"
                );
            }
        }
    }
}

#[test]
fn tga_offsets_and_palette_start_skip() {
    let _guard = ctfs::lock();
    let mut n = 300;
    // nonzero offset field: junk before the pixel data
    let mut payload = vec![0xAA; 5];
    payload.extend_from_slice(&lcg_bytes(11, 6));
    let hdr = TgaHeader {
        image_type: 2,
        width: 2,
        height: 1,
        bpp: 24,
        descriptor: 0x20,
        offset: 5,
        ..Default::default()
    };
    let out = tga_case(&mut n, &build_tga(&hdr, &payload));
    assert!(out.data.is_some());

    // nonzero palette_start: junk between the offset skip and the palette
    let mut payload = vec![0xBB; 3]; // offset junk
    payload.extend_from_slice(&[0xCC; 4]); // palette_start junk
    payload.extend_from_slice(&lcg_bytes(17, 2 * 3)); // 2 BGR entries
    payload.extend_from_slice(&[0, 1, 1, 0]); // indexes
    let hdr = TgaHeader {
        colormap_type: 1,
        image_type: 1,
        palette_start: 4,
        palette_len: 2,
        palette_bits: 24,
        width: 2,
        height: 2,
        bpp: 8,
        descriptor: 0,
        offset: 3,
        ..Default::default()
    };
    let out = tga_case(&mut n, &build_tga(&hdr, &payload));
    assert!(out.data.is_some());

    // descriptor alpha bits (low nibble) and x-origin are ignored by stb
    let hdr = TgaHeader {
        image_type: 2,
        width: 2,
        height: 2,
        bpp: 32,
        descriptor: 0x28,
        x_origin: 7,
        y_origin: 3,
        ..Default::default()
    };
    let out = tga_case(&mut n, &build_tga(&hdr, &lcg_bytes(23, 16)));
    assert!(out.data.is_some());
}

#[test]
fn tga_reject_parity() {
    let _guard = ctfs::lock();
    let mut n = 400;
    // empty palette: "bad palette", dims already published
    let hdr = TgaHeader {
        colormap_type: 1,
        image_type: 1,
        palette_len: 0,
        palette_bits: 24,
        width: 3,
        height: 2,
        bpp: 8,
        ..Default::default()
    };
    let out = tga_case(&mut n, &build_tga(&hdr, &[0, 1, 2]));
    assert_eq!(out.data, None);
    assert_eq!(
        (out.width, out.height),
        (3, 2),
        "dims published before the reject"
    );
    assert!(out.con_log[0].contains("bad palette"), "{:?}", out.con_log);

    // truncated non-rgb16 palette: "bad palette"
    let hdr = TgaHeader {
        colormap_type: 1,
        image_type: 1,
        palette_len: 4,
        palette_bits: 24,
        width: 2,
        height: 1,
        bpp: 8,
        ..Default::default()
    };
    let out = tga_case(&mut n, &build_tga(&hdr, &[9, 9, 9])); // 3 of 12 bytes
    assert_eq!(out.data, None);
    assert!(out.con_log[0].contains("bad palette"), "{:?}", out.con_log);

    // truncated rgb16 palette: accepted, missing entries read as zeros
    let hdr = TgaHeader {
        colormap_type: 1,
        image_type: 1,
        palette_len: 4,
        palette_bits: 16,
        width: 2,
        height: 1,
        bpp: 8,
        ..Default::default()
    };
    let mut payload = lcg_bytes(29, 3); // 1.5 of 4 entries
    payload.extend_from_slice(&[0, 3]); // indexes
    let out = tga_case(&mut n, &build_tga(&hdr, &payload));
    assert!(out.data.is_some(), "truncated rgb16 palette still decodes");

    // int-overflow size: "too large", dims published
    let hdr = TgaHeader {
        image_type: 2,
        width: 65535,
        height: 65535,
        bpp: 32,
        ..Default::default()
    };
    let out = tga_case(&mut n, &build_tga(&hdr, &[]));
    assert_eq!(out.data, None);
    assert_eq!((out.width, out.height), (65535, 65535));
    assert!(out.con_log[0].contains("too large"), "{:?}", out.con_log);

    // truncated RLE mid-run and mid-literal: accepted, zero tail
    for (name_seed, cut) in [(1u32, 3usize), (2, 5)] {
        let hdr = TgaHeader {
            image_type: 10,
            width: 4,
            height: 4,
            bpp: 24,
            descriptor: 0x20,
            ..Default::default()
        };
        let full = tga_rle_stream(3, 16, name_seed);
        let out = tga_case(&mut n, &build_tga(&hdr, &full[..cut]));
        assert!(out.data.is_some(), "truncated RLE still decodes (zeros)");
    }
}

// ---------------------------------------------------------------------------
// PNG matrix (M8 step 4): the structural-walk + `png`-crate pipeline vs
// streaming stb, across color types, depths, interlacing, tRNS, checksum
// tolerance and the structural reject reasons. Crate-internal rejects
// (deflate bodies, filter bytes, short pixel data) share the reject decision
// with stb but not the reason text: those cases go through
// compare_both_masked_reason per the owner-approved warning-text policy.

fn png_chunk(ctype: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&(data.len() as u32).to_be_bytes());
    v.extend_from_slice(ctype);
    v.extend_from_slice(data);
    v.extend_from_slice(&[0u8; 4]); // CRC: garbage — stb never checks it
    v
}

fn png_sig() -> Vec<u8> {
    vec![137, 80, 78, 71, 13, 10, 26, 10]
}

fn png_ihdr(w: u32, h: u32, depth: u8, color: u8, interlace: u8) -> Vec<u8> {
    let mut d = Vec::new();
    d.extend_from_slice(&w.to_be_bytes());
    d.extend_from_slice(&h.to_be_bytes());
    d.extend_from_slice(&[depth, color, 0, 0, interlace]);
    png_chunk(b"IHDR", &d)
}

fn png_channels(color: u8) -> usize {
    match color {
        0 | 3 => 1,
        2 => 3,
        4 => 2,
        _ => 4,
    }
}

/// Raw (pre-compression) scanline stream: per row one in-range filter byte
/// plus the packed row bytes, honoring adam7 sub-image layout when
/// interlaced. `max_sample` caps sample bytes (palette fixtures keep
/// indices in range — OOB indices are the UB-excluded class).
fn png_raw_stream(
    w: u32,
    h: u32,
    depth: u8,
    color: u8,
    interlace: u8,
    seed: u32,
    max_sample: u8,
) -> Vec<u8> {
    let bpp_bits = usize::from(depth) * png_channels(color);
    let mut out = Vec::new();
    let mut push_pass = |pw: u32, ph: u32, salt: u32| {
        if pw == 0 || ph == 0 {
            return;
        }
        let row_bytes = (pw as usize * bpp_bits).div_ceil(8);
        for r in 0..ph {
            let bytes = lcg_bytes(seed ^ salt ^ (r * 977), row_bytes + 1);
            out.push(bytes[0] % 5); // filter byte 0..4
            for b in &bytes[1..] {
                out.push(b % max_sample.max(1));
            }
        }
    };
    if interlace == 0 {
        push_pass(w, h, 0);
    } else {
        const PASSES: [(u32, u32, u32, u32); 7] = [
            (0, 0, 8, 8),
            (4, 0, 8, 8),
            (0, 4, 4, 8),
            (2, 0, 4, 4),
            (0, 2, 2, 4),
            (1, 0, 2, 2),
            (0, 1, 1, 2),
        ];
        for (i, (x0, y0, dx, dy)) in PASSES.into_iter().enumerate() {
            let pw = w.saturating_sub(x0).div_ceil(dx);
            let ph = h.saturating_sub(y0).div_ceil(dy);
            push_pass(pw, ph, 31 * (i as u32 + 1));
        }
    }
    out
}

fn deflate_zlib(raw: &[u8]) -> Vec<u8> {
    miniz_oxide::deflate::compress_to_vec_zlib(raw, 6)
}

/// Assemble sig + IHDR + optional PLTE/tRNS + IDAT(s) + IEND.
#[allow(clippy::too_many_arguments)] // a fixture builder mirroring the chunk layout
fn build_png(
    w: u32,
    h: u32,
    depth: u8,
    color: u8,
    interlace: u8,
    plte: Option<&[u8]>,
    trns: Option<&[u8]>,
    seed: u32,
    max_sample: u8,
) -> Vec<u8> {
    let mut f = png_sig();
    f.extend(png_ihdr(w, h, depth, color, interlace));
    if let Some(p) = plte {
        f.extend(png_chunk(b"PLTE", p));
    }
    if let Some(t) = trns {
        f.extend(png_chunk(b"tRNS", t));
    }
    let raw = png_raw_stream(w, h, depth, color, interlace, seed, max_sample);
    f.extend(png_chunk(b"IDAT", &deflate_zlib(&raw)));
    f.extend(png_chunk(b"IEND", &[]));
    f
}

/// compare_both, but the parenthesized warning reason is stripped before the
/// console logs are compared (crate-originated reject text differs from
/// stb's; decision, dims, handle balance and the warning prefix still must
/// match exactly). Owner-approved policy, task-plan amendment log.
fn compare_both_masked_reason(name: &std::ffi::CStr) -> drv::ImageOutcome {
    let mask = |mut o: drv::ImageOutcome| {
        for l in &mut o.con_log {
            if let Some(p) = l.find("couldn't load ") {
                if let Some(paren) = l[p..].find(" (") {
                    l.truncate(p + paren + 2);
                }
            }
        }
        o
    };
    // SAFETY: fixture mounted; the STB seam soft-fails on both sides
    let c =
        mask(unsafe { drv::image_decode_side(Side::C, name, drv::ImageFormat::Stb, stb_buf_len) });
    // SAFETY: as above
    let r = mask(unsafe {
        drv::image_decode_side(Side::Rust, name, drv::ImageFormat::Stb, stb_buf_len)
    });
    assert_eq!(
        c, r,
        "C vs Rust Image_DecodeSTB of {name:?} (reasons masked)"
    );
    assert_eq!(c.open_handles, 0, "decoder must close the handle");
    c
}

fn png_case(n: &mut u32, bytes: &[u8]) -> drv::ImageOutcome {
    let rel = format!("gfx/p{}.png", *n);
    *n += 1;
    let dir = file_dir();
    std::fs::write(dir.join(&rel), bytes).unwrap();
    let cname = std::ffi::CString::new(rel).unwrap();
    compare_both(&cname)
}

#[test]
fn png_matrix_color_depth_interlace() {
    let _guard = ctfs::lock();
    let mut n = 500;
    let combos: &[(u8, &[u8])] = &[
        (0, &[1, 2, 4, 8, 16]),
        (2, &[8, 16]),
        (3, &[1, 2, 4, 8]),
        (4, &[8, 16]),
        (6, &[8, 16]),
    ];
    for &(color, depths) in combos {
        for &depth in depths {
            for interlace in [0u8, 1] {
                let (plte_buf, max_sample);
                let plte: Option<&[u8]> = if color == 3 {
                    // full palette for the depth so no index is OOB
                    let entries = 1usize << depth.min(8);
                    plte_buf = lcg_bytes(7, entries * 3);
                    max_sample = 255; // all indices in range by construction
                    Some(&plte_buf)
                } else {
                    max_sample = 255;
                    None
                };
                let f = build_png(
                    5,
                    4,
                    depth,
                    color,
                    interlace,
                    plte,
                    None,
                    u32::from(color) * 131 + u32::from(depth) * 7 + u32::from(interlace),
                    max_sample,
                );
                let out = png_case(&mut n, &f);
                assert_eq!(
                    (out.width, out.height),
                    (5, 4),
                    "color {color} depth {depth} i{interlace}"
                );
                assert!(
                    out.data.is_some(),
                    "color {color} depth {depth} i{interlace} must decode"
                );
            }
        }
    }
}

#[test]
fn png_trns_variants() {
    let _guard = ctfs::lock();
    let mut n = 600;
    // gray 8: key = 3; a raw row places sample 3 so the key matches
    let f = build_png(4, 2, 8, 0, 0, None, Some(&[0, 3]), 41, 4);
    let out = png_case(&mut n, &f);
    assert!(out.data.is_some());
    assert!(
        out.data
            .as_ref()
            .unwrap()
            .chunks_exact(4)
            .any(|p| p[3] == 0),
        "the colorkey must actually clear some alpha"
    );
    // gray 1/2/4-bit with keys inside and outside the legal range
    for (depth, key) in [(1u8, 1u16), (2, 3), (4, 9), (2, 55)] {
        let f = build_png(6, 3, depth, 0, 0, None, Some(&key.to_be_bytes()), 43, 255);
        let out = png_case(&mut n, &f);
        assert!(out.data.is_some(), "gray{depth} key {key}");
    }
    // gray 16 with a full 16-bit key
    let f = build_png(
        3,
        3,
        16,
        0,
        0,
        None,
        Some(&0x1234u16.to_be_bytes()),
        47,
        255,
    );
    assert!(png_case(&mut n, &f).data.is_some());
    // rgb 8 and rgb 16
    let f = build_png(3, 2, 8, 2, 0, None, Some(&[0, 1, 0, 2, 0, 3]), 51, 4);
    assert!(png_case(&mut n, &f).data.is_some());
    let f = build_png(
        3,
        2,
        16,
        2,
        0,
        None,
        Some(&[0x11, 0x22, 0x33, 0x44, 0x55, 0x66]),
        53,
        255,
    );
    assert!(png_case(&mut n, &f).data.is_some());
    // paletted with partial alpha table (2 of 4 entries)
    let plte = lcg_bytes(3, 4 * 3);
    let f = build_png(4, 2, 2, 3, 0, Some(&plte), Some(&[7, 200]), 57, 255);
    let out = png_case(&mut n, &f);
    assert!(out.data.is_some());
    assert!(
        out.data
            .as_ref()
            .unwrap()
            .chunks_exact(4)
            .any(|p| p[3] != 255),
        "palette alpha must land in the output"
    );
}

#[test]
fn png_checksum_and_trailing_tolerance() {
    let _guard = ctfs::lock();
    let mut n = 700;
    // all fixtures already carry garbage CRCs (stb never checks); also
    // corrupt the zlib adler and append trailing garbage after IEND
    let mut f = build_png(4, 3, 8, 2, 0, None, None, 61, 255);
    let len = f.len();
    f[len - 20] ^= 0xFF; // somewhere in the adler tail of IDAT
    f.extend_from_slice(b"trailing garbage after IEND");
    let out = png_case(&mut n, &f);
    assert!(out.data.is_some(), "bad adler + trailing bytes must decode");

    // split the IDAT stream across three chunks, one of them empty
    let raw = png_raw_stream(4, 3, 8, 2, 0, 67, 255);
    let z = deflate_zlib(&raw);
    let mut f = png_sig();
    f.extend(png_ihdr(4, 3, 8, 2, 0));
    f.extend(png_chunk(b"IDAT", &z[..3]));
    f.extend(png_chunk(b"IDAT", &[]));
    f.extend(png_chunk(b"IDAT", &z[3..]));
    f.extend(png_chunk(b"IEND", &[]));
    let out = png_case(&mut n, &f);
    assert!(out.data.is_some(), "split IDAT must decode");

    // unknown ancillary chunk (lowercase first letter): skipped by both
    let mut f = png_sig();
    f.extend(png_ihdr(4, 3, 8, 0, 0));
    f.extend(png_chunk(b"gAMA", &1000u32.to_be_bytes()));
    f.extend(png_chunk(b"zzZZ", &[1, 2, 3]));
    f.extend(png_chunk(
        b"IDAT",
        &deflate_zlib(&png_raw_stream(4, 3, 8, 0, 0, 71, 255)),
    ));
    f.extend(png_chunk(b"IEND", &[]));
    let out = png_case(&mut n, &f);
    assert!(out.data.is_some(), "ancillary chunks must be skipped");

    // stb ignores the declared zlib window (cinfo=15); the wrapper
    // normalizes it, so both sides accept
    let raw = png_raw_stream(2, 2, 8, 0, 0, 73, 255);
    let mut z = deflate_zlib(&raw);
    z[0] = 0xF8;
    z[1] = 0x00; // 0xF800 % 31 == 0, FDICT clear
    let mut f = png_sig();
    f.extend(png_ihdr(2, 2, 8, 0, 0));
    f.extend(png_chunk(b"IDAT", &z));
    f.extend(png_chunk(b"IEND", &[]));
    let out = png_case(&mut n, &f);
    assert!(out.data.is_some(), "cinfo=15 zlib header accepted like stb");
}

#[test]
fn png_structural_reject_parity() {
    let _guard = ctfs::lock();
    let mut n = 800;
    let expect_reject = |n: &mut u32, f: &[u8], reason: &str| {
        let out = png_case(n, f);
        assert_eq!(out.data, None, "case {reason:?}");
        assert_eq!(out.con_log.len(), 1, "case {reason:?}: {:?}", out.con_log);
        assert!(
            out.con_log[0].contains(reason),
            "expected {reason:?} in {:?}",
            out.con_log
        );
    };
    // IHDR not first
    let mut f = png_sig();
    f.extend(png_chunk(b"sBIT", &[8]));
    f.extend(png_ihdr(2, 2, 8, 0, 0));
    expect_reject(&mut n, &f, "first not IHDR");
    // 0-pixel
    expect_reject(
        &mut n,
        &[png_sig(), png_ihdr(0, 2, 8, 0, 0)].concat(),
        "0-pixel image",
    );
    // dims over 1<<24
    expect_reject(
        &mut n,
        &[png_sig(), png_ihdr(1 << 25, 2, 8, 0, 0)].concat(),
        "too large",
    );
    // the (1<<30) pixel-buffer guard
    expect_reject(
        &mut n,
        &[png_sig(), png_ihdr(1 << 24, 65, 8, 0, 0)].concat(),
        "too large",
    );
    // bad depth / bad color type / paletted 16-bit
    expect_reject(
        &mut n,
        &[png_sig(), png_ihdr(2, 2, 3, 0, 0)].concat(),
        "1/2/4/8/16-bit only",
    );
    expect_reject(
        &mut n,
        &[png_sig(), png_ihdr(2, 2, 8, 7, 0)].concat(),
        "bad ctype",
    );
    expect_reject(
        &mut n,
        &[png_sig(), png_ihdr(2, 2, 16, 3, 0)].concat(),
        "bad ctype",
    );
    // PLTE length not a multiple of 3
    let mut f = png_sig();
    f.extend(png_ihdr(2, 2, 8, 3, 0));
    f.extend(png_chunk(b"PLTE", &[1, 2, 3, 4]));
    expect_reject(&mut n, &f, "invalid PLTE");
    // paletted IDAT without PLTE
    let mut f = png_sig();
    f.extend(png_ihdr(2, 2, 8, 3, 0));
    f.extend(png_chunk(b"IDAT", &[1, 2, 3]));
    expect_reject(&mut n, &f, "no PLTE");
    // tRNS before PLTE / oversized tRNS / tRNS with alpha
    let mut f = png_sig();
    f.extend(png_ihdr(2, 2, 8, 3, 0));
    f.extend(png_chunk(b"tRNS", &[1]));
    expect_reject(&mut n, &f, "tRNS before PLTE");
    let mut f = png_sig();
    f.extend(png_ihdr(2, 2, 8, 3, 0));
    f.extend(png_chunk(b"PLTE", &[1, 2, 3]));
    f.extend(png_chunk(b"tRNS", &[1, 2]));
    expect_reject(&mut n, &f, "bad tRNS len");
    let mut f = png_sig();
    f.extend(png_ihdr(2, 2, 8, 6, 0));
    f.extend(png_chunk(b"tRNS", &[0; 8]));
    expect_reject(&mut n, &f, "tRNS with alpha");
    // unknown critical chunk: 4-char name lands in the reason
    let mut f = png_sig();
    f.extend(png_ihdr(2, 2, 8, 0, 0));
    f.extend(png_chunk(b"AbCd", &[]));
    expect_reject(&mut n, &f, "AbCd PNG chunk not known");
    // missing IEND: the zero chunk header's NUL name truncates the reason
    // to an empty string -> "couldn't load <name> ()"
    let mut f = png_sig();
    f.extend(png_ihdr(2, 2, 8, 0, 0));
    let out = png_case(&mut n, &f);
    assert_eq!(out.data, None);
    assert!(
        out.con_log[0].trim_end().ends_with("()"),
        "{:?}",
        out.con_log
    );
    // IEND with no IDAT
    let mut f = png_sig();
    f.extend(png_ihdr(2, 2, 8, 0, 0));
    f.extend(png_chunk(b"IEND", &[]));
    expect_reject(&mut n, &f, "no IDAT");
    // truncated IDAT payload
    let mut f = png_sig();
    f.extend(png_ihdr(2, 2, 8, 0, 0));
    f.extend_from_slice(&64u32.to_be_bytes());
    f.extend_from_slice(b"IDAT");
    f.extend_from_slice(&[1, 2, 3]);
    expect_reject(&mut n, &f, "outofdata");
    // IDAT length field over 2^30 (no data needs to exist)
    let mut f = png_sig();
    f.extend(png_ihdr(2, 2, 8, 0, 0));
    f.extend_from_slice(&((1u32 << 30) + 1).to_be_bytes());
    f.extend_from_slice(b"IDAT");
    expect_reject(&mut n, &f, "IDAT size limit");
    // zlib header rejects (stb short reasons, decided by the walk)
    let zbad = |z: &[u8]| {
        let mut f = png_sig();
        f.extend(png_ihdr(2, 2, 8, 0, 0));
        f.extend(png_chunk(b"IDAT", z));
        f.extend(png_chunk(b"IEND", &[]));
        f
    };
    expect_reject(&mut n, &zbad(&[0x78, 0x02, 0, 0]), "bad zlib header");
    // an exactly-two-byte stream is "bad zlib header" whatever its bytes:
    // stb's zeof check runs after both header reads
    expect_reject(&mut n, &zbad(&[0x78, 0x01]), "bad zlib header");
    expect_reject(&mut n, &zbad(&[0x78, 0x20, 0]), "no preset dict"); // fcheck ok, FDICT set
    expect_reject(&mut n, &zbad(&[0x77, 0x09, 0]), "bad compression"); // cm=7, fcheck ok
}

#[test]
fn png_crate_rejects_share_the_decision() {
    let _guard = ctfs::lock();
    let mut n = 900;
    let masked = |n: &mut u32, bytes: &[u8]| {
        let rel = format!("gfx/p{}.png", *n);
        *n += 1;
        let dir = file_dir();
        std::fs::write(dir.join(&rel), bytes).unwrap();
        let cname = std::ffi::CString::new(rel).unwrap();
        compare_both_masked_reason(&cname)
    };
    // deflate body truncated mid-stream: stb "outofdata"-ish zlib reason,
    // crate its own — decision must agree (reject)
    let raw = png_raw_stream(4, 4, 8, 2, 0, 81, 255);
    let z = deflate_zlib(&raw);
    let mut f = png_sig();
    f.extend(png_ihdr(4, 4, 8, 2, 0));
    f.extend(png_chunk(b"IDAT", &z[..z.len() / 2]));
    f.extend(png_chunk(b"IEND", &[]));
    let out = masked(&mut n, &f);
    assert_eq!(
        out.data, None,
        "truncated deflate body rejects on both sides"
    );
    // filter byte out of range (5): both reject
    let mut raw = png_raw_stream(3, 2, 8, 0, 0, 83, 255);
    raw[0] = 5;
    let mut f = png_sig();
    f.extend(png_ihdr(3, 2, 8, 0, 0));
    f.extend(png_chunk(b"IDAT", &deflate_zlib(&raw)));
    f.extend(png_chunk(b"IEND", &[]));
    let out = masked(&mut n, &f);
    assert_eq!(out.data, None, "invalid filter rejects on both sides");
    // stream deflates fine but holds too few rows: both reject
    let raw = png_raw_stream(4, 2, 8, 0, 0, 87, 255); // 2 of 4 rows
    let mut f = png_sig();
    f.extend(png_ihdr(4, 4, 8, 0, 0));
    f.extend(png_chunk(b"IDAT", &deflate_zlib(&raw)));
    f.extend(png_chunk(b"IEND", &[]));
    let out = masked(&mut n, &f);
    assert_eq!(out.data, None, "short pixel data rejects on both sides");
}
