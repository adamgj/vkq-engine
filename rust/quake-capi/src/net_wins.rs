//! Phase 9 M2: the Rust Winsock UDP landriver (net_wins.c), installed into
//! `net_landrivers[]` by net_win.c under `USE_RUST_NET`. Windows only; the
//! net_udp.c personality is `net_udp.rs`. The two drivers share their
//! vtable slot names (`rust_udp*`) so the header block in cbindgen.toml
//! stays one list; the per-family slots net_wins.c splits and net_udp.c
//! does not (`GetNameFromAddr`) are exported as `rust_udp4_*`/`rust_udp6_*`
//! here.
//!
//! The file statics (accept/control/broadcast sockets, the bind/my
//! addresses, `winsock_initialized`) become Rust module state -- never
//! C-visible (ADR-007). Address logic lives in `quake_net::udp` (pure; the
//! `*_wins` variants where net_wins.c differs from net_udp.c), the Winsock
//! calls in `quake_net::udp::sys` (the ADR-004 island, `sys::windows`);
//! this module owns the engine globals (net_hostport, my_ipv*_address,
//! ipv*Available, com_argv) and the console/Sys_Error surfaces.
//!
//! Con_SafePrintf/Con_Warning/Con_DPrintf are called at the same points as
//! C; no C-memory borrows are held across any of those calls.
//!
//! The IPv6 half is a faithful port of net_wins.c's `#ifdef IPPROTO_IPV6`
//! code, but whether it is *installed* is net_win.c's decision under the
//! same guard: the Windows SDK defines IPPROTO_IPV6 as an enumerator, not a
//! macro, so the MSVC/clang-cl oracle has no IPv6 landriver and the
//! `USE_RUST_NET` table omits it there too. The `rust_udp6_*` exports are
//! then unreferenced. Fixing that inherited build bug is a post-deletion
//! (Phase 10) behaviour change, not a port-time one.

#![cfg(windows)]

use core::ffi::{c_char, c_int, CStr};

use quake_c_sys as c;
use quake_net::udp::{self, sys, AF_INET, AF_INET6, MAXHOSTNAMELEN};
use quake_types::net::{QHostAddr, QSockAddr, SysSocket, NET_NAMELEN};

const INVALID: SysSocket = sys::INVALID;

static mut ACCEPT4: SysSocket = INVALID;
static mut CONTROL4: SysSocket = 0;
static mut BROADCAST4: SysSocket = INVALID;
static mut BROADCAST_ADDR4: QSockAddr = QSockAddr::zeroed();
/// network byte order, like the C `in_addr_t myAddrv4, bindAddrv4`
static mut MY_ADDR4: u32 = 0;
static mut BIND_ADDR4: u32 = 0;
static mut ACCEPT6: SysSocket = INVALID;
static mut CONTROL6: SysSocket = 0;
static mut BROADCAST_ADDR6: QSockAddr = QSockAddr::zeroed();
static mut MY_ADDR6: [u8; 16] = [0; 16];
static mut BIND_ADDR6: [u8; 16] = [0; 16];
/// `int winsock_initialized` -- the WSAStartup/WSACleanup refcount shared
/// by the two drivers
static mut WINSOCK_INITIALIZED: i32 = 0;

fn cbuf(text: &str) -> Vec<u8> {
    let mut b = text.as_bytes().to_vec();
    b.push(0);
    b
}

fn safe_print(text: &str) {
    let b = cbuf(text);
    // SAFETY: b is NUL-terminated; Con_SafePrintf never redraws
    unsafe {
        c::Con_SafePrintf(c"%s".as_ptr(), b.as_ptr());
    }
}

fn warning(text: &str) {
    let b = cbuf(text);
    // SAFETY: b is NUL-terminated
    unsafe {
        c::Con_Warning(c"%s".as_ptr(), b.as_ptr());
    }
}

fn dprint(text: &str) {
    let b = cbuf(text);
    // SAFETY: b is NUL-terminated
    unsafe {
        c::Con_DPrintf(c"%s".as_ptr(), b.as_ptr());
    }
}

fn sys_error(text: &str) -> ! {
    let b = cbuf(text);
    // SAFETY: b is NUL-terminated
    unsafe { c::Sys_Error(c"%s".as_ptr(), b.as_ptr()) }
}

fn check_parm(p: &CStr) -> c_int {
    // SAFETY: COM_CheckParm reads a NUL-terminated string
    unsafe { c::COM_CheckParm(p.as_ptr()) }
}

/// `com_argv[i]` when `i < com_argc - 1` (the `-ip`/`-ip6` value slot)
fn argv_after(i: c_int) -> Option<&'static [u8]> {
    // SAFETY: engine globals, host-thread-only; i + 1 < argc so the slot
    // holds one of the engine's own NUL-terminated argument strings
    unsafe {
        let argc = core::ptr::addr_of!(c::com_argc).read();
        if i < argc - 1 {
            let argv = core::ptr::addr_of!(c::com_argv).read();
            Some(CStr::from_ptr(*argv.add(i as usize + 1)).to_bytes())
        } else {
            None
        }
    }
}

