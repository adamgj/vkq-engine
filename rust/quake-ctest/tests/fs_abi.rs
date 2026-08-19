//! ABI cross-check: the `quake_types::fs` mirrors vs what the engine's own
//! headers say on this platform.
//!
//! `MAX_OSPATH` is `PATH_MAX`, which `q_types.h` derives from a four-way
//! `MAXPATHLEN`/`_MAX_PATH`/`PATH_MAX`/fallback chain. The Rust side cannot
//! read that at build time (the bindgen output is generated on one host and
//! committed), so `quake_types::fs` hardcodes a per-target `cfg` ladder --
//! a guess about the C toolchain. Under `-Duse_rust_fs` the C walks
//! Rust-allocated `searchpath_t`/`pack_t` nodes directly, so a wrong guess
//! is silent memory corruption rather than a link error.
//!
//! This test is the check that makes the guess safe. It runs on every OS in
//! the CI test matrix; a platform whose `PATH_MAX` differs from the ladder
//! (FreeBSD, an unusual libc, a toolchain that caps `_MAX_PATH`) fails here
//! instead of corrupting the searchpath list at runtime.

use core::mem::{offset_of, size_of};

use quake_ctest as _;
use quake_types::fs::{DPackFile, DPackHeader, Pack, PackFile, SearchPath, MAX_OSPATH, MAX_QPATH};

extern "C" {
    fn ctest_abi_max_ospath() -> usize;
    fn ctest_abi_max_qpath() -> usize;
    fn ctest_abi_sizeof(which: core::ffi::c_int) -> usize;
    fn ctest_abi_offsetof(which: core::ffi::c_int) -> usize;
}

fn c_sizeof(which: core::ffi::c_int) -> usize {
    // SAFETY: `which` is one of the indices the probe's switch handles; the
    // function only reads compile-time constants.
    unsafe { ctest_abi_sizeof(which) }
}

fn c_offsetof(which: core::ffi::c_int) -> usize {
    // SAFETY: as above.
    unsafe { ctest_abi_offsetof(which) }
}

#[test]
fn path_constants_match_the_c_headers() {
    // SAFETY: constant getters, no arguments.
    let (c_ospath, c_qpath) = unsafe { (ctest_abi_max_ospath(), ctest_abi_max_qpath()) };
    assert_eq!(
        MAX_OSPATH, c_ospath,
        "quake_types::fs::MAX_OSPATH ({MAX_OSPATH}) disagrees with the C's \
         MAX_OSPATH ({c_ospath}) on this target -- fix the cfg ladder in \
         quake-types/src/fs.rs and quake-c-sys (gen_c_bindings.sh raw lines)"
    );
    assert_eq!(MAX_QPATH, c_qpath, "MAX_QPATH");
}

#[test]
fn struct_sizes_match_the_c_headers() {
    assert_eq!(size_of::<SearchPath>(), c_sizeof(0), "sizeof searchpath_t");
    assert_eq!(size_of::<Pack>(), c_sizeof(1), "sizeof pack_t");
    assert_eq!(size_of::<PackFile>(), c_sizeof(2), "sizeof packfile_t");
    assert_eq!(size_of::<DPackFile>(), c_sizeof(3), "sizeof dpackfile_t");
    assert_eq!(
        size_of::<DPackHeader>(),
        c_sizeof(4),
        "sizeof dpackheader_t"
    );
}

#[test]
fn field_offsets_match_the_c_headers() {
    // searchpath_t: the list C walks directly under -Duse_rust_fs
    assert_eq!(
        offset_of!(SearchPath, path_id),
        c_offsetof(0),
        "searchpath_t.path_id"
    );
    assert_eq!(
        offset_of!(SearchPath, filename),
        c_offsetof(1),
        "searchpath_t.filename"
    );
    assert_eq!(
        offset_of!(SearchPath, pack),
        c_offsetof(2),
        "searchpath_t.pack"
    );
    assert_eq!(
        offset_of!(SearchPath, dir),
        c_offsetof(3),
        "searchpath_t.dir"
    );
    assert_eq!(
        offset_of!(SearchPath, next),
        c_offsetof(4),
        "searchpath_t.next"
    );

    // pack_t: host_cmd.c and pr_ext.c read filename/numfiles off these
    assert_eq!(offset_of!(Pack, filename), c_offsetof(5), "pack_t.filename");
    assert_eq!(offset_of!(Pack, handle), c_offsetof(6), "pack_t.handle");
    assert_eq!(offset_of!(Pack, numfiles), c_offsetof(7), "pack_t.numfiles");
    assert_eq!(offset_of!(Pack, files), c_offsetof(8), "pack_t.files");

    // packfile_t: the directory array the shim fills and C searches
    assert_eq!(offset_of!(PackFile, name), c_offsetof(9), "packfile_t.name");
    assert_eq!(
        offset_of!(PackFile, filepos),
        c_offsetof(10),
        "packfile_t.filepos"
    );
    assert_eq!(
        offset_of!(PackFile, filelen),
        c_offsetof(11),
        "packfile_t.filelen"
    );
}
