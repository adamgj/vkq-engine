//! Phase 5 M7b: the Rust UDP landriver (net_udp.c), installed into
//! `net_landrivers[]` by net_bsd.c under `USE_RUST_NET`. Unix only: the
//! net_wins.c personality keeps its C driver until a Windows UDP runtime CI
//! leg exists (ADR-017 precedent, recorded in the task plan).
//!
//! The file statics (accept/control/broadcast sockets, broadcastaddr4,
//! myAddr4/myAddrv6, the linux iflist cache) become Rust module state --
//! never C-visible (ADR-007). Address logic lives in `quake_net::udp`
//! (pure), the syscalls in `quake_net::udp::sys` (the ADR-004 island);
//! this module owns the engine globals (net_hostport, my_ipv*_address,
//! ipv*Available) and the console/Sys_Error surfaces.
//!
//! Con_SafePrintf is called at the same points as C (it suppresses screen
//! redraws by design, so the M3 Con_Printf caution does not apply); no
//! C-memory borrows are held across any of those calls.

#![cfg(unix)]

use core::ffi::{c_char, c_int, CStr};

use quake_c_sys as c;
use quake_net::udp::{self, sys, AF_INET, AF_INET6};
use quake_types::net::{QHostAddr, QSockAddr, SysSocket, NET_NAMELEN};

const INVALID: SysSocket = -1;
#[cfg(target_os = "linux")]
const MAXHOSTNAMELEN: usize = 64;
#[cfg(not(target_os = "linux"))]
const MAXHOSTNAMELEN: usize = 256;

static mut ACCEPT4: SysSocket = INVALID;
static mut CONTROL4: SysSocket = 0;
static mut BROADCAST4: SysSocket = INVALID;
static mut BROADCAST_ADDR4: QSockAddr = QSockAddr::zeroed();
/// network byte order, like the C `in_addr_t myAddr4`
static mut MY_ADDR4: u32 = 0;
static mut ACCEPT6: SysSocket = INVALID;
static mut CONTROL6: SysSocket = 0;
static mut MY_ADDR6: [u8; 16] = [0; 16];

fn safe_print(text: &str) {
    let mut b = text.as_bytes().to_vec();
    b.push(0);
    // SAFETY: b is NUL-terminated; Con_SafePrintf never redraws
    unsafe {
        c::Con_SafePrintf(c"%s".as_ptr(), b.as_ptr());
    }
}

fn sys_error(text: &str) -> ! {
    let mut b = text.as_bytes().to_vec();
    b.push(0);
    // SAFETY: b is NUL-terminated
    unsafe { c::Sys_Error(c"%s".as_ptr(), b.as_ptr()) }
}

fn check_parm(p: &CStr) -> bool {
    // SAFETY: COM_CheckParm reads a NUL-terminated string
    unsafe { c::COM_CheckParm(p.as_ptr()) != 0 }
}

/// `UDP4_OpenSocket` (prints included)
fn open_socket4(port: c_int) -> SysSocket {
    match sys::open_socket4(port as u16) {
        Ok(fd) => fd,
        Err((err, fd)) => {
            safe_print(&format!("UDP4_OpenSocket: {}\n", sys::strerror(err)));
            if let Some(fd) = fd {
                close_socket_impl(fd);
            }
            INVALID
        }
    }
}

/// `UDP6_OpenSocket` (prints included)
fn open_socket6(port: c_int) -> SysSocket {
    match sys::open_socket6(port as u16) {
        Ok(fd) => fd,
        Err((err, fd)) => {
            safe_print(&format!("UDP6_OpenSocket: {}\n", sys::strerror(err)));
            if let Some(fd) = fd {
                close_socket_impl(fd);
            }
            INVALID
        }
    }
}

fn close_socket_impl(fd: SysSocket) -> c_int {
    // SAFETY: module statics are host-thread-only
    unsafe {
        if fd == BROADCAST4 {
            BROADCAST4 = INVALID;
        }
    }
    sys::close_socket(fd)
}