/// `strcpy (my_ip*_address, src)` / `q_strlcpy (.., NET_NAMELEN)`: bounded
/// either way so the FFI write can never outgrow the engine's buffer
fn store_c_string(dst: &mut [c_char; NET_NAMELEN], src: &[u8]) {
    let n = src.len().min(NET_NAMELEN - 1);
    for (i, &ch) in src[..n].iter().enumerate() {
        dst[i] = ch as c_char;
    }
    dst[n] = 0;
}

fn c_string_is_empty(s: &[c_char; NET_NAMELEN]) -> bool {
    s[0] == 0
}

/// `"%ld.%ld.%ld.%ld"` of a host-order address (WINIPv4_GetLocalAddress /
/// WINIPv4_GetAddresses)
fn dotted(host_order: u32) -> String {
    format!(
        "{}.{}.{}.{}",
        (host_order >> 24) & 0xff,
        (host_order >> 16) & 0xff,
        (host_order >> 8) & 0xff,
        host_order & 0xff
    )
}

/// `WINIPv4_GetLocalAddress`
///
/// # Safety
/// Single-threaded host frame.
unsafe fn get_local_address4() {
    // SAFETY: module statics + engine globals are host-thread-only
    unsafe {
        if MY_ADDR4 != sys::INADDR_ANY {
            return;
        }
        let buff = match sys::gethostname(MAXHOSTNAMELEN) {
            Err(err) => {
                safe_print(&format!(
                    "WINIPV4_GetLocalAddress: gethostname failed ({})\n",
                    sys::strerror(err)
                ));
                return;
            }
            Ok(b) => b,
        };
        match sys::host_by_name(&buff) {
            Err(err) => {
                safe_print(&format!(
                    "WINIPV4_GetLocalAddress: gethostbyname failed ({})\n",
                    sys::strerror(err)
                ));
            }
            Ok(h) => {
                // `*(in_addr_t *)local->h_addr_list[0]`: gethostbyname never
                // returns an empty list, so the C load is always in bounds
                if let Some(&a) = h.addrs.first() {
                    MY_ADDR4 = a;
                    store_c_string(
                        &mut *core::ptr::addr_of_mut!(c::my_ipv4_address),
                        dotted(u32::from_be(a)).as_bytes(),
                    );
                }
            }
        }
    }
}

/// `WINIPv6_GetLocalAddress`
///
/// # Safety
/// Single-threaded host frame.
unsafe fn get_local_address6() {
    // SAFETY: engine globals are host-thread-only
    unsafe {
        let buff = match sys::gethostname(MAXHOSTNAMELEN) {
            Err(err) => {
                safe_print(&format!(
                    "WINIPv6_GetLocalAddress: gethostname failed ({})\n",
                    sys::strerror(err)
                ));
                return;
            }
            Ok(b) => b,
        };
        match sys::getaddrinfo_first6(&buff) {
            Ok(local) => {
                let mut s = udp::addr_to_string_wins(&local, false);
                if s.len() > 2 && s.ends_with(":0") {
                    s.truncate(s.len() - 2);
                }
                store_c_string(
                    &mut *core::ptr::addr_of_mut!(c::my_ipv6_address),
                    s.as_bytes(),
                );
            }
            Err(err) => {
                safe_print(&format!(
                    "WINIPv6_GetLocalAddress: gethostbyname failed ({})\n",
                    sys::strerror(err)
                ));
            }
        }
    }
}

/// `WSAStartup` guarded by `winsock_initialized`; false when it failed
/// (the "Winsock initialization failed" print included)
unsafe fn winsock_acquire(major: u8, minor: u8) -> bool {
    // SAFETY: module static, host-thread-only
    unsafe {
        if WINSOCK_INITIALIZED == 0 {
            if let Err(err) = sys::wsa_startup(major, minor) {
                safe_print(&format!(
                    "Winsock initialization failed ({})\n",
                    sys::strerror(err)
                ));
                return false;
            }
        }
        WINSOCK_INITIALIZED += 1;
        true
    }
}

/// `if (--winsock_initialized == 0) WSACleanup ();`
unsafe fn winsock_release() {
    // SAFETY: module static, host-thread-only
    unsafe {
        WINSOCK_INITIALIZED -= 1;
        if WINSOCK_INITIALIZED == 0 {
            sys::wsa_cleanup();
        }
    }
}

