//! `Quake/pl_linux.c`: the SDL arm of `PL_*` (every non-Windows host).

use core::ffi::{c_char, CStr};
use core::ptr;

use quake_c_sys as sys;

/// `static const Uint8 bmp_bytes[] = { #include "qs_bmp.h" };` -- the
/// header is parsed into a blob by `build.rs`.
static BMP_BYTES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/qs_bmp.bin"));

/// # Safety
/// Main thread, after the SDL window exists.
#[cfg(feature = "sdl3")]
pub unsafe fn set_window_icon() {
    use sdl3::sys::iostream::SDL_IOFromConstMem;
    use sdl3::sys::surface::{
        SDL_DestroySurface, SDL_LoadBMP_IO, SDL_MapSurfaceRGBA, SDL_SetSurfaceColorKey,
    };
    use sdl3::sys::video::SDL_SetWindowIcon;

    // SAFETY: the stream reads the static blob and is closed by
    // SDL_LoadBMP_IO (closeio = true); the surface is destroyed after the
    // window has copied it.
    unsafe {
        let rwop = SDL_IOFromConstMem(BMP_BYTES.as_ptr().cast(), BMP_BYTES.len());
        if rwop.is_null() {
            return;
        }
        let icon = SDL_LoadBMP_IO(rwop, true);
        if icon.is_null() {
            return;
        }
        /* make pure magenta (#ff00ff) tranparent */
        let colorkey = SDL_MapSurfaceRGBA(icon, 255, 0, 255, 255);
        SDL_SetSurfaceColorKey(icon, true, colorkey);
        SDL_SetWindowIcon(sys::input::VID_GetWindow().cast(), icon);
        SDL_DestroySurface(icon);
    }
}

/// # Safety
/// Main thread, after the SDL window exists.
#[cfg(feature = "sdl2")]
pub unsafe fn set_window_icon() {
    use sdl2::sys::{
        SDL_FreeSurface, SDL_LoadBMP_RW, SDL_MapRGB, SDL_RWFromConstMem, SDL_SetColorKey,
        SDL_SetWindowIcon, SDL_bool,
    };

    // SAFETY: as the SDL3 arm; SDL_LoadBMP_RW (freesrc = 1) closes the
    // stream, and the surface's `format` is valid until SDL_FreeSurface.
    unsafe {
        let rwop = SDL_RWFromConstMem(BMP_BYTES.as_ptr().cast(), BMP_BYTES.len() as _);
        if rwop.is_null() {
            return;
        }
        let icon = SDL_LoadBMP_RW(rwop, 1);
        if icon.is_null() {
            return;
        }
        /* make pure magenta (#ff00ff) tranparent */
        let colorkey = SDL_MapRGB((*icon).format, 255, 0, 255);
        SDL_SetColorKey(icon, SDL_bool::SDL_TRUE as _, colorkey);
        SDL_SetWindowIcon(sys::input::VID_GetWindow().cast(), icon);
        SDL_FreeSurface(icon);
    }
}

/// `void PL_VID_Shutdown (void) {}`
///
/// # Safety
/// Main thread.
pub unsafe fn vid_shutdown() {}

/// # Safety
/// Main thread.
pub unsafe fn get_clipboard_data() -> *mut c_char {
    #[cfg(feature = "sdl2")]
    use sdl2::sys::SDL_GetClipboardText;
    #[cfg(feature = "sdl3")]
    use sdl3::sys::clipboard::SDL_GetClipboardText;

    // SAFETY: SDL returns a NUL-terminated string or null. pl_linux.c never
    // SDL_free()s it (the text leaks in C too); kept as-is for parity.
    unsafe {
        let cliptext = SDL_GetClipboardText();
        if cliptext.is_null() {
            return ptr::null_mut();
        }
        let size = CStr::from_ptr(cliptext).to_bytes().len() + 1;
        super::copy_clipboard(cliptext, size)
    }
}

/// # Safety
/// `text` is a valid NUL-terminated string.
pub unsafe fn error_dialog(text: *const c_char) {
    #[cfg(feature = "sdl2")]
    use sdl2::sys::{SDL_MessageBoxFlags::SDL_MESSAGEBOX_ERROR, SDL_ShowSimpleMessageBox};
    #[cfg(feature = "sdl3")]
    use sdl3::sys::messagebox::{SDL_ShowSimpleMessageBox, SDL_MESSAGEBOX_ERROR};

    // SAFETY: caller contract; the title is a static string, no parent.
    unsafe {
        SDL_ShowSimpleMessageBox(
            SDL_MESSAGEBOX_ERROR as _,
            c"Quake Error".as_ptr(),
            text,
            ptr::null_mut(),
        );
    }
}