/// writes the C `my_ipv4_address`-style string: AddrToString minus the
/// trailing `:port` (everything from the LAST colon)
fn store_my_address(dst: &mut [c_char; NET_NAMELEN], addr: &QSockAddr) {
    let s = udp::addr_to_string(addr);
    let cut = s.rfind(':').unwrap_or(s.len());
    let b = &s.as_bytes()[..cut];
    let n = b.len().min(NET_NAMELEN - 1);
    for (i, &ch) in b[..n].iter().enumerate() {
        dst[i] = ch as c_char;
    }
    dst[n] = 0;
}

/// `UDP4_Init`
///
/// # Safety
/// Single-threaded engine init.
#[no_mangle]
pub unsafe extern "C" fn rust_udp4_Init() -> SysSocket {
    // SAFETY: module statics + engine globals are host-thread-only
    unsafe {
        if check_parm(c"-noudp") || check_parm(c"-noudp4") {
            return INVALID;
        }

        MY_ADDR4 = 0x7f00_0001u32.to_be(); // htonl (INADDR_LOOPBACK)

        #[cfg(not(target_os = "linux"))]
        {
            // determine my name & address (linux skips this legacy path)
            match sys::gethostname(MAXHOSTNAMELEN) {
                Err(err) => {
                    safe_print(&format!(
                        "UDP4_Init: gethostname failed ({})\n",
                        sys::strerror(err)
                    ));
                }
                Ok(buff) => {
                    // COMPAT (macOS): skip gethostbyname for ".local" names
                    // -- it blocks for seconds and then fails
                    let is_local = cfg!(target_os = "macos")
                        && buff
                            .windows(6)
                            .position(|w| w == b".local")
                            .is_some_and(|i| i + 6 == buff.len());
                    if is_local {
                        safe_print(&format!(
                            "UDP_Init: skipping gethostbyname for {}\n",
                            String::from_utf8_lossy(&buff)
                        ));
                    } else {
                        match sys::host_by_name(&buff) {
                            sys::HostByName::Failed(msg) => {
                                safe_print(&format!("UDP4_Init: gethostbyname failed ({msg})\n"));
                            }
                            sys::HostByName::NotInet => {
                                safe_print("UDP4_Init: address from gethostbyname not IPv4\n");
                            }
                            sys::HostByName::V4(a) => MY_ADDR4 = a,
                        }
                    }
                }
            }
        }

        CONTROL4 = open_socket4(0);
        if CONTROL4 == INVALID {
            safe_print("UDP4_Init: Unable to open control socket, UDP disabled\n");
            return INVALID;
        }

        let mut b = QSockAddr::zeroed();
        udp::set_family(&mut b, AF_INET);
        b.qsa_data[2..6].copy_from_slice(&[255, 255, 255, 255]); // INADDR_BROADCAST
        b.qsa_data[0..2].copy_from_slice(&(c::net_hostport as u16).to_be_bytes());
        BROADCAST_ADDR4 = b;

        let mut addr = QSockAddr::zeroed();
        rust_udp_GetSocketAddr(CONTROL4, &mut addr);
        store_my_address(&mut *core::ptr::addr_of_mut!(c::my_ipv4_address), &addr);

        safe_print("UDP4 Initialized\n");
        c::ipv4Available = true;

        CONTROL4
    }
}

/// `UDP6_Init`
///
/// # Safety
/// Single-threaded engine init.
#[no_mangle]
pub unsafe extern "C" fn rust_udp6_Init() -> SysSocket {
    // SAFETY: see rust_udp4_Init
    unsafe {
        if check_parm(c"-noudp") || check_parm(c"-noudp6") {
            return INVALID;
        }

        CONTROL6 = open_socket6(0);
        if CONTROL6 == INVALID {
            safe_print("UDP6_Init: Unable to open control socket, UDPv6 disabled\n");
            return INVALID;
        }

        let mut addr = QSockAddr::zeroed();
        rust_udp_GetSocketAddr(CONTROL6, &mut addr);
        store_my_address(&mut *core::ptr::addr_of_mut!(c::my_ipv6_address), &addr);

        safe_print("UDPv6 Initialized\n");
        c::ipv6Available = true;

        CONTROL6
    }
}