/// the shared tail of `WINIPv4_OpenSocket`/`WINIPv6_OpenSocket` (prints
/// included): the post-init bind failure warns and LEAKS the socket like
/// C; every other failure takes the `ErrorReturn` label
fn open_socket_fail(err: &sys::OpenError, available: bool) -> SysSocket {
    match err {
        sys::OpenError::Socket(e) => {
            safe_print(&format!("WINS_OpenSocket: {}\n", sys::strerror(*e)));
            INVALID
        }
        sys::OpenError::Bind(e, _sock, address) if available => {
            warning(&format!(
                "Unable to bind to {} ({})\n",
                udp::addr_to_string_wins(address, false),
                sys::strerror(*e)
            ));
            INVALID
        }
        sys::OpenError::Ioctl(e, sock) | sys::OpenError::Bind(e, sock, _) => {
            safe_print(&format!("WINS_OpenSocket: {}\n", sys::strerror(*e)));
            sys::close_socket(*sock);
            INVALID
        }
    }
}

/// `WINIPv4_OpenSocket`
///
/// # Safety
/// Single-threaded host frame.
unsafe fn open_socket4(port: c_int) -> SysSocket {
    // SAFETY: module statics + engine globals are host-thread-only
    unsafe {
        match sys::open_socket4(BIND_ADDR4, port as u16) {
            Ok(s) => s,
            Err(err) => open_socket_fail(&err, c::ipv4Available),
        }
    }
}

/// `WINIPv6_OpenSocket`
///
/// # Safety
/// Single-threaded host frame.
unsafe fn open_socket6(port: c_int) -> SysSocket {
    // SAFETY: module statics + engine globals are host-thread-only
    unsafe {
        let bind = *core::ptr::addr_of!(BIND_ADDR6);
        let group = v6_bytes(&*core::ptr::addr_of!(BROADCAST_ADDR6));
        match sys::open_socket6(bind, port as u16, group) {
            Ok(s) => s,
            Err(err) => open_socket_fail(&err, c::ipv6Available),
        }
    }
}

fn v6_bytes(addr: &QSockAddr) -> [u8; 16] {
    let mut out = [0u8; 16];
    out.copy_from_slice(&addr.qsa_data[6..22]);
    out
}

fn close_socket_impl(s: SysSocket) -> c_int {
    // SAFETY: module statics are host-thread-only
    unsafe {
        if s == BROADCAST4 {
            BROADCAST4 = INVALID;
        }
    }
    sys::close_socket(s)
}

/// `WINIPv4_Init`
///
/// # Safety
/// Single-threaded engine init.
#[no_mangle]
pub unsafe extern "C" fn rust_udp4_Init() -> SysSocket {
    // SAFETY: module statics + engine globals are host-thread-only
    unsafe {
        if check_parm(c"-noudp") != 0 || check_parm(c"-noudp4") != 0 {
            return INVALID;
        }

        if !winsock_acquire(1, 1) {
            return INVALID;
        }

        // determine my name & address (the name itself is unused here)
        if let Err(err) = sys::gethostname(MAXHOSTNAMELEN) {
            safe_print(&format!(
                "WINS_Init: gethostname failed ({})\n",
                sys::strerror(err)
            ));
        }

        let i = check_parm(c"-ip");
        if i != 0 {
            match argv_after(i) {
                Some(arg) => {
                    BIND_ADDR4 = sys::inet_addr(arg);
                    if BIND_ADDR4 == sys::INADDR_NONE {
                        sys_error(&format!(
                            "{} is not a valid IP address",
                            String::from_utf8_lossy(arg)
                        ));
                    }
                    store_c_string(&mut *core::ptr::addr_of_mut!(c::my_ipv4_address), arg);
                }
                None => sys_error("NET_Init: you must specify an IP address after -ip"),
            }
        } else {
            BIND_ADDR4 = sys::INADDR_ANY;
            store_c_string(
                &mut *core::ptr::addr_of_mut!(c::my_ipv4_address),
                b"INADDR_ANY",
            );
        }

        MY_ADDR4 = BIND_ADDR4;

        CONTROL4 = open_socket4(0);
        if CONTROL4 == INVALID {
            safe_print("WINS_Init: Unable to open control socket, UDP disabled\n");
            winsock_release();
            return INVALID;
        }

        let mut b = QSockAddr::zeroed();
        udp::set_family(&mut b, AF_INET);
        b.qsa_data[2..6].copy_from_slice(&sys::INADDR_BROADCAST.to_ne_bytes());
        b.qsa_data[0..2].copy_from_slice(&(c::net_hostport as u16).to_be_bytes());
        BROADCAST_ADDR4 = b;

        safe_print("IPv4 UDP Initialized\n");
        c::ipv4Available = true;

        CONTROL4
    }
}

