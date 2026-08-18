//! Miri coverage for the quake-capi FFI shims.
//!
//! The differential suites prove the shims *agree with the C*; they cannot
//! prove the shims are free of undefined behaviour, because they run natively.
//! This file closes that gap: it supplies the engine symbols as Rust
//! `#[no_mangle]` stubs, which Miri resolves through quake-c-sys's `extern "C"`
//! declarations, so the **real** shims execute under Miri's aliasing model
//! rather than a reimplementation that could drift from them.
//!
//! Scope is deliberately narrow. These stubs are not a second reference
//! implementation — output parity is the differential suites' job. They only
//! have to be a *valid* engine so the shims run and Miri can watch the pointer
//! work: the `gfx/` retry memmove in `W_LoadWadList`, the in-place lump
//! byteswaps against a loaded image, the JSON arena's parent/child links, and
//! the cvar-name borrows in `CFG_ReadCvars`.
//!
//! Run with:
//!   cargo +nightly miri test -p quake-ctest --test miri_capi

#![cfg(miri)]
// c_variadic became stable during nightly 1.100; keep the gate so older
// nightlies still build this file, and silence the "already stable" warning
#![allow(stable_features)]
#![feature(c_variadic)]

use core::ffi::{c_char, c_int, c_uint, c_void, CStr};
use quake_c_sys::{fshandle_t, qfileofs_t, qfilesize_t, FILE};
use std::sync::Mutex;

// ---------------------------------------------------------------------------
// Allocator. Mem_Free only gets the user pointer, so the layout is stashed in
// a 16-byte header (16 also keeps the returned pointer over-aligned for every
// struct the shims build in engine memory).
// ---------------------------------------------------------------------------

const HDR: usize = 16;

fn layout(total: usize) -> std::alloc::Layout {
    std::alloc::Layout::from_size_align(total, HDR).unwrap()
}

#[no_mangle]
pub extern "C" fn Mem_Alloc(size: usize) -> *mut c_void {
    let total = size.max(1) + HDR;
    // SAFETY: non-zero size, valid layout
    let p = unsafe { std::alloc::alloc_zeroed(layout(total)) };
    assert!(!p.is_null(), "miri stub OOM");
    // SAFETY: the header fits: total >= HDR
    unsafe { (p as *mut usize).write(total) };
    // SAFETY: still inside the same allocation
    unsafe { p.add(HDR) as *mut c_void }
}

#[no_mangle]
pub extern "C" fn Mem_AllocNonZero(size: usize) -> *mut c_void {
    Mem_Alloc(size)
}

#[no_mangle]
pub extern "C" fn Mem_Free(ptr: *const c_void) {
    if ptr.is_null() {
        return;
    }
    // SAFETY: ptr came from Mem_Alloc, so a header sits HDR bytes below it
    let base = unsafe { (ptr as *const u8).sub(HDR) } as *mut u8;
    // SAFETY: as above
    let total = unsafe { (base as *const usize).read() };
    // SAFETY: same base/layout pair the allocation used
    unsafe { std::alloc::dealloc(base, layout(total)) };
}

// ---------------------------------------------------------------------------
// Console / fatal. Output is not under test; these only have to be callable,
// and Sys_Error has to diverge like the C's noreturn.
// ---------------------------------------------------------------------------

#[no_mangle]
pub unsafe extern "C" fn Con_Printf(_fmt: *const c_char, _args: ...) {}
#[no_mangle]
pub unsafe extern "C" fn Con_Warning(_fmt: *const c_char, _args: ...) {}
#[no_mangle]
pub unsafe extern "C" fn Con_DPrintf(_fmt: *const c_char, _args: ...) {}
#[no_mangle]
pub unsafe extern "C" fn Con_DPrintf2(_fmt: *const c_char, _args: ...) {}

#[no_mangle]
pub unsafe extern "C" fn Sys_Error(_error: *const c_char, _args: ...) -> ! {
    panic!("Sys_Error reached under Miri");
}

