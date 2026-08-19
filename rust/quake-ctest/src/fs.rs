//! Phase 2 differential-test support: a side-agnostic driver over the two
//! filesystem implementations — the reference C (`c_ref_*`, compiled from
//! Quake/common_fs.c + steam.c by build.rs) and the Rust shims (`quake_rs`
//! with the `fs` feature).
//!
//! Both sides run over the exact same stubs (stubs/stubs.c): the Sys_File*
//! handle table, host_parms, the console/cvar capture logs and the Sys_Error
//! longjmp trap, so their observable behavior is directly comparable.
//!
//! Everything here is main-thread-only, like the engine globals it touches;
//! tests serialize through [`FS_LOCK`].

use core::ffi::{c_char, c_int, c_uint, c_void, CStr};
use core::ptr;
use quake_c_sys::{qfilesize_t, steamgame_t, FILE};
use quake_types::fs::{Pack, SearchPath, MAX_OSPATH};
use std::sync::Mutex;

/// Serializes every test that touches the (global, per-side) fs state or the
/// shared stub state (handle table, capture logs, Sys_Error trap).
pub static FS_LOCK: Mutex<()> = Mutex::new(());

/// Poison-tolerant [`FS_LOCK`] acquisition: a panicking test must not turn
/// every later fs test into a PoisonError failure.
pub fn lock() -> std::sync::MutexGuard<'static, ()> {
    FS_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

pub const MAX_BASEDIRS: usize = 4;

mod cref {
    use super::*;

    extern "C" {
        // ---- c_ref globals (common_fs.c compiled under the rename prelude)
        pub static mut c_ref_com_searchpaths: *mut SearchPath;
        pub static mut c_ref_com_base_searchpaths: *mut SearchPath;
        pub static mut c_ref_com_basedir: [c_char; MAX_OSPATH];
        pub static mut c_ref_com_basedirs: [[c_char; MAX_OSPATH]; MAX_BASEDIRS];
        pub static mut c_ref_com_numbasedirs: c_int;
        pub static mut c_ref_com_gamedir: [c_char; MAX_OSPATH];
        pub static mut c_ref_com_gamenames: [c_char; 1024];
        pub static mut c_ref_com_modified: bool;
        pub static mut c_ref_standard_quake: bool;
        pub static mut c_ref_rogue: bool;
        pub static mut c_ref_hipnotic: bool;

        // ---- c_ref entry points
        pub fn c_ref_COM_AddBaseDir(dir: *const c_char);
        pub fn c_ref_COM_ResetGameDirectories(newdirs: *const c_char);
        pub fn c_ref_COM_InitFilesystem();
        pub fn c_ref_COM_FileExists(filename: *const c_char, path_id: *mut c_uint) -> bool;
        pub fn c_ref_COM_OpenFile(
            filename: *const c_char,
            handle: *mut c_int,
            path_id: *mut c_uint,
        ) -> qfilesize_t;
        pub fn c_ref_COM_FOpenFile(
            filename: *const c_char,
            file: *mut *mut FILE,
            path_id: *mut c_uint,
        ) -> qfilesize_t;
        pub fn c_ref_COM_CloseFile(h: c_int);
        pub fn c_ref_COM_LoadFile(path: *const c_char, path_id: *mut c_uint) -> *mut u8;
        pub fn c_ref_COM_LoadMallocFile_TextMode_OSPath(
            path: *const c_char,
            len_out: *mut core::ffi::c_long,
        ) -> *mut u8;
        pub fn c_ref_COM_WriteFile(filename: *const c_char, data: *const c_void, len: c_int);
        pub fn c_ref_COM_FOpenPrefFile(filename: *const c_char, mode: *const c_char) -> *mut FILE;
        pub fn c_ref_COM_GetGameNames(full: bool) -> *const c_char;
        pub fn c_ref_COM_GameDirMatches(tdirs: *const c_char) -> bool;
        pub fn c_ref_COM_ModForbiddenChars(p: *const c_char) -> bool;
        pub fn c_ref_COM_HashString(s: *const c_char) -> c_uint;
        pub fn c_ref_COM_HashBlock(data: *const c_void, size: usize) -> c_uint;
        pub fn c_ref_LOC_Init();
        pub fn c_ref_LOC_Shutdown();
        pub fn c_ref_LOC_GetRawString(key: *const c_char) -> *const c_char;
        pub fn c_ref_LOC_GetString(key: *const c_char) -> *const c_char;
        pub fn c_ref_LOC_HasPlaceholders(s: *const c_char) -> bool;
        pub fn c_ref_LOC_Format(
            format: *const c_char,
            getarg_fn: Option<unsafe extern "C" fn(c_int, *mut c_void) -> *const c_char>,
            userdata: *mut c_void,
            out: *mut c_char,
            len: usize,
        ) -> usize;
        pub fn c_ref_Steam_IsValidPath(path: *const c_char) -> bool;
        pub fn c_ref_Steam_FindGame(game: *mut steamgame_t, appid: c_int) -> bool;
        pub fn c_ref_Steam_ResolvePath(
            path: *mut c_char,
            pathsize: usize,
            game: *const steamgame_t,
        ) -> bool;
        pub fn c_ref_EGS_FindGame(
            path: *mut c_char,
            pathsize: usize,
            nspace: *const c_char,
            itemid: *const c_char,
            appname: *const c_char,
        ) -> bool;
    }
}