/// `WINIPv6_Init`
///
/// # Safety
/// Single-threaded engine init.
#[no_mangle]
pub unsafe extern "C" fn rust_udp6_Init() -> SysSocket {
    // SAFETY: see rust_udp4_Init
    unsafe {
        if check_parm(c"-noudp") != 0 || check_parm(c"-noudp6") != 0 {
            return INVALID;
        }

        // (C resolves getaddrinfo/freeaddrinfo from ws2_32.dll at runtime
        // and bails with "Winsock lacks getaddrinfo" when absent; the Rust
        // arm links them directly, so that branch has no counterpart)

        if !winsock_acquire(2, 2) {
            return INVALID;
        }

        // determine my name & address (the name itself is unused here)
        if let Err(err) = sys::gethostname(MAXHOSTNAMELEN) {
            safe_print(&format!(
                "WINIPv6_Init: gethostname failed ({})\n",
                sys::strerror(err)
            ));
        }

        let i = check_parm(c"-ip6");
        if i != 0 {
            match argv_after(i) {
                Some(arg) => {
                    // COMPAT: C resolves straight into the 16-byte
                    // `bindAddrv6` through a `struct qsockaddr *` cast, so a
                    // valid address stores the sockaddr_in6 header bytes as
                    // the bind address and overruns into the neighbouring
                    // statics. The Rust arm keeps the resolved sin6_addr
                    // (what `-ip6` is documented to do); recorded in the
                    // Phase 9 task plan amendment log.
                    let mut resolved = QSockAddr::zeroed();
                    let mut carg = arg.to_vec();
                    carg.push(0);
                    if rust_udp6_GetAddrFromName(carg.as_ptr().cast(), &mut resolved) != 0 {
                        sys_error(&format!(
                            "{} is not a valid IPv6 address",
                            String::from_utf8_lossy(arg)
                        ));
                    }
                    BIND_ADDR6 = v6_bytes(&resolved);
                    let my6 = &mut *core::ptr::addr_of_mut!(c::my_ipv6_address);
                    if c_string_is_empty(my6) {
                        store_c_string(my6, arg);
                    }
                }
                None => sys_error("WINIPv6_Init: you must specify an IP address after -ip"),
            }
        } else {
            BIND_ADDR6 = [0; 16];
            let my6 = &mut *core::ptr::addr_of_mut!(c::my_ipv6_address);
            if c_string_is_empty(my6) {
                store_c_string(my6, b"[::]");
                get_local_address6();
            }
        }

        MY_ADDR6 = BIND_ADDR6;

        CONTROL6 = open_socket6(0);
        if CONTROL6 == INVALID {
            safe_print("WINIPv6_Init: Unable to open control socket, UDP disabled\n");
            winsock_release();
            return INVALID;
        }

        let mut b = QSockAddr::zeroed();
        udp::set_family(&mut b, AF_INET6);
        b.qsa_data[6] = 0xff;
        b.qsa_data[7] = 0x03;
        b.qsa_data[21] = 0x01;
        b.qsa_data[0..2].copy_from_slice(&(c::net_hostport as u16).to_be_bytes());
        BROADCAST_ADDR6 = b;

        safe_print("IPv6 UDP Initialized\n");
        c::ipv6Available = true;

        CONTROL6
    }
}

/// `WINIPv4_Shutdown`
///
/// # Safety
/// Single-threaded engine shutdown.
#[no_mangle]
pub unsafe extern "C" fn rust_udp4_Shutdown() {
    // SAFETY: caller contract
    unsafe {
        rust_udp4_Listen(false);
        close_socket_impl(CONTROL4);
        winsock_release();
    }
}

/// `WINIPv6_Shutdown`
///
/// # Safety
/// Single-threaded engine shutdown.
#[no_mangle]
pub unsafe extern "C" fn rust_udp6_Shutdown() {
    // SAFETY: caller contract
    unsafe {
        rust_udp6_Listen(false);
        close_socket_impl(CONTROL6);
        winsock_release();
    }
}

/// `WINIPv4_Listen`: unlike net_udp.c a failed accept-socket open is not
/// fatal here (INVALID_SOCKET is returned)
///
/// # Safety
/// Single-threaded host frame.
#[no_mangle]
pub unsafe extern "C" fn rust_udp4_Listen(state: bool) -> SysSocket {
    // SAFETY: caller contract
    unsafe {
        if state {
            if ACCEPT4 != INVALID {
                return ACCEPT4;
            }
            get_local_address4();
            ACCEPT4 = open_socket4(c::net_hostport);
            return ACCEPT4;
        }
        if ACCEPT4 == INVALID {
            return INVALID;
        }
        close_socket_impl(ACCEPT4);
        ACCEPT4 = INVALID;
        INVALID
    }
}

/// `WINIPv6_Listen`
///
/// # Safety
/// Single-threaded host frame.
#[no_mangle]
pub unsafe extern "C" fn rust_udp6_Listen(state: bool) -> SysSocket {
    // SAFETY: caller contract
    unsafe {
        if state {
            if ACCEPT6 == INVALID {
                ACCEPT6 = open_socket6(c::net_hostport);
            }
        } else if ACCEPT6 != INVALID {
            close_socket_impl(ACCEPT6);
            ACCEPT6 = INVALID;
        }
        ACCEPT6
    }
}

/// `WINIPv4_OpenSocket` vtable slot
///
/// # Safety
/// Single-threaded host frame.
#[no_mangle]
pub unsafe extern "C" fn rust_udp4_OpenSocket(port: c_int) -> SysSocket {
    // SAFETY: caller contract
    unsafe { open_socket4(port) }
}

