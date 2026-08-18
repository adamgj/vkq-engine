//! Differential tests: the Rust wad loaders (quake-capi shims) vs the
//! original wad.c compiled as c_ref_*. Both sides load the same files through
//! the same stub filesystem (a shared temp directory) and the full observable
//! state is compared: the in-place-edited wad_base image, the lump directory,
//! and the wad_t list (order, names, ids, lump arrays).

use core::ffi::{c_char, c_int, c_void, CStr};
use quake_c_sys::fshandle_t;
use quake_ctest as _; // links the cc-built c_ref_* archive
use quake_types::wad::LumpInfo;
use std::sync::Once;

extern "C" {
    static mut c_ref_wad_numlumps: c_int;
    static mut c_ref_wad_base: *mut u8;
    fn c_ref_W_LoadWadFile();
    fn c_ref_W_CleanupName(in_: *const c_char, out: *mut c_char);
    fn c_ref_W_GetLumpName(name: *const c_char, out_info: *mut *mut LumpInfo) -> *mut c_void;
    fn c_ref_W_LoadWadList(names: *const c_char) -> *mut CWad;
    fn c_ref_W_FreeWadList(wads: *mut CWad);
    fn c_ref_W_GetLumpinfoList(
        wads: *mut CWad,
        name: *const c_char,
        out_wad: *mut *mut CWad,
    ) -> *mut LumpInfo;

    fn ctest_set_file_dir(dir: *const c_char);
}

/// wad_t through the same layout the Rust shim asserts.
#[repr(C)]
struct CWad {
    name: [c_char; 64],
    id: c_int,
    fh: fshandle_t,
    numlumps: c_int,
    lumps: *mut LumpInfo,
    next: *mut CWad,
}

static SETUP: Once = Once::new();

fn file_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("quake-ctest-wad-{}", std::process::id()));
    SETUP.call_once(|| {
        std::fs::create_dir_all(dir.join("gfx")).unwrap();
        let cdir = std::ffi::CString::new(dir.to_str().unwrap()).unwrap();
        // SAFETY: NUL-terminated path; called once before any loads
        unsafe { ctest_set_file_dir(cdir.as_ptr()) };
    });
    dir
}