/// # Safety
/// Single-threaded engine shutdown.
#[no_mangle]
pub unsafe extern "C" fn rust_udp4_Shutdown() {
    // SAFETY: caller contract
    unsafe {
        rust_udp4_Listen(false);
        close_socket_impl(CONTROL4);
    }
}

/// # Safety
/// Single-threaded engine shutdown.
#[no_mangle]
pub unsafe extern "C" fn rust_udp6_Shutdown() {
    // SAFETY: caller contract
    unsafe {
        rust_udp6_Listen(false);
        close_socket_impl(CONTROL6);
    }
}

/// # Safety
/// Single-threaded host frame.
#[no_mangle]
pub unsafe extern "C" fn rust_udp4_Listen(state: bool) -> SysSocket {
    // SAFETY: caller contract
    unsafe {
        if state {
            if ACCEPT4 == INVALID {
                ACCEPT4 = open_socket4(c::net_hostport);
                if ACCEPT4 == INVALID {
                    sys_error("UDP4_Listen: Unable to open accept socket");
                }
            }
        } else if ACCEPT4 != INVALID {
            close_socket_impl(ACCEPT4);
            ACCEPT4 = INVALID;
        }
        ACCEPT4
    }
}

/// # Safety
/// Single-threaded host frame.
#[no_mangle]
pub unsafe extern "C" fn rust_udp6_Listen(state: bool) -> SysSocket {
    // SAFETY: caller contract
    unsafe {
        if state {
            if ACCEPT6 == INVALID {
                ACCEPT6 = open_socket6(c::net_hostport);
                if ACCEPT6 == INVALID {
                    sys_error("UDP6_Listen: Unable to open accept socket");
                }
            }
        } else if ACCEPT6 != INVALID {
            close_socket_impl(ACCEPT6);
            ACCEPT6 = INVALID;
        }
        ACCEPT6
    }
}

/// `UDP4_OpenSocket` vtable slot
#[no_mangle]
pub extern "C" fn rust_udp4_OpenSocket(port: c_int) -> SysSocket {
    open_socket4(port)
}

/// `UDP6_OpenSocket` vtable slot
#[no_mangle]
pub extern "C" fn rust_udp6_OpenSocket(port: c_int) -> SysSocket {
    open_socket6(port)
}

/// `UDP_CloseSocket`
#[no_mangle]
pub extern "C" fn rust_udp_CloseSocket(socketid: SysSocket) -> c_int {
    close_socket_impl(socketid)
}

/// `UDP_Connect` (a no-op in the C driver)
#[no_mangle]
pub extern "C" fn rust_udp_Connect(_socketid: SysSocket, _addr: *mut QSockAddr) -> c_int {
    0
}

unsafe fn check_new_connections(accept: SysSocket, v6: bool) -> SysSocket {
    if accept == INVALID {
        return INVALID;
    }
    match sys::available(accept) {
        Err(err) => {
            sys_error(&format!(
                "UDP{}: ioctlsocket (FIONREAD) failed ({})",
                if v6 { "6" } else { "" },
                sys::strerror(err)
            ));
        }
        Ok(avail) => {
            if avail != 0 {
                return accept;
            }
        }
    }
    // quietly absorb empty packets
    sys::absorb_empty(accept);
    INVALID
}

/// # Safety
/// Single-threaded host frame.
#[no_mangle]
pub unsafe extern "C" fn rust_udp4_CheckNewConnections() -> SysSocket {
    // SAFETY: caller contract
    unsafe { check_new_connections(ACCEPT4, false) }
}

