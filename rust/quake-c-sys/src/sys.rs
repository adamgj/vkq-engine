//! `Quake/sys_glue.c` declarations plus the C callees `quake-platform::sys`
//! needs that the committed bindings do not carry (Rust migration Phase 9
//! M5, ADR-011).
//!
//! `sys_sdl.c`/`sys_sdl_win.c`/`sys_sdl_unix.c` leave the build under
//! `-Duse_rust_platform`; `Quake/sys_glue.c` keeps the two variadic entry
//! points (`Sys_Error`/`Sys_Printf` format in C and hand the text to the
//! Rust cores, task plan I5), the `Host_Guard` trampoline below around
//! `Host_Shutdown` (a `Host_Reraise` wrapper under `-Duse_rust_host`,
//! ADR-009 rule 3) and the MSVC comctl32 manifest pragma.
//!
//! The C runtime entries are the ones the handle table and the console use
//! through the engine's own `FILE *`: `Sys_DuplicateHandle` reopens paths,
//! so the I/O has to stay on the C stdio the rest of the engine shares
//! (task plan D6).

use crate::{qboolean, steamgame_t, FILE};
use core::ffi::{c_char, c_int, c_void};

extern "C" {
    /* Guarded callback (ADR-009 rule 3) -- Quake/sys_glue.c. */
    /// `Host_Shutdown ()` (`Sys_Quit`, and `Sys_Error` on unix).
    pub fn SysGlue_HostShutdown() -> c_int;
    /// `Sys_Init ()` (Phase 9 M6 -- the `main_sdl.c` startup sequence).
    pub fn SysGlue_SysInit() -> c_int;
    /// `Host_Init ()` (M6).
    pub fn SysGlue_HostInit() -> c_int;
    /// `Host_Frame (time)` (M6): the outermost frame guard, what
    /// `Host_Glue_FrameInner`'s `setjmp` used to provide.
    pub fn SysGlue_HostFrame(time: f64) -> c_int;

    /* M6 -- `main_sdl.c` callees that cannot raise. */
    /// The `#if __clang__ / __GNUC__ / _MSC_VER` "Built with ..." banner;
    /// stays in C so it reports the compiler that built the C remnant.
    pub fn SysGlue_PrintCompilerBanner();
    /// `ENGINE_NAME_AND_VER` from `quakedef.h`.
    pub fn SysGlue_EngineNameAndVer() -> *const c_char;
    /// `cls.timedemo`.
    pub fn SysGlue_ClientTimedemo() -> qboolean;
    /// `cl.paused`.
    pub fn SysGlue_ClientPaused() -> qboolean;
    /// `common.h:296`.
    pub fn COM_InitArgv(argc: c_int, argv: *mut *mut c_char);
    /// `harness.h:54` -- early, right after `COM_InitArgv`.
    pub fn Harness_CheckArgs();
    /// `harness.h:102`.
    pub fn Harness_FrameTime() -> f64;
    /// `harness.h:49`.
    pub static mut harness_fixed_dt: qboolean;
    /// `vid.h:89`.
    pub fn VID_HasMouseOrInputFocus() -> qboolean;
    /// `vid.h:90`.
    pub fn VID_IsMinimized() -> qboolean;

    /* Direct callees that cannot raise. */
    /// `steam.h:47` -- resolves `game` to its install directory.
    pub fn Steam_ResolvePath(
        path: *mut c_char,
        pathsize: usize,
        game: *const steamgame_t,
    ) -> qboolean;
    /// `steam.h:45`.
    pub fn Steam_IsValidPath(path: *const c_char) -> qboolean;
    /// `pr_edict.c` -- `PR_SwitchQCVM (NULL)` before the error text goes out.
    pub fn PR_SwitchQCVM(nvm: *mut c_void);
    /// `mem.h`.
    pub fn Mem_Alloc(size: usize) -> *mut c_void;
    /// `mem.h`.
    pub fn Mem_Free(ptr: *const c_void);
    /// `common.h`.
    pub fn q_strlcpy(dst: *mut c_char, src: *const c_char, size: usize) -> usize;
    /// `common.h`.
    pub fn q_strlcat(dst: *mut c_char, src: *const c_char, size: usize) -> usize;
    /// `common.h`.
    pub fn q_strcasecmp(s1: *const c_char, s2: *const c_char) -> c_int;
    /// `common.h`.
    pub fn q_strdup(str_: *const c_char) -> *mut c_char;
    /// `common.h` -- the extension after the last `.` of the file name.
    pub fn COM_FileGetExtension(in_: *const c_char) -> *const c_char;
    /// `console.h`.
    pub fn Con_SafePrintf(fmt: *const c_char, ...);
    /// `tasks.h`.
    pub fn Tasks_IsWorker() -> bool;
    /// `q_thread.h`.
    pub fn QMutex_Create() -> *mut crate::qmutex_t;
    /// `q_thread.h`.
    pub fn QMutex_Lock(mutex: *mut crate::qmutex_t);
    /// `q_thread.h`.
    pub fn QMutex_Unlock(mutex: *mut crate::qmutex_t);

    /* C runtime: the process exit that runs `atexit` handlers and flushes
     * stdio (Rust's `process::exit` does neither), and the stdio the
     * handle table shares with `FS_f*`. */
    pub fn exit(status: c_int) -> !;
    pub fn atexit(func: Option<unsafe extern "C" fn()>) -> c_int;
    pub fn fputs(s: *const c_char, stream: *mut FILE) -> c_int;
    pub fn fread(ptr: *mut c_void, size: usize, nmemb: usize, stream: *mut FILE) -> usize;
    pub fn fwrite(ptr: *const c_void, size: usize, nmemb: usize, stream: *mut FILE) -> usize;
    pub fn fclose(stream: *mut FILE) -> c_int;
    pub fn feof(stream: *mut FILE) -> c_int;
    pub fn strerror(errnum: c_int) -> *mut c_char;
}

