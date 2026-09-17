//! `IN_*` exports over `quake_platform::input` (Rust migration Phase 9 M3,
//! ADR-017/ADR-011). Built under the `platform` feature with whichever SDL
//! arm Meson selected; `Quake/in_sdl_glue.c` owns the cvars and wraps the
//! three entry points that can raise (`IN_Init`, `IN_SendKeyEvents`,
//! `IN_Commands`) in `Host_Reraise` (ADR-009), so those are exported as
//! status-returning `quake_rs_in_*` cores. Everything else in `input.h`
//! cannot raise and is exported under its C name directly.

use core::ffi::c_int;

use quake_c_sys as sys;
use quake_platform::input as backend;
use quake_types::host::UserCmd;

/// C: `void IN_Init (void)` -- the status-returning core; in_sdl_glue.c
/// re-raises.
///
/// # Safety
/// Main thread, from `Host_Init`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_in_init() -> c_int {
    // SAFETY: forwarded contract
    unsafe { backend::init() }
}

/// C: `void IN_SendKeyEvents (void)` -- the status-returning core.
///
/// # Safety
/// Main thread, after `IN_Init`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_in_send_key_events() -> c_int {
    // SAFETY: forwarded contract
    unsafe { backend::send_key_events() }
}

/// C: `void IN_Commands (void)` -- the status-returning core.
///
/// # Safety
/// Main thread, after `IN_Init`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_in_commands() -> c_int {
    // SAFETY: forwarded contract
    unsafe { backend::commands() }
}

/// C: `void IN_Shutdown (void)`
///
/// # Safety
/// Main thread.
#[no_mangle]
pub unsafe extern "C" fn IN_Shutdown() {
    // SAFETY: forwarded contract
    unsafe { backend::shutdown() }
}

/// C: `void IN_Move (usercmd_t *cmd)`
///
/// # Safety
/// Main thread; `cmd` valid.
#[no_mangle]
pub unsafe extern "C" fn IN_Move(cmd: *mut UserCmd) {
    // SAFETY: forwarded contract
    unsafe { backend::r#move(cmd) }
}

/// C: `void IN_MouseMotion (float dx, float dy)`
///
/// # Safety
/// Main thread.
#[no_mangle]
pub unsafe extern "C" fn IN_MouseMotion(dx: f32, dy: f32) {
    // SAFETY: forwarded contract
    unsafe { backend::mouse_motion(dx, dy) }
}

/// C: `void IN_UpdateInputMode (void)`
///
/// # Safety
/// Main thread.
#[no_mangle]
pub unsafe extern "C" fn IN_UpdateInputMode() {
    // SAFETY: forwarded contract
    unsafe { backend::update_input_mode() }
}

/// C: `void IN_ClearStates (void)` -- empty in in_sdl.c too.
#[no_mangle]
pub extern "C" fn IN_ClearStates() {}

/// C: `void IN_Activate (void)`
///
/// # Safety
/// Main thread.
#[no_mangle]
pub unsafe extern "C" fn IN_Activate() {
    // SAFETY: forwarded contract
    unsafe { backend::activate() }
}

/// C: `void IN_Deactivate (qboolean free_cursor)`
///
/// # Safety
/// Main thread.
#[no_mangle]
pub unsafe extern "C" fn IN_Deactivate(free_cursor: sys::qboolean) {
    // SAFETY: forwarded contract
    unsafe { backend::deactivate(free_cursor) }
}

/// C: `void IN_DeactivateForConsole (void)`
///
/// # Safety
/// Main thread.
#[no_mangle]
pub unsafe extern "C" fn IN_DeactivateForConsole() {
    // SAFETY: forwarded contract
    unsafe { backend::deactivate_for_console() }
}

/// C: `void IN_HideCursor (void)`
///
/// # Safety
/// Main thread.
#[no_mangle]
pub unsafe extern "C" fn IN_HideCursor() {
    // SAFETY: forwarded contract
    unsafe { backend::hide_cursor() }
}

/// C: `void IN_GetMousePos (int *outx, int *outy)`
///
/// # Safety
/// Main thread; `outx`/`outy` valid.
#[no_mangle]
pub unsafe extern "C" fn IN_GetMousePos(outx: *mut c_int, outy: *mut c_int) {
    // SAFETY: forwarded contract
    unsafe {
        let (x, y) = backend::get_mouse_pos();
        *outx = x;
        *outy = y;
    }
}

/// C: `void IN_ScaleMouseCoords (float x, float y, int *outx, int *outy)`
///
/// # Safety
/// Main thread; `outx`/`outy` valid.
#[no_mangle]
pub unsafe extern "C" fn IN_ScaleMouseCoords(x: f32, y: f32, outx: *mut c_int, outy: *mut c_int) {
    // SAFETY: forwarded contract
    unsafe {
        let (sx, sy) = backend::scale_mouse_coords(x, y);
        *outx = sx;
        *outy = sy;
    }
}