/// # Safety
/// Single-threaded host frame.
#[no_mangle]
pub unsafe extern "C" fn rust_udp6_CheckNewConnections() -> SysSocket {
    // SAFETY: caller contract
    unsafe { check_new_connections(ACCEPT6, true) }
}

/// `UDP_Read`
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
        let slice = core::slice::from_raw_parts_mut(buf, len as usize);
        let (ret, from, err) = sys::recvfrom(socketid, slice);
        *addr = from;
        if ret == -1 {
            if err == libc::EWOULDBLOCK || err == libc::ECONNREFUSED {
                return 0;
            }
            safe_print(&format!("UDP_Read, recvfrom: {}\n", sys::strerror(err)));
        }
        ret as c_int
    }
}

/// `UDP_Write`
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
        let a = &*addr;
        let addrsize = if udp::family(a) == AF_INET {
            core::mem::size_of::<libc::sockaddr_in>() as u32
        } else if udp::family(a) == AF_INET6 {
            core::mem::size_of::<libc::sockaddr_in6>() as u32
        } else {
            safe_print("UDP_Write: unknown family\n");
            return -1; // some kind of error. a few systems get pissy if the size doesn't exactly match the address family
        };
        let slice = core::slice::from_raw_parts(buf, len as usize);
        let (ret, err) = sys::sendto(socketid, slice, a, addrsize);
        if udp::family(&*addr) == 0 {
            safe_print("UDP_Write: family was cleared\n");
        }
        if ret == -1 {
            if err == libc::EWOULDBLOCK {
                return 0;
            }
            if err == libc::ENETUNREACH {
                safe_print(&format!(
                    "UDP_Write: {} ({})\n",
                    sys::strerror(err),
                    udp::addr_to_string(&*addr)
                ));
            } else {
                safe_print(&format!("UDP_Write, sendto: {}\n", sys::strerror(err)));
            }
        }
        ret as c_int
    }
}

/// `UDP4_Broadcast`
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

/// `UDP6_Broadcast`: the ff03::1 multicast group
///
/// # Safety
/// `buf` has `len` bytes.
#[no_mangle]
pub unsafe extern "C" fn rust_udp6_Broadcast(
    socketid: SysSocket,
    buf: *mut u8,
    len: c_int,
) -> c_int {
    // SAFETY: caller contract
    unsafe {
        let mut addr = QSockAddr::zeroed();
        udp::set_family(&mut addr, AF_INET6);
        addr.qsa_data[6] = 0xff;
        addr.qsa_data[7] = 0x03;
        addr.qsa_data[21] = 0x1;
        addr.qsa_data[0..2].copy_from_slice(&(c::net_hostport as u16).to_be_bytes());
        rust_udp_Write(socketid, buf, len, &mut addr)
    }
}

static mut ADDR_STR_BUF: [u8; 64] = [0; 64];

/// `UDP_AddrToString` (returns the driver's static buffer, like C)
///
/// # Safety
/// `addr` is a live qsockaddr; single-threaded host frame.
#[no_mangle]
pub unsafe extern "C" fn rust_udp_AddrToString(
    addr: *mut QSockAddr,
    _masked: bool,
) -> *const c_char {
    // SAFETY: caller contract; the static return buffer mirrors the C one
    unsafe {
        let s = udp::addr_to_string(&*addr);
        let buf = &mut *core::ptr::addr_of_mut!(ADDR_STR_BUF);
        let n = s.len().min(buf.len() - 1);
        buf[..n].copy_from_slice(&s.as_bytes()[..n]);
        buf[n] = 0;
        buf.as_ptr().cast()
    }
}

/// `UDP4_StringToAddr`
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