// ---------------------------------------------------------------------------
// In-memory filesystem. `FILE*` is opaque in the bindings, so an open file is
// a Box<MemFile> cast to *mut FILE; fshandle_t's start/pos/length then index
// into it exactly as the engine's do.
// ---------------------------------------------------------------------------

static FILES: Mutex<Vec<(Vec<u8>, Vec<u8>)>> = Mutex::new(Vec::new());
static THREAD_FILE_SIZE: Mutex<i64> = Mutex::new(0);
static THREAD_FROM_PAK: Mutex<bool> = Mutex::new(false);
static CVAR_LOG: Mutex<Vec<(Vec<u8>, Vec<u8>)>> = Mutex::new(Vec::new());

struct MemFile {
    data: Vec<u8>,
}

fn add_file(name: &str, data: Vec<u8>) {
    FILES.lock().unwrap().push((name.as_bytes().to_vec(), data));
}

fn reset_fs() {
    FILES.lock().unwrap().clear();
    CVAR_LOG.lock().unwrap().clear();
}

/// # Safety
/// `name` must be NUL-terminated.
unsafe fn lookup(name: *const c_char) -> Option<Vec<u8>> {
    // SAFETY: caller contract
    let want = unsafe { CStr::from_ptr(name) }.to_bytes().to_vec();
    FILES
        .lock()
        .unwrap()
        .iter()
        .find(|(n, _)| *n == want)
        .map(|(_, d)| d.clone())
}

fn open_memfile(data: Vec<u8>) -> *mut FILE {
    Box::into_raw(Box::new(MemFile { data })) as *mut FILE
}

/// # Safety
/// `f` must be a handle from `open_memfile` that has not been closed.
unsafe fn memfile<'a>(f: *mut FILE) -> &'a MemFile {
    // SAFETY: caller contract
    unsafe { &*(f as *mut MemFile) }
}

#[no_mangle]
pub unsafe extern "C" fn COM_LoadFile(path: *const c_char, _path_id: *mut c_uint) -> *mut u8 {
    // SAFETY: engine contract — NUL-terminated path
    let Some(data) = (unsafe { lookup(path) }) else {
        return core::ptr::null_mut();
    };
    *THREAD_FILE_SIZE.lock().unwrap() = data.len() as i64;
    *THREAD_FROM_PAK.lock().unwrap() = false;
    // COM_LoadFile allocates len + 1 and NUL-terminates; no header padding
    let buf = Mem_Alloc(data.len() + 1) as *mut u8;
    // SAFETY: buf is len + 1 bytes and data is len bytes
    unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), buf, data.len()) };
    buf
}

#[no_mangle]
pub extern "C" fn COM_ThreadFileSize() -> qfileofs_t {
    *THREAD_FILE_SIZE.lock().unwrap()
}

#[no_mangle]
pub extern "C" fn COM_ThreadFileFromPak() -> c_int {
    i32::from(*THREAD_FROM_PAK.lock().unwrap())
}

#[no_mangle]
pub unsafe extern "C" fn COM_FOpenFile(
    filename: *const c_char,
    file: *mut *mut FILE,
    _path_id: *mut c_uint,
) -> qfilesize_t {
    // SAFETY: engine contract — NUL-terminated name
    let Some(data) = (unsafe { lookup(filename) }) else {
        return -1;
    };
    let len = data.len() as qfilesize_t;
    *THREAD_FILE_SIZE.lock().unwrap() = len;
    *THREAD_FROM_PAK.lock().unwrap() = false;
    // SAFETY: engine contract — `file` is a writable out-parameter
    unsafe { *file = open_memfile(data) };
    len
}

#[no_mangle]
pub unsafe extern "C" fn COM_FOpenPrefFile(
    filename: *const c_char,
    _mode: *const c_char,
) -> *mut FILE {
    // SAFETY: engine contract — NUL-terminated name
    match unsafe { lookup(filename) } {
        Some(data) => open_memfile(data),
        None => core::ptr::null_mut(),
    }
}