/// `WINIPv6_OpenSocket` vtable slot
///
/// # Safety
/// Single-threaded host frame.
#[no_mangle]
pub unsafe extern "C" fn rust_udp6_OpenSocket(port: c_int) -> SysSocket {
    // SAFETY: caller contract
    unsafe { open_socket6(port) }
}

/// `WINS_CloseSocket`
#[no_mangle]
pub extern "C" fn rust_udp_CloseSocket(socketid: SysSocket) -> c_int {
    close_socket_impl(socketid)
}

/// `WINS_Connect` (a no-op in the C driver)
#[no_mangle]
pub extern "C" fn rust_udp_Connect(_socketid: SysSocket, _addr: *mut QSockAddr) -> c_int {
    0
}

fn check_new_connections(accept: SysSocket) -> SysSocket {
    if accept == INVALID {
        return INVALID;
    }
    if sys::peek_has_data(accept) {
        return accept;
    }
    INVALID
}

/// `WINIPv4_CheckNewConnections`
///
/// # Safety
/// Single-threaded host frame.
#[no_mangle]
pub unsafe extern "C" fn rust_udp4_CheckNewConnections() -> SysSocket {
    // SAFETY: caller contract
    unsafe { check_new_connections(ACCEPT4) }
}

/// `WINIPv6_CheckNewConnections`
///
/// # Safety
/// Single-threaded host frame.
#[no_mangle]
pub unsafe extern "C" fn rust_udp6_CheckNewConnections() -> SysSocket {
    // SAFETY: caller contract
    unsafe { check_new_connections(ACCEPT6) }
}

/// `WINS_Read`
///
/// # Safety
/// `buf` has `len` bytes; `addr` is a live qsockaddr out-param.
#[no_mangle]
pub unsafe extern "C" fn rust_udp_Read(
    socketid: SysSocket,
    buf: *mut u8,
    len: c_int,
    addr: *mut QSockAddr,
) -> c_int {
    // SAFETY: caller contract
    unsafe {
        let slice = core::slice::from_raw_parts_mut(buf, len.max(0) as usize);
        let (ret, from, err) = sys::recvfrom(socketid, slice);
        if ret == -1 {
            // SOCKET_ERROR: the kernel need not have written the address, so
            // the caller's struct is left as it was (see the C comment on
            // WSAECONNRESET: Datagram_GetMessage then still holds the
            // previous packet's sender)
            if err == sys::EWOULDBLOCK || err == sys::ECONNREFUSED {
                return 0;
            }
            if err == sys::ECONNRESET {
                dprint(&format!("WINS_Read, recvfrom: {}\n", sys::strerror(err)));
                return 0;
            }
            safe_print(&format!("WINS_Read, recvfrom: {}\n", sys::strerror(err)));
            return ret;
        }
        *addr = from;
        ret
    }
}

/// `WINS_Write`
///
/// # Safety
/// `buf` has `len` bytes; `addr` is a live qsockaddr.
#[no_mangle]
pub unsafe extern "C" fn rust_udp_Write(
    socketid: SysSocket,
    buf: *mut u8,
    len: c_int,
    addr: *mut QSockAddr,
) -> c_int {
    // SAFETY: caller contract
    unsafe {
        let slice = core::slice::from_raw_parts(buf, len.max(0) as usize);
        let (ret, err) = sys::sendto(socketid, slice, &*addr);
        if ret == -1 {
            if err == sys::EWOULDBLOCK {
                return 0;
            }
            safe_print(&format!("WINS_Write, sendto: {}\n", sys::strerror(err)));
        }
        ret
    }
}

/// `WINIPv4_Broadcast`
///
/// # Safety
/// `buf` has `len` bytes; single-threaded host frame.
#[no_mangle]
pub unsafe extern "C" fn rust_udp4_Broadcast(
    socketid: SysSocket,
    buf: *mut u8,
    len: c_int,
) -> c_int {
    // SAFETY: caller contract
    unsafe {
        if socketid != BROADCAST4 {
            if BROADCAST4 != INVALID {
                sys_error("Attempted to use multiple broadcasts sockets");
            }
            get_local_address4();
            // make this socket broadcast capable
            if let Err(err) = sys::set_broadcast(socketid) {
                safe_print(&format!("UDP, setsockopt: {}\n", sys::strerror(err)));
                safe_print("Unable to make socket broadcast capable\n");
                return -1;
            }
            BROADCAST4 = socketid;
        }
        let mut addr = BROADCAST_ADDR4;
        rust_udp_Write(socketid, buf, len, &mut addr)
    }
}

/// `WINIPv6_Broadcast`: the ff03::1 group, port refreshed from net_hostport
///
/// # Safety
/// `buf` has `len` bytes; single-threaded host frame.
#[no_mangle]
pub unsafe extern "C" fn rust_udp6_Broadcast(
    socketid: SysSocket,
    buf: *mut u8,
    len: c_int,
) -> c_int {
    // SAFETY: caller contract
    unsafe {
        let b = &mut *core::ptr::addr_of_mut!(BROADCAST_ADDR6);
        b.qsa_data[0..2].copy_from_slice(&(c::net_hostport as u16).to_be_bytes());
        let mut addr = *b;
        rust_udp_Write(socketid, buf, len, &mut addr)
    }
}