mod stub {
    use super::*;

    extern "C" {
        pub fn ctest_try(fn_: unsafe extern "C" fn(*mut c_void), arg: *mut c_void) -> c_int;
        pub fn ctest_sys_error_message() -> *const c_char;
        pub fn ctest_clear_con_log();
        pub fn ctest_con_log_len() -> c_int;
        pub fn ctest_con_log_get(i: c_int) -> *const c_char;
        pub fn ctest_clear_cvar_log();
        pub fn ctest_cvar_log_len() -> c_int;
        pub fn ctest_cvar_log_get(i: c_int, which: c_int) -> *const c_char;
        pub fn ctest_set_args(argc: c_int, argv: *mut *mut c_char);
        pub fn ctest_set_host_dirs(basedir: *const c_char, userdir: *const c_char);
        pub fn ctest_set_pref_path(path: *const c_char);
        pub fn ctest_set_steam_dir(path: *const c_char);
        pub fn ctest_set_gog_dir(path: *const c_char);
        pub fn ctest_set_gog_enhanced_dir(path: *const c_char);
        pub fn ctest_set_egs_manifest_dir(path: *const c_char);
        pub fn ctest_set_nightdive_dir(path: *const c_char);
        pub fn ctest_set_egs_launcher_data(json: *const c_char);
        pub fn ctest_open_handle_count() -> c_int;
    }
}

// ---------------------------------------------------------------------------
// stub state helpers (shared by both sides)

fn to_cstring(s: &str) -> std::ffi::CString {
    std::ffi::CString::new(s).unwrap()
}

/// Sets host_parms->basedir / ->userdir. `userdir: None` aliases the userdir
/// pointer to the basedir pointer, the engine's "no userdir" state.
pub fn set_host_dirs(basedir: &str, userdir: Option<&str>) {
    let b = to_cstring(basedir);
    let u = userdir.map(to_cstring);
    // SAFETY: NUL-terminated strings; the stub copies them into static bufs
    unsafe {
        stub::ctest_set_host_dirs(b.as_ptr(), u.as_ref().map_or(ptr::null(), |u| u.as_ptr()))
    };
}

pub fn set_pref_path(path: Option<&str>) {
    let p = path.map(to_cstring);
    // SAFETY: NUL-terminated or NULL; the stub copies
    unsafe { stub::ctest_set_pref_path(p.as_ref().map_or(ptr::null(), |p| p.as_ptr())) };
}

macro_rules! store_setter {
    ($name:ident, $stub:ident) => {
        pub fn $name(path: Option<&str>) {
            let p = path.map(to_cstring);
            // SAFETY: NUL-terminated or NULL; the stub copies
            unsafe { stub::$stub(p.as_ref().map_or(ptr::null(), |p| p.as_ptr())) };
        }
    };
}