#[no_mangle]
pub unsafe extern "C" fn Sys_ftell(_file: *mut FILE) -> qfileofs_t {
    0
}

#[no_mangle]
pub unsafe extern "C" fn Sys_filelength(f: *mut FILE) -> qfilesize_t {
    // SAFETY: engine contract — an open handle
    unsafe { memfile(f) }.data.len() as qfilesize_t
}

#[no_mangle]
pub unsafe extern "C" fn FS_fread(
    ptr: *mut c_void,
    size: usize,
    nmemb: usize,
    fh: *mut fshandle_t,
) -> usize {
    if size == 0 {
        return 0;
    }
    // SAFETY: engine contract — `fh` is an open handle
    let (file, start, pos, length) = unsafe { ((*fh).file, (*fh).start, (*fh).pos, (*fh).length) };
    // SAFETY: as above
    let data = &unsafe { memfile(file) }.data;

    let remaining = (length - pos).max(0) as usize;
    let from = (start + pos) as usize;
    let n = (size * nmemb)
        .min(remaining)
        .min(data.len().saturating_sub(from));
    // SAFETY: engine contract — `ptr` is writable for `size * nmemb` bytes
    unsafe { core::ptr::copy_nonoverlapping(data.as_ptr().add(from), ptr as *mut u8, n) };
    // SAFETY: as above
    unsafe { (*fh).pos = pos + n as qfileofs_t };
    n / size
}

#[no_mangle]
pub unsafe extern "C" fn FS_fseek(fh: *mut fshandle_t, offset: qfileofs_t, whence: c_int) -> c_int {
    // SAFETY: engine contract — `fh` is an open handle
    let (pos, length) = unsafe { ((*fh).pos, (*fh).length) };
    let base = match whence {
        1 => pos,    // SEEK_CUR
        2 => length, // SEEK_END
        _ => 0,      // SEEK_SET
    };
    // SAFETY: as above
    unsafe { (*fh).pos = (base + offset).clamp(0, length) };
    0
}

#[no_mangle]
pub unsafe extern "C" fn FS_rewind(fh: *mut fshandle_t) {
    // SAFETY: engine contract — `fh` is an open handle
    unsafe { (*fh).pos = 0 };
}

#[no_mangle]
pub unsafe extern "C" fn FS_feof(fh: *mut fshandle_t) -> c_int {
    // SAFETY: engine contract — `fh` is an open handle
    let (pos, length) = unsafe { ((*fh).pos, (*fh).length) };
    i32::from(pos >= length)
}

#[no_mangle]
pub unsafe extern "C" fn FS_ferror(_fh: *mut fshandle_t) -> c_int {
    0
}

#[no_mangle]
pub unsafe extern "C" fn FS_fclose(fh: *mut fshandle_t) -> c_int {
    // SAFETY: engine contract — `fh` is an open handle
    let file = unsafe { (*fh).file };
    if !file.is_null() {
        // SAFETY: the pointer came from open_memfile's Box::into_raw
        drop(unsafe { Box::from_raw(file as *mut MemFile) });
        // SAFETY: `fh` is writable
        unsafe { (*fh).file = core::ptr::null_mut() };
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn FS_fgets(s: *mut c_char, size: c_int, fh: *mut fshandle_t) -> *mut c_char {
    if size <= 1 {
        return core::ptr::null_mut();
    }
    // SAFETY: engine contract — `fh` is an open handle
    let (file, start, pos, length) = unsafe { ((*fh).file, (*fh).start, (*fh).pos, (*fh).length) };
    if pos >= length {
        return core::ptr::null_mut();
    }
    // SAFETY: as above
    let data = &unsafe { memfile(file) }.data;

    let mut n = 0usize;
    let mut p = pos;
    while p < length && n < (size as usize) - 1 {
        let idx = (start + p) as usize;
        if idx >= data.len() {
            break;
        }
        let b = data[idx];
        // SAFETY: engine contract — `s` is writable for `size` bytes
        unsafe { *(s as *mut u8).add(n) = b };
        n += 1;
        p += 1;
        if b == b'\n' {
            break;
        }
    }
    // SAFETY: n < size, so the terminator is in bounds
    unsafe { *(s as *mut u8).add(n) = 0 };
    // SAFETY: `fh` is writable
    unsafe { (*fh).pos = p };
    if n == 0 {
        core::ptr::null_mut()
    } else {
        s
    }
}

// ---------------------------------------------------------------------------
// Path helpers and command line.
// ---------------------------------------------------------------------------

/// # Safety
/// `out` must be writable for `outsize` bytes.
unsafe fn write_cstr(out: *mut c_char, outsize: usize, bytes: &[u8]) {
    if outsize == 0 {
        return;
    }
    let n = bytes.len().min(outsize - 1);
    // SAFETY: caller contract; n < outsize
    unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), out as *mut u8, n) };
    // SAFETY: as above
    unsafe { *(out as *mut u8).add(n) = 0 };
}

