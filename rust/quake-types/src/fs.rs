//! Filesystem ABI mirrors (`Quake/common.h` QUAKEFS section, `Quake/pakfile.h`).
//! Compat-critical: under -Duse_rust_fs the Rust shim owns the searchpath
//! list and C code (host_cmd.c, pr_ext.c) walks these structs directly.

use core::ffi::c_char;

/// C: `MAX_OSPATH` = PATH_MAX (q_types.h). Platform-dependent: _MAX_PATH on
/// Windows, sys limits elsewhere; sizes below are cross-checked against the
/// C compiler's sizeof in quake-ctest's layout tests.
#[cfg(windows)]
pub const MAX_OSPATH: usize = 260;
#[cfg(target_os = "macos")]
pub const MAX_OSPATH: usize = 1024;
#[cfg(all(unix, not(target_os = "macos")))]
pub const MAX_OSPATH: usize = 4096;

/// C: `MAX_QPATH` (q_types.h)
pub const MAX_QPATH: usize = 64;

/// C: `dpackfile_t` (pakfile.h, on-disk)
#[repr(C)]
#[derive(Clone, Copy)]
pub struct DPackFile {
    pub name: [c_char; 56],
    pub filepos: i32,
    pub filelen: i32,
}

/// C: `dpackheader_t` (pakfile.h, on-disk)
#[repr(C)]
#[derive(Clone, Copy)]
pub struct DPackHeader {
    pub id: [c_char; 4],
    pub dirofs: i32,
    pub dirlen: i32,
}

/// C: `packfile_t` (common.h)
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PackFile {
    pub name: [c_char; MAX_QPATH],
    pub filepos: i32,
    pub filelen: i32,
}

/// C: `pack_t` (common.h)
#[repr(C)]
pub struct Pack {
    pub filename: [c_char; MAX_OSPATH],
    pub handle: i32,
    pub numfiles: i32,
    pub files: *mut PackFile,
}

/// C: `searchpath_t` (common.h)
#[repr(C)]
pub struct SearchPath {
    pub path_id: u32,
    pub filename: [c_char; MAX_OSPATH],
    pub pack: *mut Pack,
    pub dir: [c_char; MAX_QPATH],
    pub next: *mut SearchPath,
}

const PTR: usize = core::mem::size_of::<*mut ()>();
const fn align_up(n: usize, a: usize) -> usize {
    n.div_ceil(a) * a
}

const _: () = assert!(core::mem::size_of::<DPackFile>() == 64);
const _: () = assert!(core::mem::size_of::<DPackHeader>() == 12);
const _: () = assert!(core::mem::size_of::<PackFile>() == 72);
const _: () = assert!(core::mem::offset_of!(PackFile, filepos) == 56 + 8);

const _: () = assert!(core::mem::offset_of!(Pack, handle) == MAX_OSPATH);
const _: () = assert!(core::mem::offset_of!(Pack, files) == align_up(MAX_OSPATH + 8, PTR));

const _: () = assert!(core::mem::offset_of!(SearchPath, filename) == 4);
const _: () = assert!(core::mem::offset_of!(SearchPath, pack) == align_up(4 + MAX_OSPATH, PTR));
const _: () =
    assert!(core::mem::offset_of!(SearchPath, dir) == align_up(4 + MAX_OSPATH, PTR) + PTR);
const _: () = assert!(
    core::mem::offset_of!(SearchPath, next) == align_up(4 + MAX_OSPATH, PTR) + PTR + MAX_QPATH
);
