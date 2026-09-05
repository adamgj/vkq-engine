//! Networking ABI mirrors (`Quake/common.h` sizebuf, `Quake/net.h`,
//! `Quake/net_defs.h`). Compat-critical: under `-Duse_rust_net` the Rust wire
//! layer reads and writes the C-owned `net_message` sizebuf and `qsocket_t`
//! pool, and Rust driver functions are registered into the C
//! `net_drivers[]`/`net_landrivers[]` vtables, so layout drift is silent
//! memory corruption. The net headers are not bindgen-clean roots
//! (`net_sys.h` pulls system socket headers), hence hand-written mirrors per
//! ADR-011, verified per-platform by `quake-ctest/tests/net_abi.rs` against
//! the engine's own headers.

use core::ffi::{c_char, c_int, c_void};

pub const NET_NAMELEN: usize = 64;
pub const NET_MAXMESSAGE: usize = 64000;
/// quakedef.h
pub const MAX_MSGLEN: usize = 64000;
/// quakedef.h
pub const MAX_DATAGRAM: usize = 64000;
/// quakedef.h: actual limit for unreliable messages to nonlocal clients
pub const DATAGRAM_MTU: usize = 1400;
/// 2 * sizeof(unsigned int)
pub const NET_HEADERSIZE: usize = 8;
pub const NET_DATAGRAMSIZE: usize = MAX_DATAGRAM + NET_HEADERSIZE;

pub const NETFLAG_LENGTH_MASK: u32 = 0x0000ffff;
pub const NETFLAG_DATA: u32 = 0x00010000;
pub const NETFLAG_ACK: u32 = 0x00020000;
pub const NETFLAG_NAK: u32 = 0x00040000;
pub const NETFLAG_EOM: u32 = 0x00080000;
pub const NETFLAG_UNRELIABLE: u32 = 0x00100000;
pub const NETFLAG_CTL: u32 = 0x80000000;

pub const NET_LOOPBACKBUFFERS: usize = 5;
pub const NET_LOOPBACKHEADERSIZE: usize = 4;
pub const NET_PROTOCOL_VERSION: u8 = 3;

pub const CCREQ_CONNECT: u8 = 0x01;
pub const CCREQ_SERVER_INFO: u8 = 0x02;
pub const CCREQ_PLAYER_INFO: u8 = 0x03;
pub const CCREQ_RULE_INFO: u8 = 0x04;
pub const CCREQ_RCON: u8 = 0x05;

pub const CCREP_ACCEPT: u8 = 0x81;
pub const CCREP_REJECT: u8 = 0x82;
pub const CCREP_SERVER_INFO: u8 = 0x83;
pub const CCREP_PLAYER_INFO: u8 = 0x84;
pub const CCREP_RULE_INFO: u8 = 0x85;
pub const CCREP_RCON: u8 = 0x86;

pub const HOSTCACHESIZE: usize = 128;

/// `qboolean` is C11 `_Bool` (q_types.h)
pub type QBoolean = bool;

/// `sys_socket_t` (net_sys.h): `int` on unix, `SOCKET` (`UINT_PTR`) on Windows
#[cfg(not(windows))]
pub type SysSocket = c_int;
#[cfg(windows)]
pub type SysSocket = usize;

/// `qhostaddr_t` (net.h)
pub type QHostAddr = [c_char; NET_NAMELEN];

/// `sizebuf_t` (common.h)
#[repr(C)]
#[derive(Debug)]
pub struct SizeBuf {
    /// if false, overflow is a Sys_Error/Host_Error
    pub allowoverflow: QBoolean,
    /// set to true if the buffer size failed
    pub overflowed: QBoolean,
    pub data: *mut u8,
    pub maxsize: c_int,
    pub cursize: c_int,
}

/// `struct qsockaddr` (net_defs.h): a 64-byte blob punned to
/// `sockaddr_in`/`sockaddr_in6` by the landrivers. The header layout follows
/// the platform's `struct sockaddr`: BSD-style (`sa_len` + u8 family) where
/// `HAVE_SA_LEN` is defined (macOS/BSD), else a 16-bit family.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct QSockAddr {
    #[cfg(any(
        target_os = "macos",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "dragonfly"
    ))]
    pub qsa_len: u8,
    #[cfg(any(
        target_os = "macos",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "dragonfly"
    ))]
    pub qsa_family: u8,
    #[cfg(not(any(
        target_os = "macos",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "dragonfly"
    )))]
    pub qsa_family: i16,
    pub qsa_data: [u8; 62],
}

