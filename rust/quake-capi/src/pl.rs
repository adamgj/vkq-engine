//! `PL_*` exports over `quake_platform::pl` (Rust migration Phase 9 M4,
//! ADR-017/ADR-011). Built under the `platform` feature; none of the four
//! can raise (`Mem_Alloc` is a plain `calloc`), so each is exported under
//! its C name directly and `Quake/pl_win.c`/`pl_linux.c` leave the build.

use core::ffi::c_char;

use quake_platform::pl as backend;

/// C: `void PL_SetWindowIcon (void)`
///
/// # Safety
/// Main thread, after the SDL window exists.
#[no_mangle]
pub unsafe extern "C" fn PL_SetWindowIcon() {
    // SAFETY: forwarded contract
    unsafe { backend::set_window_icon() }
}

/// C: `void PL_VID_Shutdown (void)`
///
/// # Safety
/// Main thread.
#[no_mangle]
pub unsafe extern "C" fn PL_VID_Shutdown() {
    // SAFETY: forwarded contract
    unsafe { backend::vid_shutdown() }
}

/// C: `char *PL_GetClipboardData (void)` -- `Mem_Alloc`ed, caller frees.
///
/// # Safety
/// Main thread.
#[no_mangle]
pub unsafe extern "C" fn PL_GetClipboardData() -> *mut c_char {
    // SAFETY: forwarded contract
    unsafe { backend::get_clipboard_data() }
}

/// C: `void PL_ErrorDialog (const char *text)`
///
/// # Safety
/// `text` is a valid NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn PL_ErrorDialog(text: *const c_char) {
    // SAFETY: forwarded contract
    unsafe { backend::error_dialog(text) }
}
