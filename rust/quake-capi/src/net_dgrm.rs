//! Phase 5 M7: engine shims for the Rust dgrm reliable layer
//! (`quake_net::dgrm`), wrapped by `Quake/net_dgrm_glue.c` which keeps the
//! `Datagram_*` symbol names for the net_bsd.c/net_win.c vtables and
//! net_dgrm.c's orchestration half.
//!
//! Ownership (ADR-007 net row): the `packetBuffer` scratch, the six stat
//! counters, `net_time`, `net_message` and the net_main message counters
//! stay C-owned; these shims marshal them around the pure core per call.
//! `net_landrivers[]` is reached through the hand-written ADR-011 mirror
//! (`quake_types::net::NetLanDriver`, pinned per-platform by net_abi.rs).
//!
//! Error posture (ADR-009): raises stay in C frames. The glue pre-validates
//! the send paths (the DEBUG Sys_Errors and the oversize guard) before
//! calling in, and re-raises get_message's hostile-length overflow status
//! as the exact SZ_GetSpace Host_Error.
//!
//! Console diagnostics accumulate Rust-side and drain to
//! Con_Printf/Con_DPrintf after every C-memory borrow ends (the M3 review
//! lesson: Con_Printf is not a leaf).

use core::ffi::{c_int, c_uint, CStr};

use quake_c_sys as c;
use quake_net::dgrm::{self, DgrmCounters, DgrmGlobals, NetSys, PACKET_BUFFER_SIZE};
use quake_types::net::{NetLanDriver, QSockAddr, QSocket, SizeBuf, SysSocket};

/// keep in sync with net_dgrm_glue.c (maps to the SZ_GetSpace Host_Error)
const RUST_DGRM_NET_MESSAGE_OVERFLOW: c_int = -2;

extern "C" {
    /// net_bsd.c / net_win.c; not bindgen-reachable (ADR-011) -- indexed
    /// through the first element like C's `net_landrivers[n]`
    static mut net_landrivers: NetLanDriver;
}

fn landriver(idx: c_int) -> *mut NetLanDriver {
    // SAFETY: idx is a live qsocket's landriver index into the C array
    unsafe { (&raw mut net_landrivers).add(idx as usize) }
}

/// `sfunc` + console over the C landriver vtable; prints are deferred
struct EngineSys {
    landriver: c_int,
    prints: Vec<(bool, String)>,
}

impl NetSys for EngineSys {
    fn read(&mut self, socket: SysSocket, buf: &mut [u8]) -> (i32, QSockAddr) {
        // SAFETY: vtable slot installed by net_bsd/net_win; buf is the
        // packetBuffer scratch borrow, C writes only through this pointer
        unsafe {
            let ld = landriver(self.landriver);
            let mut addr: QSockAddr = core::mem::zeroed();
            let n = ((*ld).read.expect("landriver Read"))(
                socket,
                buf.as_mut_ptr(),
                buf.len() as c_int,
                &mut addr,
            );
            (n, addr)
        }
    }
    fn write(&mut self, socket: SysSocket, buf: &[u8], addr: &QSockAddr) -> i32 {
        // SAFETY: see read; C treats buf as const despite the mut pointer
        unsafe {
            let ld = landriver(self.landriver);
            let mut a = *addr;
            ((*ld).write.expect("landriver Write"))(
                socket,
                buf.as_ptr().cast_mut(),
                buf.len() as c_int,
                &mut a,
            )
        }
    }
    fn addr_compare(&mut self, a: &QSockAddr, b: &QSockAddr) -> i32 {
        // SAFETY: see read
        unsafe {
            let ld = landriver(self.landriver);
            let mut a2 = *a;
            let mut b2 = *b;
            ((*ld).addr_compare.expect("landriver AddrCompare"))(&mut a2, &mut b2)
        }
    }
    fn addr_to_string(&mut self, addr: &QSockAddr) -> String {
        // SAFETY: see read; the C driver returns a static buffer, copied out
        // immediately
        unsafe {
            let ld = landriver(self.landriver);
            let mut a = *addr;
            let p = ((*ld).addr_to_string.expect("landriver AddrToString"))(&mut a, false);
            if p.is_null() {
                String::new()
            } else {
                CStr::from_ptr(p).to_string_lossy().into_owned()
            }
        }
    }
    fn print(&mut self, msg: &str) {
        self.prints.push((false, msg.to_owned()));
    }
    fn dprint(&mut self, msg: &str) {
        self.prints.push((true, msg.to_owned()));
    }
}