impl QSockAddr {
    /// an all-zero blob (C `memset (&addr, 0, sizeof (addr))`)
    pub const fn zeroed() -> Self {
        QSockAddr {
            #[cfg(any(
                target_os = "macos",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd",
                target_os = "dragonfly"
            ))]
            qsa_len: 0,
            #[cfg(any(
                target_os = "macos",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd",
                target_os = "dragonfly"
            ))]
            qsa_family: 0,
            #[cfg(not(any(
                target_os = "macos",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd",
                target_os = "dragonfly"
            )))]
            qsa_family: 0,
            qsa_data: [0; 62],
        }
    }
}

/// `qsocket_t` (net_defs.h)
#[repr(C)]
pub struct QSocket {
    pub next: *mut QSocket,
    pub connecttime: f64,
    pub last_message_time: f64,
    pub last_send_time: f64,

    /// qsocket is emulated by the network layer (closing will not close any
    /// system sockets)
    pub isvirtual: QBoolean,
    pub disconnected: QBoolean,
    pub can_send: QBoolean,
    pub send_next: QBoolean,

    pub driver: c_int,
    pub landriver: c_int,
    pub socket: SysSocket,
    pub driverdata: *mut c_void,

    pub ack_sequence: u32,
    pub send_sequence: u32,
    pub unreliable_send_sequence: u32,
    pub send_message_length: c_int,
    pub send_message: [u8; NET_MAXMESSAGE],

    pub receive_sequence: u32,
    pub unreliable_receive_sequence: u32,
    pub receive_message_length: c_int,
    pub receive_message: [u8; NET_MAXMESSAGE * NET_LOOPBACKBUFFERS + NET_LOOPBACKHEADERSIZE],

    pub addr: QSockAddr,
    /// lazy address string
    pub trueaddress: [c_char; NET_NAMELEN],
    /// addresses for this player that may be displayed publically
    pub maskedaddress: [c_char; NET_NAMELEN],

    /// 1 if we're trying, 2 if the server acked
    pub proquake_angle_hack: QBoolean,
    /// 32000 for local, 1442 for 666, 1024 for 15; for reliable fragments
    pub max_datagram: c_int,
    /// don't change the mtu if we're resending; that would confuse the peer
    pub pending_max_datagram: c_int,
}

/// `net_landriver_t` (net_defs.h). Field order is ABI: Rust landrivers are
/// installed by overwriting individual function-pointer slots from C.
#[repr(C)]
pub struct NetLanDriver {
    pub name: *const c_char,
    pub initialized: QBoolean,
    pub control_sock: SysSocket,
    pub init: Option<unsafe extern "C" fn() -> SysSocket>,
    pub shutdown: Option<unsafe extern "C" fn()>,
    pub listen: Option<unsafe extern "C" fn(state: QBoolean) -> SysSocket>,
    pub query_addresses:
        Option<unsafe extern "C" fn(addresses: *mut QHostAddr, maxaddresses: c_int) -> c_int>,
    pub open_socket: Option<unsafe extern "C" fn(port: c_int) -> SysSocket>,
    pub close_socket: Option<unsafe extern "C" fn(socketid: SysSocket) -> c_int>,
    pub connect: Option<unsafe extern "C" fn(socketid: SysSocket, addr: *mut QSockAddr) -> c_int>,
    pub check_new_connections: Option<unsafe extern "C" fn() -> SysSocket>,
    pub read: Option<
        unsafe extern "C" fn(
            socketid: SysSocket,
            buf: *mut u8,
            len: c_int,
            addr: *mut QSockAddr,
        ) -> c_int,
    >,
    pub write: Option<
        unsafe extern "C" fn(
            socketid: SysSocket,
            buf: *mut u8,
            len: c_int,
            addr: *mut QSockAddr,
        ) -> c_int,
    >,
    pub broadcast:
        Option<unsafe extern "C" fn(socketid: SysSocket, buf: *mut u8, len: c_int) -> c_int>,
    pub addr_to_string:
        Option<unsafe extern "C" fn(addr: *mut QSockAddr, masked: QBoolean) -> *const c_char>,
    pub string_to_addr:
        Option<unsafe extern "C" fn(string: *const c_char, addr: *mut QSockAddr) -> c_int>,
    pub get_socket_addr:
        Option<unsafe extern "C" fn(socketid: SysSocket, addr: *mut QSockAddr) -> c_int>,
    pub get_name_from_addr:
        Option<unsafe extern "C" fn(addr: *mut QSockAddr, name: *mut c_char) -> c_int>,
    pub get_addr_from_name:
        Option<unsafe extern "C" fn(name: *const c_char, addr: *mut QSockAddr) -> c_int>,
    pub addr_compare:
        Option<unsafe extern "C" fn(addr1: *mut QSockAddr, addr2: *mut QSockAddr) -> c_int>,
    pub get_socket_port: Option<unsafe extern "C" fn(addr: *mut QSockAddr) -> c_int>,
    pub set_socket_port: Option<unsafe extern "C" fn(addr: *mut QSockAddr, port: c_int) -> c_int>,
    pub listening_sock: SysSocket,
}