static mut ADDR_STR_BUF: [u8; 64] = [0; 64];

/// `WINS_AddrToString` (returns the driver's static buffer, like C)
///
/// # Safety
/// `addr` is a live qsockaddr; single-threaded host frame.
#[no_mangle]
pub unsafe extern "C" fn rust_udp_AddrToString(
    addr: *mut QSockAddr,
    masked: bool,
) -> *const c_char {
    // SAFETY: caller contract; the static return buffer mirrors the C one
    unsafe {
        let s = udp::addr_to_string_wins(&*addr, masked);
        let buf = &mut *core::ptr::addr_of_mut!(ADDR_STR_BUF);
        let n = s.len().min(buf.len() - 1);
        buf[..n].copy_from_slice(&s.as_bytes()[..n]);
        buf[n] = 0;
        buf.as_ptr().cast()
    }
}

/// `WINIPv4_StringToAddr`
///
/// # Safety
/// `string` NUL-terminated; `addr` live out-param.
#[no_mangle]
pub unsafe extern "C" fn rust_udp4_StringToAddr(
    string: *const c_char,
    addr: *mut QSockAddr,
) -> c_int {
    // SAFETY: caller contract
    unsafe {
        *addr = udp::string_to_addr4(CStr::from_ptr(string).to_bytes());
        0
    }
}

/// `WINIPv6_StringToAddr` ("This is never actually called...": -1, no
/// lookup, unlike the unix driver)
#[no_mangle]
pub extern "C" fn rust_udp6_StringToAddr(_string: *const c_char, _addr: *mut QSockAddr) -> c_int {
    -1
}

/// `WINS_GetSocketAddr`: getsockname + the loopback/any -> myAddr
/// substitution (always 0)
///
/// # Safety
/// `addr` is a live qsockaddr out-param.
#[no_mangle]
pub unsafe extern "C" fn rust_udp_GetSocketAddr(
    socketid: SysSocket,
    addr: *mut QSockAddr,
) -> c_int {
    // SAFETY: caller contract
    unsafe {
        let mut a = sys::getsockname(socketid);
        if udp::family(&a) == AF_INET {
            let cur = [a.qsa_data[2], a.qsa_data[3], a.qsa_data[4], a.qsa_data[5]];
            if cur == [0, 0, 0, 0] || cur == sys::INADDR_LOOPBACK.to_be_bytes() {
                a.qsa_data[2..6].copy_from_slice(&MY_ADDR4.to_ne_bytes());
            }
        }
        if udp::family(&a) == AF_INET6 && a.qsa_data[6..22] == [0u8; 16] {
            let my6 = *core::ptr::addr_of!(MY_ADDR6);
            a.qsa_data[6..22].copy_from_slice(&my6);
        }
        *addr = a;
        0
    }
}

/// `WINIPv4_GetNameFromAddr`: reverse DNS, AddrToString fallback
///
/// # Safety
/// `addr` live; `name` has NET_NAMELEN bytes.
#[no_mangle]
pub unsafe extern "C" fn rust_udp4_GetNameFromAddr(
    addr: *mut QSockAddr,
    name: *mut c_char,
) -> c_int {
    // SAFETY: caller contract
    unsafe {
        let a = &*addr;
        let s_addr =
            u32::from_ne_bytes([a.qsa_data[2], a.qsa_data[3], a.qsa_data[4], a.qsa_data[5]]);
        if let Some(h) = sys::gethostbyaddr4(s_addr) {
            // strncpy (name, h_name, NET_NAMELEN - 1): zero-padded when
            // shorter, unterminated when longer (mirrored exactly)
            let dst = core::slice::from_raw_parts_mut(name.cast::<u8>(), NET_NAMELEN - 1);
            let n = h.len().min(NET_NAMELEN - 1);
            dst[..n].copy_from_slice(&h[..n]);
            dst[n..].fill(0);
            return 0;
        }
        // C strcpy'd from AddrToString's 64-byte static (q_snprintf-
        // truncated); clamp to the same bound so this FFI write can never
        // outgrow the caller's NET_NAMELEN buffer
        store_name(name, &udp::addr_to_string_wins(a, false));
        0
    }
}

/// `WINIPv6_GetNameFromAddr`: `q_strlcpy (name, AddrToString, NET_NAMELEN)`
///
/// # Safety
/// `addr` live; `name` has NET_NAMELEN bytes.
#[no_mangle]
pub unsafe extern "C" fn rust_udp6_GetNameFromAddr(
    addr: *mut QSockAddr,
    name: *mut c_char,
) -> c_int {
    // SAFETY: caller contract
    unsafe {
        store_name(name, &udp::addr_to_string_wins(&*addr, false));
        0
    }
}