/// `UDP6_StringToAddr` ("This is never actually called...")
///
/// # Safety
/// `string` NUL-terminated; `addr` live out-param.
#[no_mangle]
pub unsafe extern "C" fn rust_udp6_StringToAddr(
    string: *const c_char,
    addr: *mut QSockAddr,
) -> c_int {
    // SAFETY: caller contract
    unsafe {
        safe_print(&format!(
            "UDP6_StringToAddr: {}\n",
            CStr::from_ptr(string).to_string_lossy()
        ));
        rust_udp6_GetAddrFromName(string, addr)
    }
}

/// `UDP_GetSocketAddr`: getsockname + the loopback/any -> myAddr
/// substitution
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
        let Some(mut a) = sys::getsockname(socketid) else {
            *addr = QSockAddr::zeroed();
            return -1;
        };
        if udp::family(&a) == AF_INET {
            let cur = [a.qsa_data[2], a.qsa_data[3], a.qsa_data[4], a.qsa_data[5]];
            if cur == [0, 0, 0, 0] || cur == [127, 0, 0, 1] {
                a.qsa_data[2..6].copy_from_slice(&MY_ADDR4.to_ne_bytes());
            }
        } else if udp::family(&a) == AF_INET6 && a.qsa_data[6..22] == [0u8; 16] {
            let my6 = *core::ptr::addr_of!(MY_ADDR6);
            a.qsa_data[6..22].copy_from_slice(&my6);
        }
        *addr = a;
        0
    }
}

/// `UDP_GetNameFromAddr`: reverse DNS for v4, AddrToString fallback
///
/// # Safety
/// `addr` live; `name` has NET_NAMELEN bytes.
#[no_mangle]
pub unsafe extern "C" fn rust_udp_GetNameFromAddr(
    addr: *mut QSockAddr,
    name: *mut c_char,
) -> c_int {
    // SAFETY: caller contract
    unsafe {
        let a = &*addr;
        if udp::family(a) == AF_INET {
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
        }
        // (v6: "meh, don't bother, its unreliable anyway.")
        let s = udp::addr_to_string(a);
        let dst = core::slice::from_raw_parts_mut(name.cast::<u8>(), s.len() + 1);
        dst[..s.len()].copy_from_slice(s.as_bytes());
        dst[s.len()] = 0;
        0
    }
}

/// `UDP4_GetAddrFromName`
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
        let port = port.unwrap_or(c::net_hostport as u16);
        match sys::host_by_name(&host) {
            sys::HostByName::V4(s_addr) => {
                let mut a = QSockAddr::zeroed();
                udp::set_family(&mut a, AF_INET);
                a.qsa_data[0..2].copy_from_slice(&port.to_be_bytes());
                a.qsa_data[2..6].copy_from_slice(&s_addr.to_ne_bytes());
                *addr = a;
                0
            }
            _ => -1,
        }
    }
}

/// `UDP6_GetAddrFromName`
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

        let found = if bytes.first() == Some(&b'[') {
            match bytes.iter().position(|&b| b == b']') {
                None => None,
                Some(close) => {
                    let mut len = close - 1;
                    if len >= DUPBASE {
                        len = DUPBASE - 1;
                    }
                    let host = &bytes[1..1 + len];
                    let service = if bytes.get(close + 1) == Some(&b':') {
                        Some(&bytes[close + 2..])
                    } else {
                        None
                    };
                    sys::getaddrinfo_pick6(host, service)
                }
            }
        } else {
            let with_port = match bytes.iter().rposition(|&b| b == b':') {
                Some(colon) => {
                    let mut len = colon;
                    if len >= DUPBASE {
                        len = DUPBASE - 1;
                    }
                    sys::getaddrinfo_pick6(&bytes[..len], Some(&bytes[colon + 1..]))
                }
                None => None,
            };
            // failed, try string with no port
            match with_port {
                Some(a) => Some(a),
                None => sys::getaddrinfo_pick6(bytes, None),
            }
        };

        match found {
            Some(mut a) => {
                if udp::get_socket_port(&a) == 0 {
                    udp::set_socket_port(&mut a, c::net_hostport);
                }
                *addr = a;
                0
            }
            None => -1,
        }
    }
}

