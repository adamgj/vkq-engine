//! Phase 5 M5: the loopback net driver (net_loop.c), installed into
//! `net_drivers[0]` by net_bsd.c/net_win.c under `USE_RUST_NET`.
//!
//! The three file statics (`localconnectpending`, `loop_client`,
//! `loop_server`) become Rust module state -- never C-visible (ADR-007).
//! The frame format lives in `quake_net::loopback`; this module does the
//! qsocket plumbing through the hand-written `quake_types::net` mirrors.
//! `Loop_SearchForHosts` stays C until M9: it is hostcache/slist plumbing
//! (net_main.c state), not wire logic.
//!
//! Error posture (ADR-009): the only error paths here are `Sys_Error`s
//! (fatal exit, no longjmp), mirrored via `quake_c_sys::Sys_Error`.

use core::ffi::{c_char, c_int, CStr};
use core::ptr;

use quake_c_sys as c;
use quake_net::loopback;
use quake_types::net::{QSocket, SizeBuf};

static mut LOCAL_CONNECT_PENDING: bool = false;
static mut LOOP_CLIENT: *mut QSocket = ptr::null_mut();
static mut LOOP_SERVER: *mut QSocket = ptr::null_mut();

/// copies `s` + NUL into a C char array field
fn set_addr(dst: &mut [c_char], s: &[u8]) {
    for (i, &b) in s.iter().enumerate() {
        dst[i] = b as c_char;
    }
    dst[s.len()] = 0;
}

/// COMPAT: C tests `cls.state == ca_dedicated`; host.c sets that state from
/// the `isDedicated` global before NET_Init runs, so the bound global is
/// equivalent (cls lives in a non-bindgen-clean header).
#[no_mangle]
pub extern "C" fn rust_loop_Init() -> c_int {
    // SAFETY: read of a host-owned flag set before subsystem init
    if unsafe { c::isDedicated } {
        -1
    } else {
        0
    }
}

#[no_mangle]
pub extern "C" fn rust_loop_Shutdown() {}

#[no_mangle]
pub extern "C" fn rust_loop_Listen(_state: bool) {}

/// # Safety
/// `host` is a NUL-terminated string; single-threaded host frame.
#[no_mangle]
pub unsafe extern "C" fn rust_loop_Connect(host: *const c_char) -> *mut QSocket {
    // SAFETY: caller contract; the loop statics are host-thread-only, like
    // the C file statics they replace
    unsafe {
        if CStr::from_ptr(host).to_bytes() != b"local" {
            return ptr::null_mut();
        }

        LOCAL_CONNECT_PENDING = true;

        if LOOP_CLIENT.is_null() {
            LOOP_CLIENT = c::NET_NewQSocket().cast::<QSocket>();
            if LOOP_CLIENT.is_null() {
                c::Con_Printf(c"Loop_Connect: no qsocket available\n".as_ptr());
                return ptr::null_mut();
            }
            set_addr(&mut (*LOOP_CLIENT).trueaddress, b"localhost");
            set_addr(&mut (*LOOP_CLIENT).maskedaddress, b"localhost");
        }
        (*LOOP_CLIENT).receive_message_length = 0;
        (*LOOP_CLIENT).send_message_length = 0;
        (*LOOP_CLIENT).can_send = true;

        if LOOP_SERVER.is_null() {
            LOOP_SERVER = c::NET_NewQSocket().cast::<QSocket>();
            if LOOP_SERVER.is_null() {
                c::Con_Printf(c"Loop_Connect: no qsocket available\n".as_ptr());
                return ptr::null_mut();
            }
            set_addr(&mut (*LOOP_SERVER).trueaddress, b"LOCAL");
            set_addr(&mut (*LOOP_SERVER).maskedaddress, b"LOCAL");
        }
        (*LOOP_SERVER).receive_message_length = 0;
        (*LOOP_SERVER).send_message_length = 0;
        (*LOOP_SERVER).can_send = true;

        (*LOOP_CLIENT).driverdata = LOOP_SERVER.cast();
        (*LOOP_SERVER).driverdata = LOOP_CLIENT.cast();

        (*LOOP_CLIENT).proquake_angle_hack = true;
        (*LOOP_SERVER).proquake_angle_hack = true;

        LOOP_CLIENT
    }
}

#[no_mangle]
pub extern "C" fn rust_loop_CheckNewConnections() -> *mut QSocket {
    // SAFETY: host-thread-only statics; sockets live in the C qsocket pool
    unsafe {
        if !LOCAL_CONNECT_PENDING {
            return ptr::null_mut();
        }
        LOCAL_CONNECT_PENDING = false;
        (*LOOP_SERVER).send_message_length = 0;
        (*LOOP_SERVER).receive_message_length = 0;
        (*LOOP_SERVER).can_send = true;
        (*LOOP_CLIENT).send_message_length = 0;
        (*LOOP_CLIENT).receive_message_length = 0;
        (*LOOP_CLIENT).can_send = true;
        LOOP_SERVER
    }
}