store_setter!(set_steam_dir, ctest_set_steam_dir);
store_setter!(set_gog_dir, ctest_set_gog_dir);
store_setter!(set_gog_enhanced_dir, ctest_set_gog_enhanced_dir);
store_setter!(set_egs_manifest_dir, ctest_set_egs_manifest_dir);
store_setter!(set_nightdive_dir, ctest_set_nightdive_dir);
store_setter!(set_egs_launcher_data, ctest_set_egs_launcher_data);

/// Sets the shared command line. Returns the CStrings that must stay alive
/// while the args are in use.
pub fn set_args(args: &[&str]) -> Vec<std::ffi::CString> {
    let owned: Vec<std::ffi::CString> = args.iter().map(|a| to_cstring(a)).collect();
    let mut argv: Vec<*mut c_char> = owned.iter().map(|a| a.as_ptr() as *mut c_char).collect();
    // SAFETY: argv pointers stay valid while `owned` lives (returned to the
    // caller); the stub copies the pointer array
    unsafe { stub::ctest_set_args(argv.len() as c_int, argv.as_mut_ptr()) };
    owned
}

pub fn clear_logs() {
    // SAFETY: plain stub calls, no arguments
    unsafe {
        stub::ctest_clear_con_log();
        stub::ctest_clear_cvar_log();
    }
}

/// Snapshot of the console capture log (Con_*/Sys_Printf + registration
/// events), tagged per channel by the stub.
pub fn con_log() -> Vec<String> {
    // SAFETY: indices bounded by the stub's count; entries are NUL-terminated
    unsafe {
        let n = stub::ctest_con_log_len().min(256);
        (0..n)
            .map(|i| {
                CStr::from_ptr(stub::ctest_con_log_get(i))
                    .to_string_lossy()
                    .into_owned()
            })
            .collect()
    }
}

/// Snapshot of the Cvar_Set capture log.
pub fn cvar_log() -> Vec<(String, String)> {
    // SAFETY: indices bounded by the stub's count; entries are NUL-terminated
    unsafe {
        let n = stub::ctest_cvar_log_len().min(128);
        (0..n)
            .map(|i| {
                (
                    CStr::from_ptr(stub::ctest_cvar_log_get(i, 0))
                        .to_string_lossy()
                        .into_owned(),
                    CStr::from_ptr(stub::ctest_cvar_log_get(i, 1))
                        .to_string_lossy()
                        .into_owned(),
                )
            })
            .collect()
    }
}

pub fn open_handle_count() -> i32 {
    // SAFETY: plain stub call
    unsafe { stub::ctest_open_handle_count() }
}

/// Runs `f` with the stub Sys_Error longjmp trap armed. Returns the formatted
/// Sys_Error message if one fired, None if `f` completed.
///
/// **Only wrap work that either cannot fatal, or runs entirely in C frames**
/// (i.e. `Side::C`). PLAN.md §4 forbids a longjmp unwinding a Rust frame, and
/// this is why: the jump skips the Rust shim's frames without running
/// destructors, which macOS/Linux tolerate but MSVC reports as
/// STATUS_HEAP_CORRUPTION. Rust-side fatal inputs go through
/// [`rust_fatal_in_child`] instead. (The shipped engine is unaffected —
/// `Sys_Error` there ends in `exit(1)`, so nothing unwinds.)
pub fn catch_sys_error<F: FnMut()>(mut f: F) -> Option<String> {
    unsafe extern "C" fn trampoline<F: FnMut()>(arg: *mut c_void) {
        // SAFETY: arg is the &mut F passed below, alive for the whole call
        let f = unsafe { &mut *(arg as *mut F) };
        f();
    }
    // SAFETY: the trampoline is monomorphized for F and receives &mut f
    let hit = unsafe { stub::ctest_try(trampoline::<F>, &mut f as *mut F as *mut c_void) };
    if hit != 0 {
        // SAFETY: the stub message buffer is static and NUL-terminated
        let msg = unsafe { CStr::from_ptr(stub::ctest_sys_error_message()) };
        Some(msg.to_string_lossy().into_owned())
    } else {
        None
    }
}