/// Marshals the C-owned ambient state around one rel-layer call: counters
/// in/out, direct borrows of net_message and the packetBuffer scratch, and
/// the deferred console drain.
///
/// # Safety
/// Single-threaded host frame; `sock` is a live qsocket.
unsafe fn with_dgrm<R>(
    sock: *mut QSocket,
    f: impl FnOnce(&mut EngineSys, &mut QSocket, &mut DgrmGlobals, &mut [u8]) -> R,
) -> R {
    // SAFETY: caller contract; all globals are host-thread-only
    unsafe {
        let s = &mut *sock;
        let mut counters = DgrmCounters {
            packets_sent: c::packetsSent,
            packets_resent: c::packetsReSent,
            packets_received: c::packetsReceived,
            received_duplicate_count: c::receivedDuplicateCount,
            short_packet_count: c::shortPacketCount,
            dropped_datagrams: c::droppedDatagrams,
        };
        let mut mr = c::messagesReceived;
        let mut umr = c::unreliableMessagesReceived;
        let nm = &raw mut c::net_message;
        let nm_data = core::slice::from_raw_parts_mut((*nm).data, (*nm).maxsize as usize);
        let scratch = core::slice::from_raw_parts_mut(
            (&raw mut c::packetBuffer).cast::<u8>(),
            PACKET_BUFFER_SIZE,
        );
        let mut sys = EngineSys {
            landriver: s.landriver,
            prints: Vec::new(),
        };
        let r = {
            let mut g = DgrmGlobals {
                net_time: c::net_time,
                counters: &mut counters,
                messages_received: &mut mr,
                unreliable_messages_received: &mut umr,
                net_message: nm_data,
                net_message_cursize: &mut (*nm).cursize,
                net_message_maxsize: (*nm).maxsize,
            };
            f(&mut sys, s, &mut g, scratch)
        };
        c::packetsSent = counters.packets_sent;
        c::packetsReSent = counters.packets_resent;
        c::packetsReceived = counters.packets_received;
        c::receivedDuplicateCount = counters.received_duplicate_count;
        c::shortPacketCount = counters.short_packet_count;
        c::droppedDatagrams = counters.dropped_datagrams;
        c::messagesReceived = mr;
        c::unreliableMessagesReceived = umr;
        // all C-memory borrows have ended; drain the console backlog
        for (dev, text) in sys.prints {
            let mut bytes = text.into_bytes();
            bytes.push(0);
            if let Ok(cs) = CStr::from_bytes_with_nul(&bytes) {
                if dev {
                    c::Con_DPrintf(c"%s".as_ptr(), cs.as_ptr());
                } else {
                    c::Con_Printf(c"%s".as_ptr(), cs.as_ptr());
                }
            }
        }
        r
    }
}

/// # Safety
/// `sock` live qsocket; `data` a live sizebuf pre-validated by the glue
/// (cursize in 1..=NET_MAXMESSAGE); single-threaded host frame.
#[no_mangle]
pub unsafe extern "C" fn rust_dgrm_SendMessage(sock: *mut QSocket, data: *mut SizeBuf) -> c_int {
    // SAFETY: caller contract
    unsafe {
        let payload = core::slice::from_raw_parts((*data).data, (*data).cursize as usize);
        with_dgrm(sock, |sys, s, g, pkt| {
            match dgrm::send_message(sys, s, g, pkt, payload) {
                Ok(v) => v,
                // unreachable: the glue's C frame pre-validates (ADR-009)
                Err(_) => {
                    c::Sys_Error(c"rust_dgrm_SendMessage: unvalidated message".as_ptr());
                }
            }
        })
    }
}

/// # Safety
/// As rust_dgrm_SendMessage (cursize in 1..=MAX_DATAGRAM).
#[no_mangle]
pub unsafe extern "C" fn rust_dgrm_SendUnreliableMessage(
    sock: *mut QSocket,
    data: *mut SizeBuf,
) -> c_int {
    // SAFETY: caller contract
    unsafe {
        let payload = core::slice::from_raw_parts((*data).data, (*data).cursize as usize);
        with_dgrm(sock, |sys, s, g, pkt| {
            match dgrm::send_unreliable_message(sys, s, g, pkt, payload) {
                Ok(v) => v,
                Err(_) => {
                    c::Sys_Error(c"rust_dgrm_SendUnreliableMessage: unvalidated message".as_ptr());
                }
            }
        })
    }
}

/// # Safety
/// `sock` live qsocket; single-threaded host frame.
#[no_mangle]
pub unsafe extern "C" fn rust_dgrm_SendMessageNext(sock: *mut QSocket) -> c_int {
    // SAFETY: caller contract
    unsafe { with_dgrm(sock, dgrm::send_message_next) }
}

/// # Safety
/// `sock` live qsocket; single-threaded host frame.
#[no_mangle]
pub unsafe extern "C" fn rust_dgrm_ReSendMessage(sock: *mut QSocket) -> c_int {
    // SAFETY: caller contract
    unsafe { with_dgrm(sock, dgrm::resend_message) }
}

/// # Safety
/// `sock` live qsocket; single-threaded host frame.
#[no_mangle]
pub unsafe extern "C" fn rust_dgrm_CanSendMessage(sock: *mut QSocket) -> bool {
    // SAFETY: caller contract
    unsafe { with_dgrm(sock, dgrm::can_send_message) }
}

#[no_mangle]
pub extern "C" fn rust_dgrm_CanSendUnreliableMessage(_sock: *mut QSocket) -> bool {
    dgrm::can_send_unreliable_message()
}

/// # Safety
/// `sock` live qsocket; the packet bytes are already in the C packetBuffer
/// scratch (net_dgrm.c's GetAnyMessage read them there); single-threaded.
#[no_mangle]
pub unsafe extern "C" fn rust_dgrm_ProcessPacket(length: c_uint, sock: *mut QSocket) -> bool {
    // SAFETY: caller contract
    unsafe {
        with_dgrm(sock, |sys, s, g, pkt| {
            dgrm::process_packet(sys, s, g, pkt, length)
        })
    }
}

/// Returns the C codes (-1/0/1/2) or RUST_DGRM_NET_MESSAGE_OVERFLOW, which
/// the glue re-raises as the SZ_GetSpace Host_Error (ADR-009).
///
/// # Safety
/// `sock` live qsocket; single-threaded host frame.
#[no_mangle]
pub unsafe extern "C" fn rust_dgrm_GetMessage(sock: *mut QSocket) -> c_int {
    // SAFETY: caller contract
    unsafe {
        let r = with_dgrm(sock, dgrm::get_message);
        if r == dgrm::GET_MESSAGE_NET_MESSAGE_OVERFLOW {
            RUST_DGRM_NET_MESSAGE_OVERFLOW
        } else {
            r
        }
    }
}