/// `UDP_AddrCompare`
///
/// # Safety
/// Both addresses are live qsockaddrs.
#[no_mangle]
pub unsafe extern "C" fn rust_udp_AddrCompare(
    addr1: *mut QSockAddr,
    addr2: *mut QSockAddr,
) -> c_int {
    // SAFETY: caller contract
    unsafe { udp::addr_compare(&*addr1, &*addr2) }
}

/// `UDP_GetSocketPort`
///
/// # Safety
/// `addr` is a live qsockaddr.
#[no_mangle]
pub unsafe extern "C" fn rust_udp_GetSocketPort(addr: *mut QSockAddr) -> c_int {
    // SAFETY: caller contract
    unsafe { udp::get_socket_port(&*addr) }
}

/// `UDP_SetSocketPort`
///
/// # Safety
/// `addr` is a live qsockaddr.
#[no_mangle]
pub unsafe extern "C" fn rust_udp_SetSocketPort(addr: *mut QSockAddr, port: c_int) -> c_int {
    // SAFETY: caller contract
    unsafe { udp::set_socket_port(&mut *addr, port) }
}

/// linux iflist cache (`static struct ifaddrs *iflist; static double
/// iftime`; keyed per family here since sys::interface_addrs pre-filters)
#[cfg(target_os = "linux")]
static mut IF_CACHE: (f64, i32, Vec<QSockAddr>) = (0.0, -1, Vec::new());

unsafe fn get_addresses(addresses: *mut QHostAddr, maxaddresses: c_int, fam: i32) -> c_int {
    #[cfg(not(target_os = "linux"))]
    {
        // "for other systems, like macs, where we don't know how to query
        // this stuff properly."
        let _ = (addresses, maxaddresses, fam);
        0
    }
    #[cfg(target_os = "linux")]
    // SAFETY: single-threaded host frame; addresses has maxaddresses slots
    unsafe {
        let time = c::Sys_DoubleTime();
        let cache = &mut *core::ptr::addr_of_mut!(IF_CACHE);
        if time - cache.0 > 1.0 || cache.1 != fam {
            cache.0 = time;
            cache.1 = fam;
            cache.2 = sys::interface_addrs(fam);
        }
        let mut result = 0usize;
        for ifaddr in cache.2.iter().take(maxaddresses.max(0) as usize) {
            if udp::family(ifaddr) != fam {
                continue;
            }
            let mut s = udp::addr_to_string(ifaddr);
            // trim any useless :0 port numbers.
            if let Some(stripped) = s.strip_suffix(":0") {
                if s.len() > 2 {
                    s = stripped.to_owned();
                }
            }
            let dst = &mut *addresses.add(result);
            let n = s.len().min(dst.len() - 1);
            for (i, &b) in s.as_bytes()[..n].iter().enumerate() {
                dst[i] = b as c_char;
            }
            dst[n] = 0;
            result += 1;
        }
        result as c_int
    }
}

/// `UDP4_GetAddresses`
///
/// # Safety
/// `addresses` has `maxaddresses` slots.
#[no_mangle]
pub unsafe extern "C" fn rust_udp4_GetAddresses(
    addresses: *mut QHostAddr,
    maxaddresses: c_int,
) -> c_int {
    // SAFETY: caller contract
    unsafe { get_addresses(addresses, maxaddresses, AF_INET) }
}

/// `UDP6_GetAddresses`
///
/// # Safety
/// `addresses` has `maxaddresses` slots.
#[no_mangle]
pub unsafe extern "C" fn rust_udp6_GetAddresses(
    addresses: *mut QHostAddr,
    maxaddresses: c_int,
) -> c_int {
    // SAFETY: caller contract
    unsafe { get_addresses(addresses, maxaddresses, AF_INET6) }
}
