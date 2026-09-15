//! The process entry symbols `Quake/main_sdl.c` used to define (Rust
//! migration Phase 9 M6, host inversion): the C remnant is now a static
//! library the Rust entry calls into, per PLAN.md §"Ownership inversion".
//!
//! The body is `quake_platform::main_sdl::quake_main`; this module only spells
//! the symbol each platform's C runtime / SDL start-up looks for:
//!
//! - everywhere but Windows: `main`, exactly as `main_sdl.c` (neither SDL
//!   generation renames it on Linux/macOS);
//! - Windows + SDL3: `WinMain` (the executable links with `win_subsystem :
//!   'windows'`), forwarding to `SDL_RunApp` the way `SDL3/SDL_main.h`'s
//!   inline `WinMain` did in `main_sdl.c` -- SDL converts the wide command
//!   line to a UTF-8 `argv`;
//! - Windows + SDL2: `SDL_main`, the symbol `SDL2main.lib`'s own `WinMain`
//!   calls (`SDL.h` renamed `main` to it in the C build).
//!
//! No C TU may define `main`/`WinMain` or include `SDL3/SDL_main.h`'s
//! entry shim any more (Meson drops `main_sdl.c` under
//! `-Duse_rust_platform`).

#![allow(clippy::missing_safety_doc)]

use core::ffi::{c_char, c_int};

#[cfg(not(all(windows, feature = "sdl3")))]
use quake_platform::main_sdl::quake_main;

/// `int main (int argc, char *argv[])`.
#[cfg(not(windows))]
#[no_mangle]
pub unsafe extern "C" fn main(argc: c_int, argv: *mut *mut c_char) -> c_int {
    // SAFETY: the C runtime's entry call.
    unsafe { quake_main(argc, argv) }
}

/// `int WINAPI WinMain (HINSTANCE, HINSTANCE, LPSTR, int)` --
/// `SDL3/SDL_main.h`'s Windows shim: `SDL_RunApp (0, NULL, SDL_main, NULL)`.
#[cfg(all(windows, feature = "sdl3"))]
#[no_mangle]
pub unsafe extern "system" fn WinMain(
    _hinstance: *mut core::ffi::c_void,
    _hprev: *mut core::ffi::c_void,
    _cmdline: *mut c_char,
    _show: c_int,
) -> c_int {
    // SAFETY: the CRT's entry call.
    unsafe { quake_platform::main_sdl::run_app() }
}

/// `int SDL_main (int argc, char *argv[])` -- what `SDL2main.lib`'s
/// `WinMain` calls after building the UTF-8 `argv`.
#[cfg(all(windows, feature = "sdl2"))]
#[no_mangle]
pub unsafe extern "C" fn SDL_main(argc: c_int, argv: *mut *mut c_char) -> c_int {
    // SAFETY: SDL2main's entry call.
    unsafe { quake_main(argc, argv) }
}