/// registered cvar value (the shareware gate); shared by both sides.
pub fn set_registered(value: f32) {
    // SAFETY: the stub cvar global, main-thread access under FS_LOCK
    unsafe { quake_c_sys::registered.value = value };
}

/// developer cvar value; shared by both sides.
pub fn set_developer(value: f32) {
    // SAFETY: the stub cvar global, main-thread access under FS_LOCK
    unsafe { quake_c_sys::developer.value = value };
}

pub fn set_harness_active(active: bool) {
    // SAFETY: the stub global, main-thread access under FS_LOCK
    unsafe { quake_c_sys::harness_active = active };
}

pub fn thread_file_size() -> i64 {
    // SAFETY: TLS accessor over the stub state
    unsafe { quake_c_sys::COM_ThreadFileSize() }
}

pub fn thread_file_from_pak() -> i32 {
    // SAFETY: TLS accessor over the stub state
    unsafe { quake_c_sys::COM_ThreadFileFromPak() }
}

// ---------------------------------------------------------------------------
// per-side dispatch

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    C,
    Rust,
}

pub const BOTH: [Side; 2] = [Side::C, Side::Rust];

/// Raw pointers to one side's fs globals.
struct SidePtrs {
    searchpaths: *mut *mut SearchPath,
    base_searchpaths: *mut *mut SearchPath,
    basedir: *mut c_char,
    basedirs: *mut c_char,
    numbasedirs: *mut c_int,
    gamedir: *mut c_char,
    gamenames: *mut c_char,
    modified: *mut bool,
    standard_quake: *mut bool,
    rogue: *mut bool,
    hipnotic: *mut bool,
}

fn ptrs(side: Side) -> SidePtrs {
    match side {
        // addresses of the c_ref globals; never dereferenced here
        Side::C => SidePtrs {
            searchpaths: ptr::addr_of_mut!(cref::c_ref_com_searchpaths),
            base_searchpaths: ptr::addr_of_mut!(cref::c_ref_com_base_searchpaths),
            basedir: ptr::addr_of_mut!(cref::c_ref_com_basedir).cast(),
            basedirs: ptr::addr_of_mut!(cref::c_ref_com_basedirs).cast(),
            numbasedirs: ptr::addr_of_mut!(cref::c_ref_com_numbasedirs),
            gamedir: ptr::addr_of_mut!(cref::c_ref_com_gamedir).cast(),
            gamenames: ptr::addr_of_mut!(cref::c_ref_com_gamenames).cast(),
            modified: ptr::addr_of_mut!(cref::c_ref_com_modified),
            standard_quake: ptr::addr_of_mut!(cref::c_ref_standard_quake),
            rogue: ptr::addr_of_mut!(cref::c_ref_rogue),
            hipnotic: ptr::addr_of_mut!(cref::c_ref_hipnotic),
        },
        // addresses of the quake_rs::fs statics; never dereferenced here
        Side::Rust => SidePtrs {
            searchpaths: ptr::addr_of_mut!(quake_rs::fs::com_searchpaths),
            base_searchpaths: ptr::addr_of_mut!(quake_rs::fs::com_base_searchpaths),
            basedir: ptr::addr_of_mut!(quake_rs::fs::com_basedir).cast(),
            basedirs: ptr::addr_of_mut!(quake_rs::fs::com_basedirs).cast(),
            numbasedirs: ptr::addr_of_mut!(quake_rs::fs::com_numbasedirs),
            gamedir: ptr::addr_of_mut!(quake_rs::fs::com_gamedir).cast(),
            gamenames: ptr::addr_of_mut!(quake_rs::fs::com_gamenames).cast(),
            modified: ptr::addr_of_mut!(quake_rs::fs::com_modified),
            standard_quake: ptr::addr_of_mut!(quake_rs::fs::standard_quake),
            rogue: ptr::addr_of_mut!(quake_rs::fs::rogue),
            hipnotic: ptr::addr_of_mut!(quake_rs::fs::hipnotic),
        },
    }
}

