//! Rust-side engine-global stand-ins for the quake-capi `net` shims in the
//! test binaries (Phase 5).
//!
//! In the engine these symbols come from net_main.c / net_msg_glue.c /
//! harness.c; here the c_ref net subsystem owns its *renamed* copies (see
//! c_ref_prelude.h), so the unrenamed names the Rust shims import are
//! defined here. Tests point `net_message.data` at their own buffers and
//! reset the qsocket pool between scenarios.

#![allow(non_upper_case_globals, missing_docs)]

use core::ffi::c_int;
use core::ptr;

use quake_c_sys::{qboolean, qsocket_s, sizebuf_t};
use quake_types::net::QSocket;

#[no_mangle]
pub static mut net_message: sizebuf_t = sizebuf_t {
    allowoverflow: false,
    overflowed: false,
    data: ptr::null_mut(),
    maxsize: 0,
    cursize: 0,
};
#[no_mangle]
pub static mut msg_readcount: c_int = 0;
#[no_mangle]
pub static mut msg_badread: qboolean = false;
#[no_mangle]
pub static mut harness_badread_count: core::ffi::c_uint = 0;
#[no_mangle]
pub static mut net_driverlevel: c_int = 0;

const POOL: usize = 4;
static mut QSOCKET_POOL: [core::mem::MaybeUninit<QSocket>; POOL] =
    [const { core::mem::MaybeUninit::uninit() }; POOL];
static mut QSOCKET_USED: usize = 0;

/// Rust-side NET_NewQSocket stand-in: hands out zeroed pool slots like the
/// c_ref stub in stubs.c does for the C side.
///
/// # Safety
/// Single-threaded tests (the differential suites serialize on a mutex).
#[no_mangle]
pub unsafe extern "C" fn NET_NewQSocket() -> *mut qsocket_s {
    // SAFETY: single-threaded test contract; the slot is zeroed before use
    unsafe {
        if QSOCKET_USED >= POOL {
            return ptr::null_mut();
        }
        let slot = (&raw mut QSOCKET_POOL).cast::<QSocket>().add(QSOCKET_USED);
        QSOCKET_USED += 1;
        ptr::write_bytes(slot.cast::<u8>(), 0, core::mem::size_of::<QSocket>());
        slot.cast::<qsocket_s>()
    }
}

/// # Safety
/// Single-threaded tests.
#[no_mangle]
pub unsafe extern "C" fn NET_FreeQSocket(_sock: *mut qsocket_s) {}

/// resets the Rust-side pool between test scenarios
///
/// # Safety
/// Single-threaded tests.
pub unsafe fn qsocket_reset_rust() {
    // SAFETY: single-threaded test contract
    unsafe {
        QSOCKET_USED = 0;
    }
}
