//! Rust-side engine-global stand-ins for the quake-capi `net` shims in the
//! test binaries (Phase 5).
//!
//! In the engine these symbols come from net_main.c / net_bsd.c / harness.c;
//! here the c_ref net subsystem owns its *renamed* copies (see
//! c_ref_prelude.h), so the unrenamed names the Rust shims import are
//! defined here. The reader trio is the exception: quake-capi owns it from
//! Phase 7 M9e and this module only re-exports it. Tests point
//! `net_message.data` at their own buffers and reset the qsocket pool
//! between scenarios.

#![allow(non_upper_case_globals, missing_docs)]

use core::ffi::c_int;
use core::ptr;

use quake_c_sys::{qboolean, qsocket_s};
use quake_types::net::QSocket;

// Phase 7 M9e: net_message/msg_readcount/msg_badread became Rust-owned
// storage in quake-capi's `net` module (ADR-007 net row closed), and this
// link enables that feature, so they are re-exported here rather than
// defined -- defining them again is a duplicate symbol. The re-export keeps
// the `net_stubs::net_message` path the tests already use.
pub use quake_rs::net::{msg_badread, msg_readcount, net_message};
#[no_mangle]
pub static mut harness_badread_count: core::ffi::c_uint = 0;
#[no_mangle]
pub static mut net_driverlevel: c_int = 0;

// Phase 5 M7: the dgrm/udp shims' ambient C globals (net_dgrm_glue.c /
// net_main.c / net_bsd.c in the engine). The linux linker resolves every
// rlib object, so these must exist even in tests that never call the shims.
#[no_mangle]
pub static mut net_time: f64 = 0.0;
#[no_mangle]
pub static mut net_hostport: c_int = 26000;
#[repr(C)]
pub struct StubPacketBuffer(pub [u8; 64008]);
#[no_mangle]
pub static mut packetBuffer: StubPacketBuffer = StubPacketBuffer([0; 64008]);
#[no_mangle]
pub static mut packetsSent: c_int = 0;
#[no_mangle]
pub static mut packetsReSent: c_int = 0;
#[no_mangle]
pub static mut packetsReceived: c_int = 0;
#[no_mangle]
pub static mut receivedDuplicateCount: c_int = 0;
#[no_mangle]
pub static mut shortPacketCount: c_int = 0;
#[no_mangle]
pub static mut droppedDatagrams: c_int = 0;
#[no_mangle]
pub static mut messagesReceived: c_int = 0;
#[no_mangle]
pub static mut unreliableMessagesReceived: c_int = 0;

// Phase 5 M9: the net_main.c core's ambient globals
#[no_mangle]
pub static mut net_activeSockets: *mut qsocket_s = ptr::null_mut();
#[no_mangle]
pub static mut net_freeSockets: *mut qsocket_s = ptr::null_mut();
#[no_mangle]
pub static mut net_activeconnections: c_int = 0;
#[no_mangle]
pub static mut DEFAULTnet_hostport: c_int = 26000;
#[no_mangle]
pub static mut listening: bool = false;
#[no_mangle]
pub static mut hostCacheCount: usize = 0;

const EMPTY_HOSTCACHE: quake_types::net::HostCache = quake_types::net::HostCache {
    name: [0; 64],
    map: [0; 16],
    gamedir: [0; 16],
    cname: [0; 64],
    users: 0,
    maxusers: 0,
    driver: 0,
    ldriver: 0,
    addr: quake_types::net::QSockAddr::zeroed(),
};
#[no_mangle]
pub static mut hostcache: [quake_types::net::HostCache; 128] = [EMPTY_HOSTCACHE; 128];

#[no_mangle]
pub static net_numdrivers: c_int = 2;

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

// Phase 7 M9c: net_main.c's remaining ambient globals (the funnel cores) and
// the harness.c net-replay seam. The replay is always off in ctest, so the
// four Harness_Net* entry points are inert stand-ins.
#[no_mangle]
pub static mut slistInProgress: qboolean = false;
#[no_mangle]
pub static mut slist_silent: qboolean = false;
#[no_mangle]
pub static mut net_numsockets: c_int = 0;
#[no_mangle]
pub static mut harness_netreplay: qboolean = false;

/// # Safety
/// Single-threaded tests.
#[no_mangle]
pub unsafe extern "C" fn Harness_NetCapture(
    _direction: c_int,
    _driver: c_int,
    _kind: c_int,
    _data: *const u8,
    _len: c_int,
) {
}

/// # Safety
/// Single-threaded tests.
#[no_mangle]
pub unsafe extern "C" fn Harness_NetReplayConnect() -> *mut qsocket_s {
    ptr::null_mut()
}

/// # Safety
/// Single-threaded tests.
#[no_mangle]
pub unsafe extern "C" fn Harness_NetReplayOwns(_sock: *mut qsocket_s) -> qboolean {
    false
}

/// # Safety
/// Single-threaded tests.
#[no_mangle]
pub unsafe extern "C" fn Harness_NetReplayGetMessage() -> c_int {
    -1
}