/// `hostcache_t` (net_defs.h)
#[repr(C)]
#[derive(Clone, Copy)]
pub struct HostCache {
    pub name: [c_char; 64],
    pub map: [c_char; 16],
    pub gamedir: [c_char; 16],
    pub cname: [c_char; NET_NAMELEN],
    pub users: c_int,
    pub maxusers: c_int,
    pub driver: c_int,
    pub ldriver: c_int,
    pub addr: QSockAddr,
}

/// `PollProcedure` (net_defs.h).
///
/// Phase 7 M9c ported `NET_Poll` / `SchedulePollProcedure`, so this is a
/// live mirror now. The nodes stay caller-owned; only the list head is
/// Rust state (ADR-007). Layout is pinned by `abi_probe.c` and
/// `tests/net_abi.rs` on all three CI platforms.
#[repr(C)]
pub struct PollProcedure {
    pub next: *mut PollProcedure,
    pub next_time: f64,
    pub procedure: Option<unsafe extern "C" fn(arg: *mut c_void)>,
    pub arg: *mut c_void,
}

/// `net_driver_t` (net_defs.h). Field order is ABI (see `NetLanDriver`).
/// Loop driver must always be registered first (`IS_LOOP_DRIVER(0)`).
#[repr(C)]
pub struct NetDriver {
    pub name: *const c_char,
    pub initialized: QBoolean,
    pub init: Option<unsafe extern "C" fn() -> c_int>,
    pub listen: Option<unsafe extern "C" fn(state: QBoolean)>,
    pub query_addresses:
        Option<unsafe extern "C" fn(addresses: *mut QHostAddr, maxaddresses: c_int) -> c_int>,
    pub search_for_hosts: Option<unsafe extern "C" fn(xmit: QBoolean) -> QBoolean>,
    pub connect: Option<unsafe extern "C" fn(host: *const c_char) -> *mut QSocket>,
    pub check_new_connections: Option<unsafe extern "C" fn() -> *mut QSocket>,
    pub qget_any_message: Option<unsafe extern "C" fn() -> *mut QSocket>,
    pub qget_message: Option<unsafe extern "C" fn(sock: *mut QSocket) -> c_int>,
    pub qsend_message:
        Option<unsafe extern "C" fn(sock: *mut QSocket, data: *mut SizeBuf) -> c_int>,
    pub send_unreliable_message:
        Option<unsafe extern "C" fn(sock: *mut QSocket, data: *mut SizeBuf) -> c_int>,
    pub can_send_message: Option<unsafe extern "C" fn(sock: *mut QSocket) -> QBoolean>,
    pub can_send_unreliable_message: Option<unsafe extern "C" fn(sock: *mut QSocket) -> QBoolean>,
    pub close: Option<unsafe extern "C" fn(sock: *mut QSocket)>,
    pub shutdown: Option<unsafe extern "C" fn()>,
}

// Pointer-width-independent pins
const _: () = {
    assert!(core::mem::size_of::<QSockAddr>() == 64);
    assert!(core::mem::offset_of!(QSockAddr, qsa_data) == 2);
    assert!(NET_MAXMESSAGE & (NETFLAG_LENGTH_MASK as usize) == NET_MAXMESSAGE);
    assert!(core::mem::size_of::<HostCache>() == 64 + 16 + 16 + 64 + 16 + 64);
    assert!(core::mem::offset_of!(HostCache, addr) == 176);
};

// Layout pins for the 64-bit targets (the whole current CI matrix). These are
// const asserts, so they would fail COMPILATION rather than degrade on a
// 32-bit target -- which upstream QuakeSpasm still builds and common.make
// does not forbid -- hence the cfg. The real per-platform gate against the
// engine's own headers is tests/net_abi.rs, which runs everywhere.
#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(core::mem::size_of::<SizeBuf>() == 24);
    assert!(core::mem::offset_of!(SizeBuf, data) == 8);
    assert!(core::mem::offset_of!(SizeBuf, cursize) == 20);
    assert!(core::mem::offset_of!(QSocket, isvirtual) == 32);
    assert!(core::mem::offset_of!(QSocket, driver) == 36);
    // fields past `socket` shift between unix (int) and windows (UINT_PTR);
    // those offsets are pinned per-platform by net_abi.rs only
};
