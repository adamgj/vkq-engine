//! Differential tests: the Rust PCX/LMP decoders (quake-capi image shims)
//! vs the original image_decode.c compiled as c_ref_*. Both sides open the
//! same fixture files through their own filesystem (Phase 3 M2, AC3) and
//! the full observable state is compared: returned buffer bytes (including
//! the PCX +1 padding pixel), out-dimensions, console log, open-handle
//! count, and Sys_Error messages for reject inputs.
//!
//! Reject parity covers the C-defined error paths only: inputs whose C
//! behavior is out-of-bounds reads/writes (RLE overrun / mid-run EOF,
//! sub-768-byte resources inside paks) are UB in the oracle and diverge by
//! design — see the COMPAT notes in quake-image/src/pcx.rs and the task
//! plan amendment log.

use core::ffi::{c_char, c_int, c_uint};
use quake_ctest::fs as ctfs; // also links the cc-built c_ref_* archive
use quake_ctest::fs::Side;
use std::sync::Once;

extern "C" {
    fn c_ref_Image_DecodePCX(
        file_handle: c_int,
        width: *mut c_int,
        height: *mut c_int,
        image_name: *const c_char,
    ) -> *mut u8;
    fn c_ref_Image_DecodeLMP(
        file_handle: c_int,
        width: *mut c_int,
        height: *mut c_int,
        image_name: *const c_char,
    ) -> *mut u8;
}

type DecodeFn = unsafe extern "C" fn(c_int, *mut c_int, *mut c_int, *const c_char) -> *mut u8;

fn decoder(side: Side, pcx: bool) -> DecodeFn {
    match (side, pcx) {
        (Side::C, true) => c_ref_Image_DecodePCX,
        (Side::C, false) => c_ref_Image_DecodeLMP,
        (Side::Rust, true) => quake_rs::image_decode::Image_DecodePCX,
        (Side::Rust, false) => quake_rs::image_decode::Image_DecodeLMP,
    }
}

static SETUP: Once = Once::new();

/// Shared fixture dir mounted as a searchpath (root = tmp, gamedir
/// "imggame") on both sides, like wad_differential's. Caller must hold
/// [`ctfs::lock`].
fn file_dir() -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("quake-ctest-img-{}", std::process::id()));
    let dir = root.join("imggame");
    SETUP.call_once(|| {
        std::fs::create_dir_all(dir.join("gfx")).unwrap();
        for side in ctfs::BOTH {
            ctfs::setup(side, &[&root], 0, c"imggame");
        }
    });
    dir
}

/// Everything one side observes from a successful (or NULL-returning)
/// decode call.
#[derive(Debug, PartialEq, Eq)]
struct Outcome {
    width: c_int,
    height: c_int,
    /// None for a NULL return; the buffer length is supplied by the caller
    /// (the C size expressions, identical on both sides by construction)
    data: Option<Vec<u8>>,
    file_size: i64,
    con_log: Vec<String>,
    open_handles: i32,
}

/// Opens `name` on `side`, runs the side's decoder, and snapshots the
/// observable state. `buf_len` maps (width, height) to the byte count of the
/// returned allocation.
fn decode_on(
    side: Side,
    name: &std::ffi::CStr,
    pcx: bool,
    buf_len: impl Fn(c_int, c_int) -> usize,
) -> Outcome {
    ctfs::clear_logs();
    let mut handle: c_int = -1;
    let mut path_id: c_uint = 0;
    // SAFETY: side's searchpaths are mounted; out-params are valid
    let size = unsafe { (ctfs::fns(side).open_file)(name.as_ptr(), &mut handle, &mut path_id) };
    assert!(size >= 0, "fixture {name:?} must open on {side:?}");
    let file_size = ctfs::thread_file_size();

    let mut width: c_int = -1;
    let mut height: c_int = -1;
    // SAFETY: open handle positioned at the resource start, valid
    // out-pointers, NUL-terminated name (the C ABI contract)
    let data = unsafe { decoder(side, pcx)(handle, &mut width, &mut height, name.as_ptr()) };
    let data = if data.is_null() {
        None
    } else {
        let len = buf_len(width, height);
        // SAFETY: the decoder returned a Mem_Alloc'd buffer of exactly the
        // size its C original allocates, which buf_len reproduces
        let bytes = unsafe { core::slice::from_raw_parts(data, len) }.to_vec();
        // SAFETY: buffer came from the stub Mem_Alloc just above
        unsafe { quake_c_sys::Mem_Free(data.cast()) };
        Some(bytes)
    };
    Outcome {
        width,
        height,
        data,
        file_size,
        con_log: ctfs::con_log(),
        open_handles: ctfs::open_handle_count(),
    }
}