#[no_mangle]
pub unsafe extern "C" fn COM_FileBase(in_: *const c_char, out: *mut c_char, outsize: usize) {
    // SAFETY: engine contract — NUL-terminated input
    let s = unsafe { CStr::from_ptr(in_) }.to_bytes();
    let after_sep = match s.iter().rposition(|&b| b == b'/' || b == b'\\') {
        Some(i) => &s[i + 1..],
        None => s,
    };
    let base = match after_sep.iter().rposition(|&b| b == b'.') {
        Some(i) => &after_sep[..i],
        None => after_sep,
    };
    let base: &[u8] = if base.is_empty() { b"?model?" } else { base };
    // SAFETY: engine contract — `out` is writable for `outsize` bytes
    unsafe { write_cstr(out, outsize, base) };
}

#[no_mangle]
pub unsafe extern "C" fn COM_AddExtension(path: *mut c_char, extension: *const c_char, len: usize) {
    // SAFETY: engine contract — both NUL-terminated
    let (cur, ext) = unsafe {
        (
            CStr::from_ptr(path).to_bytes().to_vec(),
            CStr::from_ptr(extension).to_bytes().to_vec(),
        )
    };
    if cur.ends_with(&ext) || cur.len() + ext.len() + 1 > len {
        return;
    }
    let mut joined = cur;
    joined.extend_from_slice(&ext);
    // SAFETY: engine contract — `path` is writable for `len` bytes, and the
    // result was just checked to fit
    unsafe { write_cstr(path, len, &joined) };
}

#[no_mangle]
pub static mut com_argc: c_int = 0;
#[no_mangle]
pub static mut com_argv: *mut *mut c_char = core::ptr::null_mut();