/// Builds a WAD2 image from (name, type, payload) entries, with optional
/// header/lump corruption applied afterwards.
fn build_wad(magic: &[u8; 4], entries: &[(&[u8], i8, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(magic);
    let mut payloads = Vec::new();
    let mut offset = 12usize;
    for (_, _, data) in entries {
        payloads.push((offset, data.len()));
        offset += data.len();
    }
    out.extend_from_slice(&(entries.len() as i32).to_le_bytes());
    out.extend_from_slice(&(offset as i32).to_le_bytes());
    for (_, _, data) in entries {
        out.extend_from_slice(data);
    }
    for (i, (name, type_, data)) in entries.iter().enumerate() {
        out.extend_from_slice(&(payloads[i].0 as i32).to_le_bytes()); // filepos
        out.extend_from_slice(&(data.len() as i32).to_le_bytes()); // disksize
        out.extend_from_slice(&(data.len() as i32).to_le_bytes()); // size
        out.push(*type_ as u8);
        out.push(0); // compression
        out.push(0);
        out.push(0);
        let mut n = [0u8; 16];
        n[..name.len().min(16)].copy_from_slice(&name[..name.len().min(16)]);
        out.extend_from_slice(&n);
    }
    out
}

fn qpic_payload(w: i32, h: i32) -> Vec<u8> {
    let mut p = Vec::new();
    p.extend_from_slice(&w.to_le_bytes());
    p.extend_from_slice(&h.to_le_bytes());
    p.extend(std::iter::repeat_n(0x42u8, (w * h) as usize));
    p
}

/// Loads `file` (already written as gfx.wad) through both implementations and
/// compares the full observable state.
fn compare_gfx_wad_load(file_len: usize) {
    // SAFETY: both loaders own their separate global sets; the stub COM_LoadFile
    // hands each a fresh buffer
    unsafe {
        c_ref_W_LoadWadFile();
        quake_rs::wad::W_LoadWadFile();
        let c_len = file_len as i64;
        let r_len = file_len as i64;

        let c_numlumps = core::ptr::addr_of!(c_ref_wad_numlumps).read();
        let r_numlumps = core::ptr::addr_of!(quake_rs::wad::wad_numlumps).read();
        assert_eq!(c_numlumps, r_numlumps, "numlumps");
        // the entire in-place-edited file image must match byte for byte
        let c_img =
            core::slice::from_raw_parts(core::ptr::addr_of!(c_ref_wad_base).read(), c_len as usize);
        let r_img = core::slice::from_raw_parts(
            core::ptr::addr_of!(quake_rs::wad::wad_base).read(),
            r_len as usize,
        );
        assert_eq!(c_img, r_img, "wad_base image after load");

        // lump lookups agree (present and missing)
        for name in [c"pic_a", c"PIC_A", c"flat", c"missing", c"conchars"] {
            let mut c_info: *mut LumpInfo = core::ptr::null_mut();
            let mut r_info: *mut LumpInfo = core::ptr::null_mut();
            let c_data = c_ref_W_GetLumpName(name.as_ptr(), &mut c_info);
            let r_data = quake_rs::wad::W_GetLumpName(name.as_ptr(), &mut r_info);
            assert_eq!(c_data.is_null(), r_data.is_null(), "lookup {name:?}");
            if !c_data.is_null() {
                // same offsets within the respective images
                assert_eq!(
                    c_data as usize - core::ptr::addr_of!(c_ref_wad_base).read() as usize,
                    r_data as usize - core::ptr::addr_of!(quake_rs::wad::wad_base).read() as usize,
                    "data offset for {name:?}"
                );
                assert_eq!((*c_info).filepos, (*r_info).filepos, "filepos for {name:?}");
                assert_eq!((*c_info).size, (*r_info).size, "size for {name:?}");
            }
        }
    }
}

#[test]
fn gfx_wad_healthy_and_corrupt() {
    let dir = file_dir();

    let cases: Vec<Vec<u8>> = vec![
        // healthy wad with a qpic (SwapPic applies) and a plain lump
        build_wad(
            b"WAD2",
            &[
                (b"PIC_A", 66, qpic_payload(4, 2)),
                (b"FLAT", 64, vec![1, 2, 3, 4]),
                (b"SIXTEENCHARNAMEX", 64, vec![9; 8]),
            ],
        ),
        // wrong magic: loader keeps going with zero lumps
        build_wad(b"WAD3", &[(b"X", 64, vec![0; 4])]),
        // truncated directory: header extends beyond end
        {
            let mut w = build_wad(b"WAD2", &[(b"PIC_A", 66, qpic_payload(2, 2))]);
            w[4..8].copy_from_slice(&1000i32.to_le_bytes());
            w
        },
        // lump begins beyond end of file
        {
            let mut w = build_wad(b"WAD2", &[(b"BAD", 64, vec![0; 4])]);
            let dirofs = 12 + 4;
            w[dirofs..dirofs + 4].copy_from_slice(&100000i32.to_le_bytes());
            w
        },
        // lump size overruns but disksize fits (falls back to disksize)
        {
            let mut w = build_wad(b"WAD2", &[(b"OVR", 64, vec![7; 10])]);
            let dirofs = 12 + 10 + 8;
            w[dirofs..dirofs + 4].copy_from_slice(&50000i32.to_le_bytes()); // size
            w
        },
        // lump extends beyond the end and disksize does not rescue it: the only
        // way to reach the repair's q_max (0, size - filepos) clamp
        // (LumpProblem::ExtendsBeyond)
        {
            let mut w = build_wad(b"WAD2", &[(b"EXT", 64, vec![7; 10])]);
            let dirofs = 12 + 10;
            w[dirofs + 4..dirofs + 8].copy_from_slice(&50000i32.to_le_bytes()); // disksize
            w[dirofs + 8..dirofs + 12].copy_from_slice(&50000i32.to_le_bytes()); // size
            w
        },
        // negative lump size
        {
            let mut w = build_wad(b"WAD2", &[(b"NEG", 64, vec![7; 10])]);
            let dirofs = 12 + 10 + 8;
            w[dirofs..dirofs + 4].copy_from_slice(&(-5i32).to_le_bytes());
            w
        },
    ];

    for (i, wad) in cases.iter().enumerate() {
        std::fs::write(dir.join("gfx.wad"), wad).unwrap();
        eprintln!("gfx.wad case {i}");
        compare_gfx_wad_load(wad.len());
    }
}

#[test]
fn cleanup_name_matches() {
    let inputs: &[&[u8]] = &[
        b"UPPER",
        b"lower",
        b"MiXeD123",
        b"",
        b"exactly16charsXY",
        b"way_more_than_sixteen_characters",
        b"odd{}[]!",
    ];
    for input in inputs {
        let mut z = input.to_vec();
        z.push(0);
        let mut c_out = [0x7fu8; 16];
        let mut r_out = [0x7fu8; 16];
        // SAFETY: in buffers NUL-terminated; out buffers 16 bytes
        unsafe {
            c_ref_W_CleanupName(
                z.as_ptr() as *const c_char,
                c_out.as_mut_ptr() as *mut c_char,
            );
            quake_rs::wad::W_CleanupName(
                z.as_ptr() as *const c_char,
                r_out.as_mut_ptr() as *mut c_char,
            );
        }
        assert_eq!(
            c_out,
            r_out,
            "cleanup of {:?}",
            String::from_utf8_lossy(input)
        );
    }
}

#[test]
fn wad_list_matches() {
    let dir = file_dir();

    // a valid wad reachable directly, one only under gfx/, one invalid, one
    // empty; editor-path garbage and empty segments in the list
    std::fs::write(
        dir.join("texlist.wad"),
        build_wad(
            b"WAD2",
            &[(b"BRICK", 68, vec![1; 16]), (b"WATER", 68, vec![2; 16])],
        ),
    )
    .unwrap();
    std::fs::write(
        dir.join("gfx").join("gfxonly.wad"),
        build_wad(b"WAD3", &[(b"SLIME", 68, vec![3; 16])]),
    )
    .unwrap();
    std::fs::write(
        dir.join("badmagic.wad"),
        build_wad(b"PACK", &[(b"X", 68, vec![0; 4])]),
    )
    .unwrap();
    std::fs::write(dir.join("empty.wad"), build_wad(b"WAD2", &[])).unwrap();
    // same ExtendsBeyond clamp, but through W_AddWadFile: it cleans the name
    // *before* warning and prints the size with %i instead of %u, so the two
    // loaders' repair paths are not interchangeable
    std::fs::write(dir.join("extlump.wad"), {
        let mut w = build_wad(b"WAD2", &[(b"EXTLUMP", 68, vec![5; 16])]);
        let dirofs = 12 + 16;
        w[dirofs + 4..dirofs + 8].copy_from_slice(&50000i32.to_le_bytes()); // disksize
        w[dirofs + 8..dirofs + 12].copy_from_slice(&50000i32.to_le_bytes()); // size
        w
    })
    .unwrap();

    let names = c"C:\\editor\\path\\texlist.wad;gfxonly;badmagic.wad;;empty;extlump;missing.wad";

    // SAFETY: both lists are built and freed here; layouts are asserted in
    // the Rust shim and mirrored by CWad
    unsafe {
        let c_list = c_ref_W_LoadWadList(names.as_ptr());
        let r_list = quake_rs::wad::W_LoadWadList(names.as_ptr()) as *mut CWad;

        // walk both lists comparing order and contents
        let (mut c, mut r) = (c_list, r_list);
        let mut count = 0;
        while !c.is_null() || !r.is_null() {
            assert!(
                !c.is_null() && !r.is_null(),
                "list length mismatch at {count}"
            );
            assert_eq!(
                CStr::from_ptr((*c).name.as_ptr()),
                CStr::from_ptr((*r).name.as_ptr()),
                "wad name at {count}"
            );
            assert_eq!((*c).id, (*r).id, "id at {count}");
            assert_eq!((*c).numlumps, (*r).numlumps, "numlumps at {count}");
            let c_lumps =
                core::slice::from_raw_parts((*c).lumps as *const u8, (*c).numlumps as usize * 32);
            let r_lumps =
                core::slice::from_raw_parts((*r).lumps as *const u8, (*r).numlumps as usize * 32);
            assert_eq!(c_lumps, r_lumps, "lump directory at {count}");
            c = (*c).next;
            r = (*r).next;
            count += 1;
        }
        assert_eq!(count, 3, "expected texlist + gfxonly + extlump to load");

        // list-wide lookup agrees
        for name in [c"BRICK", c"slime", c"nope"] {
            let mut c_wad: *mut CWad = core::ptr::null_mut();
            let mut r_wad: *mut CWad = core::ptr::null_mut();
            let c_info = c_ref_W_GetLumpinfoList(c_list, name.as_ptr(), &mut c_wad);
            let r_info = quake_rs::wad::W_GetLumpinfoList(
                r_list as *mut quake_rs::wad::Wad,
                name.as_ptr(),
                &mut r_wad as *mut *mut CWad as *mut *mut quake_rs::wad::Wad,
            );
            assert_eq!(c_info.is_null(), r_info.is_null(), "list lookup {name:?}");
            if !c_info.is_null() {
                assert_eq!((*c_info).filepos, (*r_info).filepos);
                assert_eq!(
                    CStr::from_ptr((*c_wad).name.as_ptr()),
                    CStr::from_ptr((*r_wad).name.as_ptr()),
                    "owning wad for {name:?}"
                );
            }
        }

        c_ref_W_FreeWadList(c_list);
        quake_rs::wad::W_FreeWadList(r_list as *mut quake_rs::wad::Wad);
    }
}