/// SZ_Clear + SZ_Write of one popped payload into the C `net_message`.
///
/// # Safety
/// Single-threaded host frame; net_message initialized by NET_Init.
unsafe fn fill_net_message(payload: &[u8]) {
    // SAFETY: caller contract
    unsafe {
        let nm = &raw mut c::net_message;
        (*nm).cursize = 0;
        (*nm).overflowed = false;
        if payload.len() as i32 > (*nm).maxsize {
            // unreachable: senders' sizebufs are capped at NET_MAXMESSAGE,
            // which is net_message's own allocation size. C would Host_Error
            // out of SZ_GetSpace; a longjmp must not cross this Rust frame
            // (ADR-009), so the impossible case is a hard stop instead.
            c::Sys_Error(c"Loop_GetMessage: net_message overflow".as_ptr());
        }
        ptr::copy_nonoverlapping(payload.as_ptr(), (*nm).data, payload.len());
        (*nm).cursize = payload.len() as c_int;
    }
}

/// # Safety
/// `sock` is a live qsocket from the C pool; single-threaded host frame.
#[no_mangle]
pub unsafe extern "C" fn rust_loop_GetMessage(sock: *mut QSocket) -> c_int {
    // SAFETY: caller contract; receive_message is an in-struct array
    unsafe {
        let s = &mut *sock;
        let Some(p) = loopback::pop_peek(&s.receive_message, s.receive_message_length) else {
            return 0;
        };
        fill_net_message(&s.receive_message[p.payload_start..p.payload_start + p.payload_len]);
        if let Some(seq) = p.new_unreliable_receive_sequence {
            s.unreliable_receive_sequence = seq;
        }
        s.receive_message_length =
            loopback::pop_finish(&mut s.receive_message, s.receive_message_length, &p);

        if !s.driverdata.is_null() && p.msg_type == 1 {
            (*s.driverdata.cast::<QSocket>()).can_send = true;
        }
        p.msg_type
    }
}

#[no_mangle]
pub extern "C" fn rust_loop_GetAnyMessage() -> *mut QSocket {
    // SAFETY: host-thread-only statics
    unsafe {
        if !LOOP_SERVER.is_null() && rust_loop_GetMessage(LOOP_SERVER) > 0 {
            return LOOP_SERVER;
        }
        ptr::null_mut()
    }
}

/// # Safety
/// `sock` live qsocket; `data` a live sizebuf_t; single-threaded host frame.
#[no_mangle]
pub unsafe extern "C" fn rust_loop_SendMessage(sock: *mut QSocket, data: *mut SizeBuf) -> c_int {
    // SAFETY: caller contract
    unsafe {
        let s = &mut *sock;
        if s.driverdata.is_null() {
            return -1;
        }
        // aliasing invariant: Loop_Connect always cross-links two DISTINCT
        // pool sockets, so `peer` never overlaps `s`
        debug_assert!(!core::ptr::eq(s.driverdata.cast::<QSocket>(), sock));
        let peer = &mut *s.driverdata.cast::<QSocket>();
        let payload = core::slice::from_raw_parts((*data).data, (*data).cursize as usize);
        if !loopback::push_reliable(
            &mut peer.receive_message,
            &mut peer.receive_message_length,
            payload,
        ) {
            c::Sys_Error(c"Loop_SendMessage: overflow".as_ptr());
        }
        s.can_send = false;
        1
    }
}

/// # Safety
/// `sock` live qsocket; `data` a live sizebuf_t; single-threaded host frame.
#[no_mangle]
pub unsafe extern "C" fn rust_loop_SendUnreliableMessage(
    sock: *mut QSocket,
    data: *mut SizeBuf,
) -> c_int {
    // SAFETY: caller contract
    unsafe {
        let s = &mut *sock;
        let sequence = s.unreliable_send_sequence;
        s.unreliable_send_sequence = s.unreliable_send_sequence.wrapping_add(1);
        if s.driverdata.is_null() {
            return -1;
        }
        // aliasing invariant: see rust_loop_SendMessage
        debug_assert!(!core::ptr::eq(s.driverdata.cast::<QSocket>(), sock));
        let peer = &mut *s.driverdata.cast::<QSocket>();
        let payload = core::slice::from_raw_parts((*data).data, (*data).cursize as usize);
        if loopback::push_unreliable(
            &mut peer.receive_message,
            &mut peer.receive_message_length,
            payload,
            sequence,
        ) {
            1
        } else {
            0
        }
    }
}

/// # Safety
/// `sock` is a live qsocket.
#[no_mangle]
pub unsafe extern "C" fn rust_loop_CanSendMessage(sock: *mut QSocket) -> bool {
    // SAFETY: caller contract
    unsafe {
        if (*sock).driverdata.is_null() {
            return false;
        }
        (*sock).can_send
    }
}

/// # Safety
/// `sock` is a live qsocket (unused, like the C original).
#[no_mangle]
pub unsafe extern "C" fn rust_loop_CanSendUnreliableMessage(_sock: *mut QSocket) -> bool {
    true
}

/// # Safety
/// `sock` is a live qsocket; single-threaded host frame.
#[no_mangle]
pub unsafe extern "C" fn rust_loop_Close(sock: *mut QSocket) {
    // SAFETY: caller contract; statics host-thread-only
    unsafe {
        let s = &mut *sock;
        if !s.driverdata.is_null() {
            (*s.driverdata.cast::<QSocket>()).driverdata = ptr::null_mut();
        }
        s.receive_message_length = 0;
        s.send_message_length = 0;
        s.can_send = true;
        if sock == LOOP_CLIENT {
            LOOP_CLIENT = ptr::null_mut();
        } else {
            LOOP_SERVER = ptr::null_mut();
        }
    }
}
