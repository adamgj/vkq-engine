//! `Quake/pl_win.c` / `Quake/pl_linux.c` (Rust migration Phase 9 M4,
//! ADR-017): the four `PL_*` entry points behind `platform.h`.
//!
//! The Windows arm (`pl/win.rs`) keeps the Win32 calls the C file made
//! (`windows-sys`, ADR-003 Phase 9 amendment); every other host uses SDL
//! (`pl/sdl.rs`), which is what `pl_linux.c` did. `pl_osx.m` was never in
//! the Meson build, so macOS takes the SDL arm too -- its `NSPasteboard`
//! read becomes `SDL_GetClipboardText` over SDL's Cocoa backend (task plan
//! D5, accepted divergence).
//!
//! Nothing here can raise: `Mem_Alloc` is a plain `calloc` (`mem.c:88`),
//! so no `Host_Guard` trampolines are involved (ADR-009).

use core::ffi::c_char;

use quake_c_sys as sys;

#[cfg(windows)]
#[path = "pl/win.rs"]
mod imp;
#[cfg(not(windows))]
#[path = "pl/sdl.rs"]
mod imp;

/// `#define MAX_CLIPBOARDTXT MAXCMDLINE /* 256 */`
const MAX_CLIPBOARDTXT: usize = sys::console::MAXCMDLINE;

/// C: `void PL_SetWindowIcon (void)`
///
/// # Safety
/// Main thread, after the SDL window exists (`VID_GetWindow` non-null).
pub unsafe fn set_window_icon() {
    // SAFETY: forwarded contract
    unsafe { imp::set_window_icon() }
}

/// C: `void PL_VID_Shutdown (void)`
///
/// # Safety
/// Main thread.
pub unsafe fn vid_shutdown() {
    // SAFETY: forwarded contract
    unsafe { imp::vid_shutdown() }
}

/// C: `char *PL_GetClipboardData (void)` -- a `Mem_Alloc`ed copy the
/// caller `Mem_Free`s, or null when there is no text on the clipboard.
///
/// # Safety
/// Main thread.
pub unsafe fn get_clipboard_data() -> *mut c_char {
    // SAFETY: forwarded contract
    unsafe { imp::get_clipboard_data() }
}

/// C: `void PL_ErrorDialog (const char *text)`
///
/// # Safety
/// `text` is a valid NUL-terminated string.
pub unsafe fn error_dialog(text: *const c_char) {
    // SAFETY: forwarded contract
    unsafe { imp::error_dialog(text) }
}

/// The tail both arms share: chop `size` (the source length plus NUL) to
/// `MAX_CLIPBOARDTXT` before allocating -- "this is intended for simple
/// small text copies such as an ip address, etc" -- then `q_strlcpy`.
///
/// # Safety
/// `cliptext` is readable for `min (size, MAX_CLIPBOARDTXT)` bytes or
/// NUL-terminated before that.
unsafe fn copy_clipboard(cliptext: *const c_char, size: usize) -> *mut c_char {
    let size = MAX_CLIPBOARDTXT.min(size);
    // SAFETY: Mem_Alloc returns `size` zeroed bytes (a null on allocation
    // failure faults in q_strlcpy exactly as the C did); q_strlcpy stops at
    // `size - 1` or the source NUL, whichever comes first.
    unsafe {
        let data = sys::Mem_Alloc(size).cast::<c_char>();
        sys::console::q_strlcpy(data, cliptext, size);
        data
    }
}