#[no_mangle]
pub unsafe extern "C" fn COM_CheckParm(parm: *const c_char) -> c_int {
    // SAFETY: engine contract — NUL-terminated
    let want = unsafe { CStr::from_ptr(parm) }.to_bytes();
    // SAFETY: the test sets these before the call and never mutates them
    // concurrently
    let (argc, argv) = unsafe { (com_argc, com_argv) };
    for i in 1..argc {
        // SAFETY: i < argc, and every entry is a NUL-terminated string
        let a = unsafe { *argv.add(i as usize) };
        if a.is_null() {
            continue;
        }
        // SAFETY: as above
        if unsafe { CStr::from_ptr(a) }.to_bytes() == want {
            return i;
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn Cvar_Set(var_name: *const c_char, value: *const c_char) {
    // SAFETY: engine contract — both NUL-terminated
    let (n, v) = unsafe {
        (
            CStr::from_ptr(var_name).to_bytes().to_vec(),
            CStr::from_ptr(value).to_bytes().to_vec(),
        )
    };
    CVAR_LOG.lock().unwrap().push((n, v));
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn le32(v: i32) -> [u8; 4] {
    v.to_le_bytes()
}

/// A WAD2 image: header, lump data, then the directory.
fn build_wad(lumps: &[(&str, u8, Vec<u8>)]) -> Vec<u8> {
    let mut data = Vec::new();
    let mut dir = Vec::new();
    let mut ofs = 12i32;
    for (name, type_, bytes) in lumps {
        dir.push((name.to_string(), *type_, ofs, bytes.len() as i32));
        ofs += bytes.len() as i32;
        data.extend_from_slice(bytes);
    }
    let infotableofs = 12 + data.len() as i32;

    let mut out = Vec::new();
    out.extend_from_slice(b"WAD2");
    out.extend_from_slice(&le32(lumps.len() as i32));
    out.extend_from_slice(&le32(infotableofs));
    out.extend_from_slice(&data);
    for (name, type_, filepos, size) in dir {
        out.extend_from_slice(&le32(filepos));
        out.extend_from_slice(&le32(size)); // disksize
        out.extend_from_slice(&le32(size));
        out.push(type_);
        out.push(0); // compression
        out.push(0);
        out.push(0);
        let mut n = [0u8; 16];
        let b = name.as_bytes();
        let k = b.len().min(16);
        n[..k].copy_from_slice(&b[..k]);
        out.extend_from_slice(&n);
    }
    out
}

/// A qpic lump: width, height, then pixels.
fn qpic(w: i32, h: i32) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&le32(w));
    v.extend_from_slice(&le32(h));
    v.extend(std::iter::repeat_n(7u8, (w * h) as usize));
    v
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn wad_and_cfgfile_shims_are_alias_clean() {
    reset_fs();

    // --- W_LoadWadFile: byteswaps and SwapPic against the loaded image ---
    add_file(
        "gfx.wad",
        build_wad(&[
            ("conchars", 66 /* TYP_QPIC */, qpic(4, 4)),
            ("palette", 64, vec![1, 2, 3, 4]),
        ]),
    );
    // SAFETY: single-threaded here; the stubs supply every engine symbol
    unsafe { quake_rs::wad::W_LoadWadFile() };
    // SAFETY: set by W_LoadWadFile
    assert_eq!(
        unsafe { quake_rs::wad::wad_numlumps },
        2,
        "both lumps survived the header checks"
    );

    // SAFETY: the lump exists in the image just loaded
    let found = unsafe {
        let mut info = core::ptr::null_mut();
        quake_rs::wad::W_GetLumpName(c"conchars".as_ptr(), &mut info)
    };
    assert!(!found.is_null());

    // a corrupt directory must take the repair path without tripping Miri
    let mut corrupt = build_wad(&[("bad", 64, vec![0; 8])]);
    let dir = corrupt.len() - 32;
    corrupt[dir..dir + 4].copy_from_slice(&le32(0x0100_0000)); // filepos past EOF
    reset_fs();
    add_file("gfx.wad", corrupt);
    // SAFETY: as above
    unsafe { quake_rs::wad::W_LoadWadFile() };

    // --- W_LoadWadList: exercises the `gfx/` retry memmove ---
    reset_fs();
    // only present under gfx/, so the first open fails and the retry runs
    add_file("gfx/retry.wad", build_wad(&[("lump", 64, vec![9; 4])]));
    add_file("plain.wad", build_wad(&[("other", 64, vec![8; 4])]));
    // SAFETY: NUL-terminated list per the wad.h contract
    let wads = unsafe { quake_rs::wad::W_LoadWadList(c"plain;retry".as_ptr()) };
    assert!(!wads.is_null(), "at least one wad opened");

    // SAFETY: `wads` is the list just built
    let info = unsafe {
        let mut owner = core::ptr::null_mut();
        quake_rs::wad::W_GetLumpinfoList(wads, c"lump".as_ptr(), &mut owner)
    };
    assert!(!info.is_null(), "found the lump behind the gfx/ retry");

    // SAFETY: the list came from W_LoadWadList
    unsafe { quake_rs::wad::W_FreeWadList(wads) };

    // --- W_CleanupName in place (the read must complete before the write) ---
    let mut name = *b"PROGS/E1M1.BSP\0\0";
    // SAFETY: 16 readable and writable bytes, aliased on purpose
    unsafe {
        let p = name.as_mut_ptr() as *mut c_char;
        quake_rs::wad::W_CleanupName(p, p);
    }

    // --- cfgfile: the read loop's borrows of the cvar-name array ---
    reset_fs();
    add_file(
        "vkQuake.cfg",
        b"vid_width \"1280\"\nvid_height \"720\"\n".to_vec(),
    );
    // SAFETY: NUL-terminated name; single-threaded boot path
    assert_eq!(
        unsafe { quake_rs::cfgfile::CFG_OpenConfig(c"vkQuake.cfg".as_ptr()) },
        0
    );

    let mut vars: [*const c_char; 2] = [c"vid_width".as_ptr(), c"vid_height".as_ptr()];
    // SAFETY: two readable, NUL-terminated entries
    unsafe { quake_rs::cfgfile::CFG_ReadCvars(vars.as_mut_ptr(), 2) };
    assert_eq!(CVAR_LOG.lock().unwrap().len(), 2, "both cvars were set");

    // SAFETY: single-threaded boot path
    unsafe { quake_rs::cfgfile::CFG_CloseConfig() };

    // --- CFG_ReadCvarOverrides: com_argv reads plus the '+' buffer build ---
    let mut a0 = *b"vkquake\0";
    let mut a1 = *b"+vid_width\0";
    let mut a2 = *b"1920\0";
    let mut argv: [*mut c_char; 3] = [
        a0.as_mut_ptr() as *mut c_char,
        a1.as_mut_ptr() as *mut c_char,
        a2.as_mut_ptr() as *mut c_char,
    ];
    // SAFETY: the globals are only written here, before the call below
    unsafe {
        com_argc = 3;
        com_argv = argv.as_mut_ptr();
    }
    CVAR_LOG.lock().unwrap().clear();
    // SAFETY: one readable, NUL-terminated entry
    unsafe { quake_rs::cfgfile::CFG_ReadCvarOverrides(vars.as_mut_ptr(), 1) };
    assert_eq!(
        CVAR_LOG.lock().unwrap().len(),
        1,
        "the +vid_width override was applied"
    );
}

#[test]
fn json_arena_is_alias_clean() {
    // No numbers: JSON number parsing calls the platform strtod, which Miri
    // cannot execute (the same reason quake_util::json's test is miri-ignored).
    let text = c"{\"a\": \"x\", \"b\": [true, null, {\"c\": \"y\"}], \"d\": {}}";
    // SAFETY: NUL-terminated per the json.h contract
    let json = unsafe { quake_rs::json::JSON_Parse(text.as_ptr()) };
    assert!(!json.is_null());

    // SAFETY: `json` came from JSON_Parse
    let root = unsafe { (*json).root };
    // SAFETY: `root` is a node in that arena
    let found = unsafe { quake_rs::json::JSON_FindString(root, c"a".as_ptr()) };
    assert!(!found.is_null());
    // SAFETY: JSON_FindString returned a pointer into the arena's string block
    assert_eq!(unsafe { CStr::from_ptr(found) }.to_bytes(), b"x");

    // walks the whole child list, including non-string children
    // SAFETY: as above
    let missing = unsafe { quake_rs::json::JSON_FindString(root, c"nope".as_ptr()) };
    assert!(missing.is_null());

    // SAFETY: the block came from JSON_Parse
    unsafe { quake_rs::json::JSON_Free(json) };

    // NULL and malformed inputs must not touch memory
    // SAFETY: NULL is explicitly allowed
    assert!(unsafe { quake_rs::json::JSON_Parse(core::ptr::null()) }.is_null());
    // SAFETY: NUL-terminated
    assert!(unsafe { quake_rs::json::JSON_Parse(c"{".as_ptr()) }.is_null());
}
