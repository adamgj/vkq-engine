//! `Quake/sys_sdl.c` / `Quake/sys_sdl_win.c` / `Quake/sys_sdl_unix.c`
//! (Rust migration Phase 9 M5, ADR-017): the `sys.h` system layer.
//!
//! Layout follows the C split: [`handles`] is the file-handle table and
//! [`common`] the SDL-backed helpers both platforms share (`sys_sdl.c`);
//! `imp` is `sys/win.rs` on Windows and `sys/unix.rs` everywhere else
//! (`sys_sdl_win.c` / `sys_sdl_unix.c`, macOS taking the `PLATFORM_OSX`
//! branches of the latter). File I/O stays on the C runtime's `FILE *`
//! (task plan D6) so the `FS_f*` shims and `Sys_DuplicateHandle` reopen
//! share one stdio layer with the rest of the engine, and process exit goes
//! through C `exit` so `Sys_AtExit` handlers and stdio flushing run as
//! before.
//!
//! Raising (ADR-009): `Sys_Error` on Unix and `Sys_Quit` on both platforms
//! run `Host_Shutdown`, which can raise. Those two and `Sys_SendKeyEvents`
//! (`IN_Commands` / `IN_SendKeyEvents`) are exported as status-returning
//! cores that `Quake/sys_glue.c` wraps in `Host_Reraise`; the variadic
//! `Sys_Error` / `Sys_Printf` are formatted in that C file (I5) and reach
//! [`error_core`] / [`print_text`] as finished text. Every other `sys.h`
//! entry cannot raise. Internal error paths call the C `Sys_Error` symbol
//! so they take the same wrapper as the rest of the engine.

use core::ffi::{c_char, c_int, CStr};
use std::ffi::CString;

use quake_c_sys::sys as c;

pub mod handles;

mod common;

#[cfg(windows)]
#[path = "sys/win.rs"]
mod imp;
#[cfg(not(windows))]
#[path = "sys/unix.rs"]
mod imp;

#[cfg(feature = "sdl3")]
pub use common::select_folder;
pub use common::{
    choose_quake_flavor, double_time, get_pref_path, message_box_warning, quit_no_shutdown, sleep,
};
pub use imp::{
    console_input, debug_break, error_core, explore, file_type, find_close, find_first, find_next,
    fopen, fseek, ftell, get_egs_launcher_data, get_egs_manifest_dir, get_gog_quake_dir,
    get_gog_quake_enhanced_dir, get_nightdive_user_dir, get_steam_api_library_path, get_steam_dir,
    init, is_in_debugger, mkdir, pin_current_thread, print_text, quit_core, stack_trace,
};

use imp::errno;

/// `static const char errortxt1[] = "\nERROR-OUT BEGIN\n\n";`
const ERRORTXT1: &CStr = c"\nERROR-OUT BEGIN\n\n";
/// `static const char errortxt2[] = "\nQUAKE ERROR: ";`
const ERRORTXT2: &CStr = c"\nQUAKE ERROR: ";
/// `#define MAX_STACK_FRAMES 24` (both `Sys_StackTrace` arms).
const MAX_STACK_FRAMES: usize = 24;

/// C: `void Sys_SendKeyEvents (void)` -- the status-returning core.
///
/// # Safety
/// Main thread, after `IN_Init`.
pub unsafe fn send_key_events() -> c_int {
    // SAFETY: caller contract.
    unsafe {
        // ericw -- allow joysticks to add keys so they can be used to confirm SCR_ModalMessage
        let r = crate::input::commands();
        if r != 0 {
            return r;
        }
        crate::input::send_key_events()
    }
}

/// `Sys_Error ("%s", msg)` through the engine symbol, so the C wrapper's
/// `Host_Reraise` and `exit (1)` apply exactly as for every other caller.
pub(crate) fn sys_error(msg: &str) -> ! {
    let msg = CString::new(msg.replace('\0', " ")).unwrap_or_default();
    // SAFETY: both strings are NUL-terminated for the duration of the call.
    unsafe { quake_c_sys::Sys_Error(c"%s".as_ptr(), msg.as_ptr()) }
}

/// `Sys_Printf ("%s", msg)` -- already-formatted text straight to the
/// platform core (what the C variadic wrapper does after formatting).
pub(crate) fn sys_printf(msg: &str) {
    let msg = CString::new(msg.replace('\0', " ")).unwrap_or_default();
    // SAFETY: NUL-terminated for the duration of the call.
    unsafe { print_text(msg.as_ptr()) }
}

/// `(size_t)q_snprintf (dst, size, "%s", s) < size`: copy with truncation
/// and NUL termination, returning whether the whole string fit.
///
/// # Safety
/// `dst` is writable for `size` bytes.
unsafe fn copy_into(dst: *mut c_char, size: usize, bytes: &[u8]) -> bool {
    if size == 0 {
        return false;
    }
    let n = bytes.len().min(size - 1);
    // SAFETY: caller contract; `n < size`.
    unsafe {
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), dst.cast::<u8>(), n);
        *dst.add(n) = 0;
    }
    bytes.len() < size
}

/// `Mem_Alloc (len + 1)` + copy: an engine-owned C string the caller
/// `Mem_Free`s (`Sys_StackTrace` / `Sys_GetEGSLauncherData` contract).
fn mem_strdup(s: &[u8]) -> *mut c_char {
    // SAFETY: Mem_Alloc returns `len + 1` zeroed bytes (a null on allocation
    // failure faults here exactly as `q_strcatf` would have).
    unsafe {
        let out = c::Mem_Alloc(s.len() + 1).cast::<u8>();
        core::ptr::copy_nonoverlapping(s.as_ptr(), out, s.len());
        *out.add(s.len()) = 0;
        out.cast()
    }
}

/// The `Sys_Error` prologue both platforms share: `host_parms->errstate++`
/// off the worker threads, `Sys_DebugBreak`, the appended stack trace when
/// no debugger is attached, `PR_SwitchQCVM (NULL)`. Returns the full
/// message (the C `text` after `q_strcatf`).
///
/// # Safety
/// `text` is a NUL-terminated string.
unsafe fn error_prologue(text: *const c_char) -> Vec<u8> {
    // SAFETY: caller contract; `host_parms` is set before `Sys_Init`.
    unsafe {
        if !quake_c_sys::tasks::Tasks_IsWorker() {
            (*quake_c_sys::host::host_parms).errstate += 1;
        }

        let mut full = CStr::from_ptr(text).to_bytes().to_vec();

        debug_break();

        if !is_in_debugger() {
            let captured_stack_trace = stack_trace();

            full.extend_from_slice(b"\nSTACK TRACE:\n");
            full.extend_from_slice(CStr::from_ptr(captured_stack_trace).to_bytes());

            c::Mem_Free(captured_stack_trace.cast());
        }

        if !quake_c_sys::tasks::Tasks_IsWorker() {
            c::PR_SwitchQCVM(core::ptr::null_mut());
        }

        full
    }
}