fn compare_both(
    name: &std::ffi::CStr,
    pcx: bool,
    buf_len: impl Fn(c_int, c_int) -> usize,
) -> Outcome {
    let c = decode_on(Side::C, name, pcx, &buf_len);
    let rust = decode_on(Side::Rust, name, pcx, &buf_len);
    assert_eq!(c, rust, "C vs Rust decode of {name:?}");
    assert_eq!(c.open_handles, 0, "decoder must close the handle");
    c
}

fn pcx_buf_len(w: c_int, h: c_int) -> usize {
    ((w * h + 1) * 4) as usize
}

fn lmp_buf_len(w: c_int, h: c_int) -> usize {
    (w as u32).wrapping_mul(h as u32) as usize
}

/// Same builder as quake-image's unit tests: minimal valid PCX with a
/// deterministic non-trivial palette.
fn build_pcx(w: u16, h: u16, bytes_per_line: u16, rle: &[u8]) -> Vec<u8> {
    let mut f = vec![0u8; 128];
    f[0] = 0x0A;
    f[1] = 5;
    f[2] = 1;
    f[3] = 8;
    f[8..10].copy_from_slice(&(w - 1).to_le_bytes());
    f[10..12].copy_from_slice(&(h - 1).to_le_bytes());
    f[65] = 1;
    f[66..68].copy_from_slice(&bytes_per_line.to_le_bytes());
    f.extend_from_slice(rle);
    let mut palette = [0u8; 768];
    for (i, b) in palette.iter_mut().enumerate() {
        *b = (i % 251) as u8;
    }
    f.extend_from_slice(&palette);
    f
}

fn build_lmp(w: u32, h: u32, pixels: &[u8]) -> Vec<u8> {
    let mut f = Vec::new();
    f.extend_from_slice(&w.to_le_bytes());
    f.extend_from_slice(&h.to_le_bytes());
    f.extend_from_slice(pixels);
    f
}

#[test]
fn pcx_decode_matches() {
    let _guard = ctfs::lock();
    let dir = file_dir();
    // literals then runs, exercising both RLE arms and a run crossing a row
    // boundary (the C decoder lets runs spill into the next row's region)
    std::fs::write(
        dir.join("gfx/basic.pcx"),
        build_pcx(4, 3, 4, &[1, 2, 3, 4, 0xC4, 7, 0xC4, 9]),
    )
    .unwrap();
    let out = compare_both(c"gfx/basic.pcx", true, pcx_buf_len);
    assert_eq!((out.width, out.height), (4, 3));
    let data = out.data.expect("valid PCX decodes");
    assert_eq!(&data[0..4], &[3, 4, 5, 255]); // palette[1*3..] + alpha
}

#[test]
fn pcx_padding_byte_matches() {
    let _guard = ctfs::lock();
    let dir = file_dir();
    // bytes_per_line > width: the pad byte on the last row lands in the +1
    // pixel slot, which the byte compare covers
    std::fs::write(dir.join("gfx/pad.pcx"), build_pcx(1, 2, 2, &[5, 6, 7, 8])).unwrap();
    let out = compare_both(c"gfx/pad.pcx", true, pcx_buf_len);
    assert_eq!((out.width, out.height), (1, 2));
}

#[test]
fn lmp_decode_matches() {
    let _guard = ctfs::lock();
    let dir = file_dir();
    let pixels: Vec<u8> = (0..12u8).collect();
    std::fs::write(dir.join("gfx/basic.lmp"), build_lmp(4, 3, &pixels)).unwrap();
    let out = compare_both(c"gfx/basic.lmp", false, lmp_buf_len);
    assert_eq!((out.width, out.height), (4, 3));
    assert_eq!(out.data.as_deref(), Some(&pixels[..]));
}

#[test]
fn lmp_size_mismatch_returns_null() {
    let _guard = ctfs::lock();
    let dir = file_dir();
    // one byte short of 8 + w*h: both sides take the NULL early-return and
    // still close the handle
    std::fs::write(dir.join("gfx/short.lmp"), build_lmp(4, 3, &[0; 11])).unwrap();
    let out = compare_both(c"gfx/short.lmp", false, lmp_buf_len);
    assert_eq!(out.data, None);
}

// ---------------------------------------------------------------------------
// Sys_Error parity: the C side runs under the longjmp trap (C frames only);
// the Rust side re-runs rust_fatal_child in a child process (PLAN.md §4
// forbids longjmp across Rust frames) and the messages are compared.