/// # Safety
/// `name` has NET_NAMELEN bytes.
unsafe fn store_name(name: *mut c_char, s: &str) {
    let n = s.len().min(NET_NAMELEN - 1);
    // SAFETY: caller contract; n + 1 <= NET_NAMELEN
    unsafe {
        let dst = core::slice::from_raw_parts_mut(name.cast::<u8>(), n + 1);
        dst[..n].copy_from_slice(&s.as_bytes()[..n]);
        dst[n] = 0;
    }
}

/// `WINIPv4_GetAddrFromName`
///
/// # Safety
/// `name` NUL-terminated; `addr` live out-param; single-threaded.
#[no_mangle]
pub unsafe extern "C" fn rust_udp4_GetAddrFromName(
    name: *const c_char,
    addr: *mut QSockAddr,
) -> c_int {
    // SAFETY: caller contract
    unsafe {
        let bytes = CStr::from_ptr(name).to_bytes();
        if bytes.first().copied().is_some_and(|b| b.is_ascii_digit()) {
            return match udp::partial_ip_address(bytes, MY_ADDR4, c::net_hostport) {
                Some(a) => {
                    *addr = a;
                    0
                }
                None => -1,
            };
        }

        let Some((host, port)) = udp::split_host_port(bytes, MAXHOSTNAMELEN) else {
            return -1;
        };
        // don't resolve a name to an ipv4 address if it has multiple colons
        // in it. its probably an ipx or ipv6 address (net_wins.c only)
        if port.is_some() && host.contains(&b':') {
            return -1;
        }
        let port = port.unwrap_or(c::net_hostport as u16);
        match sys::host_by_name(&host) {
            // `*(in_addr_t *)hostentry->h_addr_list[0]` with no h_addrtype
            // check (net_wins.c differs from net_udp.c here)
            Ok(h) => match h.addrs.first() {
                Some(&s_addr) => {
                    let mut a = QSockAddr::zeroed();
                    udp::set_family(&mut a, AF_INET);
                    a.qsa_data[0..2].copy_from_slice(&port.to_be_bytes());
                    a.qsa_data[2..6].copy_from_slice(&s_addr.to_ne_bytes());
                    *addr = a;
                    0
                }
                None => -1,
            },
            Err(_) => -1,
        }
    }
}

/// `WINIPv6_GetAddrFromName` (same control flow as net_udp.c's; see the
/// unix driver for the retry/clobber notes)
///
/// # Safety
/// `name` NUL-terminated; `addr` live out-param; single-threaded.
#[no_mangle]
pub unsafe extern "C" fn rust_udp6_GetAddrFromName(
    name: *const c_char,
    addr: *mut QSockAddr,
) -> c_int {
    // SAFETY: caller contract
    unsafe {
        let bytes = CStr::from_ptr(name).to_bytes();
        const DUPBASE: usize = 256; // char dupbase[256]

        // true when getaddrinfo returned success but no AF_INET6 entry --
        // the one case where C has already clobbered the caller's family
        let mut resolved_but_unusable = false;

        let found = if bytes.first() == Some(&b'[') {
            // the bracket branch never retries (C sets EAI_NONAME and falls
            // straight through to the success check)
            match bytes.iter().position(|&b| b == b']') {
                None => None,
                Some(close) => {
                    let mut len = close - 1;
                    if len >= DUPBASE {
                        len = DUPBASE - 1;
                    }
                    let host = &bytes[1..=len];
                    let service = if bytes.get(close + 1) == Some(&b':') {
                        Some(&bytes[close + 2..])
                    } else {
                        None
                    };
                    match sys::getaddrinfo_pick6(host, service) {
                        Ok(opt) => {
                            resolved_but_unusable = opt.is_none();
                            opt
                        }
                        Err(_) => None,
                    }
                }
            }
        } else {
            // C retries the whole string with no service ONLY when the
            // host:port getaddrinfo errored (or there was no colon) -- a
            // successful lookup with no AF_INET6 result does NOT retry
            let with_port = match bytes.iter().rposition(|&b| b == b':') {
                Some(colon) => {
                    let mut len = colon;
                    if len >= DUPBASE {
                        len = DUPBASE - 1;
                    }
                    sys::getaddrinfo_pick6(&bytes[..len], Some(&bytes[colon + 1..]))
                }
                None => Err(-2), // EAI_NONAME stand-in
            };
            match with_port {
                Ok(opt) => {
                    resolved_but_unusable = opt.is_none();
                    opt
                }
                Err(_) => match sys::getaddrinfo_pick6(bytes, None) {
                    Ok(opt) => {
                        resolved_but_unusable = opt.is_none();
                        opt
                    }
                    Err(_) => None,
                },
            }
        };

        if let Some(mut a) = found {
            if udp::get_socket_port_wins(&a) == 0 {
                udp::set_socket_port_wins(&mut a, c::net_hostport);
            }
            // COMPAT: C memcpy's only ai_addrlen bytes, leaving the
            // caller's tail; the port writes all 64. Unobservable --
            // every caller gates on the return value (see the unix
            // driver's note).
            *addr = a;
            0
        } else {
            // C sets `((struct sockaddr *)addr)->sa_family = 0` before
            // walking the addrinfo list, so a lookup that SUCCEEDS with
            // no AF_INET6 result leaves the caller's family clobbered
            // even though it returns -1. Mirrored.
            if resolved_but_unusable {
                udp::set_family(&mut *addr, 0);
            }
            -1
        }
    }
}

