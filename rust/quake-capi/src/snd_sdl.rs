//! SNDDMA_* exports over the quake-platform SDL3 audio backend (Phase 4 M9,
//! ADR-017/ADR-011). Built only under `snd`+`sdl3`: with SDL2, snd_sdl.c
//! stays the C backend (its Rust port follows once a use_rust+SDL2 CI leg
//! exists to verify it -- recorded in the phase plan).

use core::ffi::c_int;

use quake_c_sys as sys;
use quake_platform::snd_sdl3 as backend;

/// C: `qboolean SNDDMA_Init (dma_t *dma)`
///
/// # Safety
/// Main thread; `dma` valid.
#[no_mangle]
pub unsafe extern "C" fn SNDDMA_Init(dma: *mut sys::dma_t) -> sys::qboolean {
    // SAFETY: forwarded contract
    unsafe { backend::snddma_init(dma) }
}

/// C: `int SNDDMA_GetDMAPos (void)`
///
/// # Safety
/// Sound started.
#[no_mangle]
pub unsafe extern "C" fn SNDDMA_GetDMAPos() -> c_int {
    // SAFETY: forwarded contract
    unsafe { backend::snddma_get_dma_pos() }
}

/// C: `void SNDDMA_Shutdown (void)`
///
/// # Safety
/// Main thread.
#[no_mangle]
pub unsafe extern "C" fn SNDDMA_Shutdown() {
    // SAFETY: forwarded contract
    unsafe { backend::snddma_shutdown() }
}

/// C: `void SNDDMA_LockBuffer (void)`
///
/// # Safety
/// Main thread.
#[no_mangle]
pub unsafe extern "C" fn SNDDMA_LockBuffer() {
    // SAFETY: forwarded contract
    unsafe { backend::snddma_lock_buffer() }
}

/// C: `void SNDDMA_Submit (void)`
///
/// # Safety
/// Main thread.
#[no_mangle]
pub unsafe extern "C" fn SNDDMA_Submit() {
    // SAFETY: forwarded contract
    unsafe { backend::snddma_submit() }
}

/// C: `void SNDDMA_BlockSound (void)`
///
/// # Safety
/// Main thread.
#[no_mangle]
pub unsafe extern "C" fn SNDDMA_BlockSound() {
    // SAFETY: forwarded contract
    unsafe { backend::snddma_block_sound() }
}

/// C: `void SNDDMA_UnblockSound (void)`
///
/// # Safety
/// Main thread.
#[no_mangle]
pub unsafe extern "C" fn SNDDMA_UnblockSound() {
    // SAFETY: forwarded contract
    unsafe { backend::snddma_unblock_sound() }
}