/// One side's entry points, unified behind identical signatures.
#[allow(clippy::type_complexity)]
pub struct SideFns {
    pub add_base_dir: unsafe extern "C" fn(*const c_char),
    pub reset_game_directories: unsafe extern "C" fn(*const c_char),
    pub init_filesystem: unsafe extern "C" fn(),
    pub file_exists: unsafe extern "C" fn(*const c_char, *mut c_uint) -> bool,
    pub open_file: unsafe extern "C" fn(*const c_char, *mut c_int, *mut c_uint) -> qfilesize_t,
    pub fopen_file: unsafe extern "C" fn(*const c_char, *mut *mut FILE, *mut c_uint) -> qfilesize_t,
    pub close_file: unsafe extern "C" fn(c_int),
    pub load_file: unsafe extern "C" fn(*const c_char, *mut c_uint) -> *mut u8,
    pub load_malloc_file_textmode:
        unsafe extern "C" fn(*const c_char, *mut core::ffi::c_long) -> *mut u8,
    pub write_file: unsafe extern "C" fn(*const c_char, *const c_void, c_int),
    pub fopen_pref_file: unsafe extern "C" fn(*const c_char, *const c_char) -> *mut FILE,
    pub get_game_names: unsafe extern "C" fn(bool) -> *const c_char,
    pub game_dir_matches: unsafe extern "C" fn(*const c_char) -> bool,
    pub mod_forbidden_chars: unsafe extern "C" fn(*const c_char) -> bool,
    pub hash_string: unsafe extern "C" fn(*const c_char) -> c_uint,
    pub hash_block: unsafe extern "C" fn(*const c_void, usize) -> c_uint,
    pub loc_init: unsafe extern "C" fn(),
    pub loc_shutdown: unsafe extern "C" fn(),
    pub loc_get_raw_string: unsafe extern "C" fn(*const c_char) -> *const c_char,
    pub loc_get_string: unsafe extern "C" fn(*const c_char) -> *const c_char,
    pub loc_has_placeholders: unsafe extern "C" fn(*const c_char) -> bool,
    pub loc_format: unsafe extern "C" fn(
        *const c_char,
        Option<unsafe extern "C" fn(c_int, *mut c_void) -> *const c_char>,
        *mut c_void,
        *mut c_char,
        usize,
    ) -> usize,
    pub steam_is_valid_path: unsafe extern "C" fn(*const c_char) -> bool,
    pub steam_find_game: unsafe extern "C" fn(*mut steamgame_t, c_int) -> bool,
    pub steam_resolve_path: unsafe extern "C" fn(*mut c_char, usize, *const steamgame_t) -> bool,
    pub egs_find_game: unsafe extern "C" fn(
        *mut c_char,
        usize,
        *const c_char,
        *const c_char,
        *const c_char,
    ) -> bool,
}

static C_FNS: SideFns = SideFns {
    add_base_dir: cref::c_ref_COM_AddBaseDir,
    reset_game_directories: cref::c_ref_COM_ResetGameDirectories,
    init_filesystem: cref::c_ref_COM_InitFilesystem,
    file_exists: cref::c_ref_COM_FileExists,
    open_file: cref::c_ref_COM_OpenFile,
    fopen_file: cref::c_ref_COM_FOpenFile,
    close_file: cref::c_ref_COM_CloseFile,
    load_file: cref::c_ref_COM_LoadFile,
    load_malloc_file_textmode: cref::c_ref_COM_LoadMallocFile_TextMode_OSPath,
    write_file: cref::c_ref_COM_WriteFile,
    fopen_pref_file: cref::c_ref_COM_FOpenPrefFile,
    get_game_names: cref::c_ref_COM_GetGameNames,
    game_dir_matches: cref::c_ref_COM_GameDirMatches,
    mod_forbidden_chars: cref::c_ref_COM_ModForbiddenChars,
    hash_string: cref::c_ref_COM_HashString,
    hash_block: cref::c_ref_COM_HashBlock,
    loc_init: cref::c_ref_LOC_Init,
    loc_shutdown: cref::c_ref_LOC_Shutdown,
    loc_get_raw_string: cref::c_ref_LOC_GetRawString,
    loc_get_string: cref::c_ref_LOC_GetString,
    loc_has_placeholders: cref::c_ref_LOC_HasPlaceholders,
    loc_format: cref::c_ref_LOC_Format,
    steam_is_valid_path: cref::c_ref_Steam_IsValidPath,
    steam_find_game: cref::c_ref_Steam_FindGame,
    steam_resolve_path: cref::c_ref_Steam_ResolvePath,
    egs_find_game: cref::c_ref_EGS_FindGame,
};