/// `WINS_AddrCompare`
///
/// # Safety
/// Both addresses are live qsockaddrs.
#[no_mangle]
pub unsafe extern "C" fn rust_udp_AddrCompare(
    addr1: *mut QSockAddr,
    addr2: *mut QSockAddr,
) -> c_int {
    // SAFETY: caller contract
    unsafe { udp::addr_compare_wins(&*addr1, &*addr2) }
}

/// `WINS_GetSocketPort`
///
/// # Safety
/// `addr` is a live qsockaddr.
#[no_mangle]
pub unsafe extern "C" fn rust_udp_GetSocketPort(addr: *mut QSockAddr) -> c_int {
    // SAFETY: caller contract
    unsafe { udp::get_socket_port_wins(&*addr) }
}

/// `WINS_SetSocketPort`
///
/// # Safety
/// `addr` is a live qsockaddr.
#[no_mangle]
pub unsafe extern "C" fn rust_udp_SetSocketPort(addr: *mut QSockAddr, port: c_int) -> c_int {
    // SAFETY: caller contract
    unsafe { udp::set_socket_port_wins(&mut *addr, port) }
}

/// `q_strlcpy (addresses[i], s, sizeof (addresses[0]))`
///
/// # Safety
/// `slot` is a live qhostaddr_t.
unsafe fn store_address(slot: *mut QHostAddr, s: &[u8]) {
    // SAFETY: caller contract
    unsafe {
        let dst = &mut *slot;
        let n = s.len().min(dst.len() - 1);
        for (i, &b) in s[..n].iter().enumerate() {
            dst[i] = b as c_char;
        }
        dst[n] = 0;
    }
}

/// `WINIPv4_GetAddresses`: a DNS lookup on our own hostname, else
/// my_ipv4_address (written even when `maxaddresses` is 0, like C)
///
/// # Safety
/// `addresses` has `maxaddresses` slots.
#[no_mangle]
pub unsafe extern "C" fn rust_udp4_GetAddresses(
    addresses: *mut QHostAddr,
    maxaddresses: c_int,
) -> c_int {
    // SAFETY: caller contract; module statics + engine globals are
    // host-thread-only
    unsafe {
        let mut result = 0usize;
        if BIND_ADDR4 == sys::INADDR_ANY {
            // gethostname (buf, sizeof (buf)) with the result ignored: C
            // passes whatever the 64-byte stack buffer holds on failure;
            // the empty name (Winsock: the local host) stands in for that
            let buf = sys::gethostname(64).unwrap_or_default();
            if let Ok(h) = sys::host_by_name(&buf) {
                if h.addrtype == AF_INET {
                    for &a in h.addrs.iter().take(maxaddresses.max(0) as usize) {
                        store_address(addresses.add(result), dotted(u32::from_be(a)).as_bytes());
                        result += 1;
                    }
                }
            }
        }
        if result == 0 {
            let my4 = &*core::ptr::addr_of!(c::my_ipv4_address);
            let len = my4.iter().position(|&ch| ch == 0).unwrap_or(my4.len());
            let bytes: Vec<u8> = my4[..len].iter().map(|&ch| ch as u8).collect();
            store_address(addresses, &bytes);
            result = 1;
        }
        result as c_int
    }
}

/// `WINIPv6_GetAddresses`
///
/// # Safety
/// `addresses` has `maxaddresses` slots.
#[no_mangle]
pub unsafe extern "C" fn rust_udp6_GetAddresses(
    addresses: *mut QHostAddr,
    maxaddresses: c_int,
) -> c_int {
    // SAFETY: caller contract; engine globals are host-thread-only
    unsafe {
        let mut result = 0usize;
        let buf = sys::gethostname(64).unwrap_or_default();
        if let Some(list) = sys::getaddrinfo_all6(&buf) {
            for a in list.iter().take(maxaddresses.max(0) as usize) {
                store_address(
                    addresses.add(result),
                    udp::addr_to_string_wins(a, false).as_bytes(),
                );
                result += 1;
            }
        }
        if result == 0 {
            let my6 = &*core::ptr::addr_of!(c::my_ipv6_address);
            let len = my6.iter().position(|&ch| ch == 0).unwrap_or(my6.len());
            let bytes: Vec<u8> = my6[..len].iter().map(|&ch| ch as u8).collect();
            store_address(addresses, &bytes);
            result = 1;
        }
        result as c_int
    }
}
