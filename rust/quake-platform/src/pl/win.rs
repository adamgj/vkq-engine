//! `Quake/pl_win.c`: the Win32 arm of `PL_*`. The icon is the `icon`
//! resource `Windows/vkqr-engine.rc` compiles into the executable; it is
//! attached to the SDL window's class through the HWND SDL3 exposes as a
//! window property (Windows builds are SDL3-only, task plan M4 landmine).

use core::ffi::{c_char, c_void};
use core::ptr::{self, addr_of, addr_of_mut};

use windows_sys::Win32::System::DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleA;
use windows_sys::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DestroyIcon, LoadIconA, MessageBoxA, SetClassLongPtrA, GCLP_HICON, HICON, MB_ICONSTOP, MB_OK,
    MB_SETFOREGROUND,
};

/// `winuser.h` -- `#define CF_TEXT 1`; spelled here rather than pulling
/// `Win32_System_Ole` in for one constant.
const CF_TEXT: u32 = 1;

/// `static HICON icon;` -- destroyed by [`vid_shutdown`].
static mut ICON: HICON = ptr::null_mut();

/// The HWND behind `VID_GetWindow ()`, or null.
#[cfg(feature = "sdl3")]
unsafe fn window_hwnd() -> *mut c_void {
    use quake_c_sys as sys;
    use sdl3::sys::properties::SDL_GetPointerProperty;
    use sdl3::sys::video::{SDL_GetWindowProperties, SDL_PROP_WINDOW_WIN32_HWND_POINTER};
    // SAFETY: caller contract -- main thread with the SDL window created;
    // the property name is a static NUL-terminated string.
    unsafe {
        SDL_GetPointerProperty(
            SDL_GetWindowProperties(sys::input::VID_GetWindow().cast()),
            SDL_PROP_WINDOW_WIN32_HWND_POINTER,
            ptr::null_mut(),
        )
    }
}

/// `pl_win.c` uses the SDL3 property API unconditionally, so an SDL2
/// Windows engine is not a configuration the C build has either; the arm
/// exists only so `--features platform,sdl2` type-checks on a Windows dev
/// box.
#[cfg(feature = "sdl2")]
unsafe fn window_hwnd() -> *mut c_void {
    ptr::null_mut()
}

/// # Safety
/// Main thread, after the SDL window exists.
pub unsafe fn set_window_icon() {
    // SAFETY: LoadIconA with the module handle and a static resource name;
    // the icon handle is stored for DestroyIcon at shutdown.
    let icon = unsafe { LoadIconA(GetModuleHandleA(ptr::null()), c"icon".as_ptr().cast()) };
    // SAFETY: main-thread-only static (caller contract).
    unsafe { *addr_of_mut!(ICON) = icon };
    if icon.is_null() {
        return; /* no icon in the exe */
    }
    // SAFETY: caller contract.
    let hwnd = unsafe { window_hwnd() };
    if hwnd.is_null() {
        return;
    }
    // SAFETY: hwnd is the live SDL window's handle; GCLP_HICON takes a
    // handle-sized value.
    unsafe { SetClassLongPtrA(hwnd, GCLP_HICON, icon as isize) };
}

/// # Safety
/// Main thread.
pub unsafe fn vid_shutdown() {
    // SAFETY: main-thread-only static; DestroyIcon (NULL) merely fails, as
    // in C.
    unsafe { DestroyIcon(*addr_of!(ICON)) };
}

/// # Safety
/// Main thread.
pub unsafe fn get_clipboard_data() -> *mut c_char {
    let mut data = ptr::null_mut();
    // SAFETY: the Win32 clipboard protocol as pl_win.c runs it -- the
    // global handle is locked only between OpenClipboard/CloseClipboard,
    // and GlobalSize is the lockable extent, so q_strlcpy never reads past
    // the block even when the text lacks a NUL.
    unsafe {
        if OpenClipboard(ptr::null_mut()) != 0 {
            let clipboard_data = GetClipboardData(CF_TEXT);
            if !clipboard_data.is_null() {
                let cliptext = GlobalLock(clipboard_data).cast::<c_char>();
                if !cliptext.is_null() {
                    let size = GlobalSize(clipboard_data) + 1;
                    data = super::copy_clipboard(cliptext, size);
                    GlobalUnlock(clipboard_data);
                }
            }
            CloseClipboard();
        }
    }
    data
}

/// # Safety
/// `text` is a valid NUL-terminated string.
pub unsafe fn error_dialog(text: *const c_char) {
    // SAFETY: caller contract; the caption is a static string.
    unsafe {
        MessageBoxA(
            ptr::null_mut(),
            text.cast(),
            c"Quake Error".as_ptr().cast(),
            MB_OK | MB_SETFOREGROUND | MB_ICONSTOP,
        );
    }
}