// MSVC CRT spellings of the 64-bit seek/tell and the wide-path `fopen`
// `sys_sdl_win.c` used (`_fseeki64`/`_ftelli64`/`_wfopen`).
#[cfg(windows)]
extern "C" {
    pub fn _fseeki64(stream: *mut FILE, offset: i64, origin: c_int) -> c_int;
    pub fn _ftelli64(stream: *mut FILE) -> i64;
    pub fn _wfopen(filename: *const u16, mode: *const u16) -> *mut FILE;
    pub fn _errno() -> *mut c_int;
}

// `fseeko`/`ftello` under `-D_FILE_OFFSET_BITS=64` (`sys_sdl_unix.c`).
#[cfg(unix)]
extern "C" {
    pub fn fseeko(stream: *mut FILE, offset: i64, whence: c_int) -> c_int;
    pub fn ftello(stream: *mut FILE) -> i64;
    pub fn fopen(filename: *const c_char, mode: *const c_char) -> *mut FILE;
}

// `stdout` is a macro on every C runtime the engine links: `__acrt_iob_func (1)`
// on the MSVC UCRT (its `printf` is header-inline, so only `fputs` resolves
// from a Rust object), `__stdoutp` on Apple, a plain extern on glibc/musl.
#[cfg(windows)]
extern "C" {
    pub fn __acrt_iob_func(ix: core::ffi::c_uint) -> *mut FILE;
}
#[cfg(target_vendor = "apple")]
extern "C" {
    pub static mut __stdoutp: *mut FILE;
}
#[cfg(all(unix, not(target_vendor = "apple")))]
extern "C" {
    pub static mut stdout: *mut FILE;
}

/// The C `stdout` stream, for the `fputs` calls `sys_sdl_win.c` /
/// `sys_sdl_unix.c` made.
pub fn stdout_stream() -> *mut FILE {
    // SAFETY: the CRT owns the stream pointer; it is read by value, never
    // borrowed.
    unsafe {
        #[cfg(windows)]
        {
            __acrt_iob_func(1)
        }
        #[cfg(target_vendor = "apple")]
        {
            __stdoutp
        }
        #[cfg(all(unix, not(target_vendor = "apple")))]
        {
            stdout
        }
    }
}