fn fatal_fixture(case: &str) -> (std::path::PathBuf, &'static std::ffi::CStr, bool, Vec<u8>) {
    let (name, pcx, bytes): (&'static std::ffi::CStr, bool, Vec<u8>) = match case {
        "pcx-bad-signature" => {
            let mut f = build_pcx(2, 2, 2, &[0; 4]);
            f[0] = 0x0B;
            (c"gfx/badsig.pcx", true, f)
        }
        "pcx-bad-version" => {
            let mut f = build_pcx(2, 2, 2, &[0; 4]);
            f[1] = 3;
            (c"gfx/badver.pcx", true, f)
        }
        "pcx-bad-encoding" => {
            let mut f = build_pcx(2, 2, 2, &[0; 4]);
            f[3] = 24;
            (c"gfx/badenc.pcx", true, f)
        }
        "pcx-short-header" => (c"gfx/tiny.pcx", true, vec![0x0A; 12]),
        "lmp-short-header" => (c"gfx/tiny.lmp", false, vec![1, 2, 3, 4]),
        _ => panic!("unknown fatal case {case}"),
    };
    (
        std::path::PathBuf::from(name.to_str().unwrap()),
        name,
        pcx,
        bytes,
    )
}

fn assert_fatal_parity(case: &str) {
    let root;
    let c_msg;
    {
        let _guard = ctfs::lock();
        let dir = file_dir();
        root = dir.parent().unwrap().to_path_buf();
        let (rel, name, pcx, bytes) = fatal_fixture(case);
        std::fs::write(dir.join(rel), bytes).unwrap();

        let mut handle: c_int = -1;
        let mut path_id: c_uint = 0;
        // SAFETY: C searchpaths mounted; out-params valid
        let size =
            unsafe { (ctfs::fns(Side::C).open_file)(name.as_ptr(), &mut handle, &mut path_id) };
        assert!(size >= 0, "fixture {name:?} must open");
        let mut w: c_int = 0;
        let mut h: c_int = 0;
        c_msg = ctfs::catch_sys_error(|| {
            // SAFETY: C frames only under the trap; valid handle/pointers
            unsafe {
                decoder(Side::C, pcx)(handle, &mut w, &mut h, name.as_ptr());
            }
        })
        .expect("C side must Sys_Error");
        // the C decoder never reached COM_CloseFile; drop the handle so
        // later tests still see a balanced count
        // SAFETY: handle is open (the decoder fataled before closing)
        unsafe { (ctfs::fns(Side::C).close_file)(handle) };
    }

    let rust_msg = ctfs::rust_fatal_in_child(
        "rust_fatal_child",
        case,
        &[("CTEST_FATAL_ROOT", root.to_str().unwrap())],
    )
    .expect("Rust side must Sys_Error");
    assert_eq!(c_msg, rust_msg, "Sys_Error parity for {case}");
}

#[test]
fn pcx_bad_signature_fatal_parity() {
    assert_fatal_parity("pcx-bad-signature");
}

#[test]
fn pcx_bad_version_fatal_parity() {
    assert_fatal_parity("pcx-bad-version");
}

#[test]
fn pcx_bad_encoding_fatal_parity() {
    assert_fatal_parity("pcx-bad-encoding");
}

#[test]
fn pcx_short_header_fatal_parity() {
    assert_fatal_parity("pcx-short-header");
}

#[test]
fn lmp_short_header_fatal_parity() {
    assert_fatal_parity("lmp-short-header");
}

/// Child-process half of the fatal-parity tests: runs only when the parent
/// re-executes this binary with CTEST_FATAL_CASE set. The trap is NOT armed;
/// the stub Sys_Error prints and aborts, and the parent reads stderr.
#[test]
fn rust_fatal_child() {
    let Some(case) = ctfs::fatal_child_case() else {
        return;
    };
    let root = std::path::PathBuf::from(std::env::var("CTEST_FATAL_ROOT").expect("root"));
    ctfs::setup(Side::Rust, &[&root], 0, c"imggame");
    let (_, name, pcx, _) = fatal_fixture(&case);

    let mut handle: c_int = -1;
    let mut path_id: c_uint = 0;
    // SAFETY: Rust searchpaths mounted; out-params valid
    let size =
        unsafe { (ctfs::fns(Side::Rust).open_file)(name.as_ptr(), &mut handle, &mut path_id) };
    assert!(size >= 0, "fixture {name:?} must open in child");
    let mut w: c_int = 0;
    let mut h: c_int = 0;
    // SAFETY: open handle, valid out-pointers; expected to Sys_Error (abort)
    unsafe {
        decoder(Side::Rust, pcx)(handle, &mut w, &mut h, name.as_ptr());
    }
    panic!("expected the Rust decoder to Sys_Error for {case}");
}