static RUST_FNS: SideFns = SideFns {
    add_base_dir: quake_rs::fs::COM_AddBaseDir,
    reset_game_directories: quake_rs::fs::COM_ResetGameDirectories,
    init_filesystem: quake_rs::fs::COM_InitFilesystem,
    file_exists: quake_rs::fs::COM_FileExists,
    open_file: quake_rs::fs::COM_OpenFile,
    fopen_file: quake_rs::fs::COM_FOpenFile,
    close_file: quake_rs::fs::COM_CloseFile,
    load_file: quake_rs::fs::COM_LoadFile,
    load_malloc_file_textmode: quake_rs::fs::COM_LoadMallocFile_TextMode_OSPath,
    write_file: quake_rs::fs::COM_WriteFile,
    fopen_pref_file: quake_rs::fs::COM_FOpenPrefFile,
    get_game_names: quake_rs::fs::COM_GetGameNames,
    game_dir_matches: quake_rs::fs::COM_GameDirMatches,
    mod_forbidden_chars: quake_rs::fs::COM_ModForbiddenChars,
    hash_string: quake_rs::loc::COM_HashString,
    hash_block: quake_rs::loc::COM_HashBlock,
    loc_init: quake_rs::loc::LOC_Init,
    loc_shutdown: quake_rs::loc::LOC_Shutdown,
    loc_get_raw_string: quake_rs::loc::LOC_GetRawString,
    loc_get_string: quake_rs::loc::LOC_GetString,
    loc_has_placeholders: rust_loc_has_placeholders,
    loc_format: quake_rs::loc::LOC_Format,
    steam_is_valid_path: quake_rs::steam::Steam_IsValidPath,
    steam_find_game: quake_rs::steam::Steam_FindGame,
    steam_resolve_path: quake_rs::steam::Steam_ResolvePath,
    egs_find_game: quake_rs::steam::EGS_FindGame,
};

/// quake_rs::loc::LOC_HasPlaceholders returns qboolean (= bool); the direct
/// item reference fails the fn() type match on some toolchains, so go through
/// a thin adapter.
unsafe extern "C" fn rust_loc_has_placeholders(s: *const c_char) -> bool {
    // SAFETY: forwarded contract
    unsafe { quake_rs::loc::LOC_HasPlaceholders(s) }
}

pub fn fns(side: Side) -> &'static SideFns {
    match side {
        Side::C => &C_FNS,
        Side::Rust => &RUST_FNS,
    }
}

// ---------------------------------------------------------------------------
// state control

fn write_cbuf(dst: *mut c_char, cap: usize, s: &[u8]) {
    let n = s.len().min(cap - 1);
    for (i, &b) in s[..n].iter().enumerate() {
        // SAFETY: i < cap - 1; dst points at a buffer of cap bytes
        unsafe { *dst.add(i) = b as c_char };
    }
    // SAFETY: n < cap
    unsafe { *dst.add(n) = 0 };
}

fn read_cbuf(src: *const c_char) -> String {
    // SAFETY: the engine buffers are NUL-terminated C strings
    unsafe { CStr::from_ptr(src) }
        .to_string_lossy()
        .into_owned()
}

/// Hard-resets one side's fs state: frees every searchpath (closing pak
/// handles), clears the base marker, the basedir list and the flag globals.
pub fn reset(side: Side) {
    let p = ptrs(side);
    let f = fns(side);
    // SAFETY: main-thread (FS_LOCK) access to the side's globals; the
    // teardown mirrors what the engine's "game" command does, with the base
    // marker cleared first so everything is freed
    unsafe {
        *p.base_searchpaths = ptr::null_mut();
        if *p.numbasedirs == 0 {
            // COM_ResetGameDirectories indexes com_basedirs[numbasedirs - 1]
            write_cbuf(p.basedirs, MAX_OSPATH, b".");
            *p.numbasedirs = 1;
        }
        (f.reset_game_directories)(c"".as_ptr());
        *p.modified = false;
        *p.standard_quake = true;
        *p.rogue = false;
        *p.hipnotic = false;
        *p.numbasedirs = 0;
        write_cbuf(p.basedir, MAX_OSPATH, b"");
        write_cbuf(p.gamedir, MAX_OSPATH, b"");
        write_cbuf(p.gamenames, 1024, b"");
    }
}

/// Resets one side and mounts `dirs` (a `;`-separated gamedir list, id1
/// filtered like the engine) over the given roots. `main_idx` names the root
/// that acts as com_basedir (the is_main / embedded-pak root).
pub fn setup(side: Side, roots: &[&std::path::Path], main_idx: usize, dirs: &CStr) {
    reset(side);
    let p = ptrs(side);
    let f = fns(side);
    for root in roots {
        let c = to_cstring(root.to_str().unwrap());
        // SAFETY: NUL-terminated path; side globals under FS_LOCK
        unsafe { (f.add_base_dir)(c.as_ptr()) };
    }
    let main = to_cstring(roots[main_idx].to_str().unwrap());
    // SAFETY: com_basedir is a MAX_OSPATH buffer; then the reset mounts dirs
    unsafe {
        write_cbuf(p.basedir, MAX_OSPATH, main.as_bytes());
        (f.reset_game_directories)(dirs.as_ptr());
    }
}

/// Marks the current searchpaths as the base (what COM_InitFilesystem does
/// after mounting the -basegame/id1 set).
pub fn mark_base(side: Side) {
    let p = ptrs(side);
    // SAFETY: side globals under FS_LOCK
    unsafe { *p.base_searchpaths = *p.searchpaths };
}

// ---------------------------------------------------------------------------
// snapshots

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackSnap {
    pub filename: String,
    pub numfiles: i32,
    pub files: Vec<(String, i32, i32)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpSnap {
    pub path_id: u32,
    pub filename: String,
    pub dir: String,
    pub pack: Option<PackSnap>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsSnapshot {
    pub modified: bool,
    pub standard_quake: bool,
    pub rogue: bool,
    pub hipnotic: bool,
    pub gamenames: String,
    pub gamedir: String,
    pub basedir: String,
    pub numbasedirs: i32,
    /// how many searchpaths sit above the base marker (head-first)
    pub above_base: usize,
    pub searchpaths: Vec<SpSnap>,
}

/// Reads one side's full observable fs state (handle numbers excluded: the
/// two sides allocate from the same stub handle table in different order).
pub fn snapshot(side: Side) -> FsSnapshot {
    let p = ptrs(side);
    // SAFETY: side globals under FS_LOCK; the searchpath/pack nodes are live
    // C-layout allocations owned by the side until the next reset
    unsafe {
        let mut searchpaths = Vec::new();
        let mut above_base = 0usize;
        let mut counting_above = true;
        let mut s = *p.searchpaths;
        while !s.is_null() {
            if s == *p.base_searchpaths {
                counting_above = false;
            }
            if counting_above {
                above_base += 1;
            }
            let pack = (*s).pack;
            let pack_snap = if pack.is_null() {
                None
            } else {
                let pk: &Pack = &*pack;
                let files = core::slice::from_raw_parts(pk.files, pk.numfiles.max(0) as usize);
                Some(PackSnap {
                    filename: read_cbuf(pk.filename.as_ptr()),
                    numfiles: pk.numfiles,
                    files: files
                        .iter()
                        .map(|pf| (read_cbuf(pf.name.as_ptr()), pf.filepos, pf.filelen))
                        .collect(),
                })
            };
            searchpaths.push(SpSnap {
                path_id: (*s).path_id,
                filename: read_cbuf((*s).filename.as_ptr()),
                dir: read_cbuf((*s).dir.as_ptr()),
                pack: pack_snap,
            });
            s = (*s).next;
        }
        if (*p.base_searchpaths).is_null() && counting_above {
            // no base marker: everything counts as above only if a marker
            // exists; report usize::MAX sentinel when the null marker was
            // never hit (searchpaths may be empty)
            above_base = searchpaths.len();
        }
        FsSnapshot {
            modified: *p.modified,
            standard_quake: *p.standard_quake,
            rogue: *p.rogue,
            hipnotic: *p.hipnotic,
            gamenames: read_cbuf(p.gamenames),
            gamedir: read_cbuf(p.gamedir),
            basedir: read_cbuf(p.basedir),
            numbasedirs: *p.numbasedirs,
            above_base,
            searchpaths,
        }
    }
}

/// The engine file handles of every mounted pak, head-first (side-specific
/// values: the two sides allocate from the shared stub table independently).
pub fn pak_handles(side: Side) -> Vec<i32> {
    let p = ptrs(side);
    // SAFETY: side globals under FS_LOCK; live nodes until the next reset
    unsafe {
        let mut out = Vec::new();
        let mut s = *p.searchpaths;
        while !s.is_null() {
            let pack = (*s).pack;
            if !pack.is_null() {
                out.push((*pack).handle);
            }
            s = (*s).next;
        }
        out
    }
}

/// Loads a file through one side's COM_LoadFile: returns (bytes-with-appended
/// NUL excluded, com_filesize, file_from_pak, path_id) or None on miss.
pub fn load_file(side: Side, name: &CStr) -> Option<(Vec<u8>, i64, i32, u32)> {
    let f = fns(side);
    let mut path_id: c_uint = 0;
    // SAFETY: NUL-terminated name and valid out-param; the returned buffer is
    // Mem-owned with com_filesize bytes + NUL and freed below
    unsafe {
        let buf = (f.load_file)(name.as_ptr(), &mut path_id);
        if buf.is_null() {
            return None;
        }
        let size = thread_file_size();
        let bytes = core::slice::from_raw_parts(buf, size.max(0) as usize).to_vec();
        quake_c_sys::Mem_Free(buf.cast());
        Some((bytes, size, thread_file_from_pak(), path_id))
    }
}

/// Runs one named fatal scenario in a **child process** and returns the
/// message it died with (the text after `Sys_Error: `), or None if the child
/// exited without fataling.
///
/// The in-process trap ([`catch_sys_error`]) longjmps, which is legal across
/// the c_ref side's pure C frames but is UB across the Rust shim's — see the
/// note there. So Rust-side fatality is probed out-of-process: the child runs
/// with the trap unarmed, so the stub's `Sys_Error` prints
/// `Sys_Error: <msg>` to stderr and aborts, and this reads it back.
///
/// `test_fn_name` is the `#[test]` in the calling integration test that
/// dispatches on `CTEST_FATAL_CASE` (by convention `rust_fatal_child`).
pub fn rust_fatal_in_child(test_fn_name: &str, case: &str, env: &[(&str, &str)]) -> Option<String> {
    let exe = std::env::current_exe().expect("current_exe");
    let mut cmd = std::process::Command::new(exe);
    cmd.args([test_fn_name, "--exact", "--nocapture", "--test-threads=1"])
        .env("CTEST_FATAL_CASE", case);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn fatal-probe child");

    // the child aborts on purpose, so its exit status is expected to be
    // non-zero; only the captured message matters
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let marker = "Sys_Error: ";
    let at = stderr.rfind(marker)?;

    // Take the remainder verbatim rather than splitting on lines: the engine's
    // messages can themselves end in (or contain) a newline, and the C side's
    // trapped copy keeps it, so the comparison has to. The stub's fprintf
    // appends exactly one newline of its own, which is the only one to drop.
    let mut msg = stderr[at + marker.len()..].to_owned();
    if msg.ends_with('\n') {
        msg.pop();
    }
    Some(msg)
}

/// True when this process IS the child spawned by [`rust_fatal_in_child`]
/// for `case`; the dispatch tests return early otherwise.
pub fn fatal_child_case() -> Option<String> {
    std::env::var("CTEST_FATAL_CASE").ok()
}
