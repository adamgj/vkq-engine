//! The Windows arm of the ADR-004 island (Phase 9 M2): every Winsock call
//! of net_wins.c, made through `windows-sys` (ADR-003) as the *same*
//! ws2_32 entry points the C driver used -- `socket`/`ioctlsocket`/`bind`
//! rather than socket2's `WSASocketW` wrapper, so the WSAStartup version
//! negotiation, the WSACleanup refcount and every error code are inherited
//! rather than reimplemented (task plan amendment log, M2). No engine state
//! in here; quake-capi owns that.

use core::mem;

use quake_types::net::{QSockAddr, SysSocket};
use windows_sys::Win32::Networking::WinSock as ws;

/// `INVALID_SOCKET`
pub const INVALID: SysSocket = ws::INVALID_SOCKET;

/// `WSAGetLastError ()` (net_sys.h `SOCKETERRNO`)
pub fn last_error() -> i32 {
    // SAFETY: plain thread-local query
    unsafe { ws::WSAGetLastError() }
}

/// `__WSAE_StrError` (wsaerror.h), transliterated table for table:
/// FormatMessage would give different (localized) text
pub fn strerror(err: i32) -> String {
    let s = match err {
        0 => "No error",
        ws::WSAEINTR => "Interrupted system call",
        ws::WSAEBADF => "Bad file number",
        ws::WSAEACCES => "Permission denied",
        ws::WSAEFAULT => "Bad address",
        ws::WSAEINVAL => "Invalid argument (not bind)",
        ws::WSAEMFILE => "Too many open files",
        ws::WSAEWOULDBLOCK => "Operation would block",
        ws::WSAEINPROGRESS => "Operation now in progress",
        ws::WSAEALREADY => "Operation already in progress",
        ws::WSAENOTSOCK => "Socket operation on non-socket",
        ws::WSAEDESTADDRREQ => "Destination address required",
        ws::WSAEMSGSIZE => "Message too long",
        ws::WSAEPROTOTYPE => "Protocol wrong type for socket",
        ws::WSAENOPROTOOPT => "Bad protocol option",
        ws::WSAEPROTONOSUPPORT => "Protocol not supported",
        ws::WSAESOCKTNOSUPPORT => "Socket type not supported",
        ws::WSAEOPNOTSUPP => "Operation not supported on socket",
        ws::WSAEPFNOSUPPORT => "Protocol family not supported",
        ws::WSAEAFNOSUPPORT => "Address family not supported by protocol family",
        ws::WSAEADDRINUSE => "Address already in use",
        ws::WSAEADDRNOTAVAIL => "Can't assign requested address",
        ws::WSAENETDOWN => "Network is down",
        ws::WSAENETUNREACH => "Network is unreachable",
        ws::WSAENETRESET => "Net dropped connection or reset",
        ws::WSAECONNABORTED => "Software caused connection abort",
        ws::WSAECONNRESET => "Connection reset by peer",
        ws::WSAENOBUFS => "No buffer space available",
        ws::WSAEISCONN => "Socket is already connected",
        ws::WSAENOTCONN => "Socket is not connected",
        ws::WSAESHUTDOWN => "Can't send after socket shutdown",
        ws::WSAETOOMANYREFS => "Too many references, can't splice",
        ws::WSAETIMEDOUT => "Connection timed out",
        ws::WSAECONNREFUSED => "Connection refused",
        ws::WSAELOOP => "Too many levels of symbolic links",
        ws::WSAENAMETOOLONG => "File name too long",
        ws::WSAEHOSTDOWN => "Host is down",
        ws::WSAEHOSTUNREACH => "No Route to Host",
        ws::WSAENOTEMPTY => "Directory not empty",
        ws::WSAEPROCLIM => "Too many processes",
        ws::WSAEUSERS => "Too many users",
        ws::WSAEDQUOT => "Disc Quota Exceeded",
        ws::WSAESTALE => "Stale NFS file handle",
        ws::WSAEREMOTE => "Too many levels of remote in path",
        ws::WSAEDISCON => "Graceful shutdown in progress",
        ws::WSASYSNOTREADY => "Network SubSystem is unavailable",
        ws::WSAVERNOTSUPPORTED => "WINSOCK DLL Version out of range",
        ws::WSANOTINITIALISED => "Successful WSASTARTUP not yet performed",
        ws::WSAHOST_NOT_FOUND => "Authoritative answer: Host not found",
        ws::WSATRY_AGAIN => "Non-Authoritative: Host not found or SERVERFAIL",
        ws::WSANO_RECOVERY => "Non-Recoverable errors, FORMERR, REFUSED, NOTIMP",
        ws::WSANO_DATA => "Valid name, no data record of requested type",
        ws::WSAENOMORE => "10102: No more results",
        ws::WSAECANCELLED => "10103: Call has been canceled",
        ws::WSAEINVALIDPROCTABLE => "Procedure call table is invalid",
        ws::WSAEINVALIDPROVIDER => "Service provider is invalid",
        ws::WSAEPROVIDERFAILEDINIT => "Service provider failed to initialize",
        ws::WSASYSCALLFAILURE => "System call failure",
        ws::WSASERVICE_NOT_FOUND => "Service not found",
        ws::WSATYPE_NOT_FOUND => "Class type not found",
        ws::WSA_E_NO_MORE => "10110: No more results",
        ws::WSA_E_CANCELLED => "10111: Call was canceled",
        ws::WSAEREFUSED => "Database query was refused",
        _ => return format!("Unknown WSAE error ({err})"),
    };
    s.to_owned()
}

/// `WSAStartup (MAKEWORD (major, minor), &winsockdata)`; Err = its return
/// code (WSAStartup reports errors directly, not via WSAGetLastError)
pub fn wsa_startup(major: u8, minor: u8) -> Result<(), i32> {
    // SAFETY: WSADATA is plain data filled by the call
    unsafe {
        let mut data: ws::WSADATA = mem::zeroed();
        let err = ws::WSAStartup(u16::from_le_bytes([major, minor]), &mut data);
        if err == 0 {
            Ok(())
        } else {
            Err(err)
        }
    }
}

/// `WSACleanup ()`
pub fn wsa_cleanup() {
    // SAFETY: paired with a successful wsa_startup by the caller's refcount
    unsafe {
        ws::WSACleanup();
    }
}

/// How `WIN*_OpenSocket` failed, with the `SOCKETERRNO` read at the point
/// C reads it; the socket (when one was created) is returned to the caller,
/// which decides between `closesocket` (the `ErrorReturn` label) and the
/// post-init bind-failure branch that leaks it exactly like C.
pub enum OpenError {
    /// `socket ()` itself failed
    Socket(i32),
    /// `ioctlsocket (FIONBIO)` failed
    Ioctl(i32, SysSocket),
    /// `bind ()` failed; carries the address that was being bound (for the
    /// `Unable to bind to %s` warning)
    Bind(i32, SysSocket, QSockAddr),
}

fn set_nonblocking(s: SysSocket) -> Result<(), i32> {
    let mut one: u32 = 1;
    // SAFETY: s is an open socket; `one` is a live local
    unsafe {
        if ws::ioctlsocket(s, ws::FIONBIO, &mut one) == ws::SOCKET_ERROR {
            return Err(last_error());
        }
    }
    Ok(())
}

fn bind(s: SysSocket, addr: &QSockAddr, len: usize) -> Result<(), i32> {
    // SAFETY: addr is a 64-byte qsockaddr blob punned to sockaddr_in/in6
    // (the same pun as the C driver); len is the family's sockaddr size
    unsafe {
        if ws::bind(s, std::ptr::from_ref::<QSockAddr>(addr).cast(), len as i32) == 0 {
            Ok(())
        } else {
            Err(last_error())
        }
    }
}

/// `WINIPv4_OpenSocket` minus the prints: `bind_addr` is the network-order
/// `bindAddrv4`
pub fn open_socket4(bind_addr: u32, port: u16) -> Result<SysSocket, OpenError> {
    // SAFETY: plain Winsock creation call
    let s = unsafe { ws::socket(i32::from(ws::AF_INET), ws::SOCK_DGRAM, ws::IPPROTO_UDP) };
    if s == ws::INVALID_SOCKET {
        return Err(OpenError::Socket(last_error()));
    }
    if let Err(err) = set_nonblocking(s) {
        return Err(OpenError::Ioctl(err, s));
    }
    let mut address = QSockAddr::zeroed();
    address.qsa_family = ws::AF_INET as i16;
    address.qsa_data[0..2].copy_from_slice(&port.to_be_bytes());
    address.qsa_data[2..6].copy_from_slice(&bind_addr.to_ne_bytes());
    match bind(s, &address, 16) {
        Ok(()) => Ok(s),
        Err(err) => Err(OpenError::Bind(err, s, address)),
    }
}

/// `WINIPv6_OpenSocket` minus the prints: v6only + nonblocking + bind to
/// `bind_addr` (the raw `bindAddrv6` bytes) + the `IPV6_JOIN_GROUP` of
/// `group` (both setsockopt results ignored exactly like C)
pub fn open_socket6(
    bind_addr: [u8; 16],
    port: u16,
    group: [u8; 16],
) -> Result<SysSocket, OpenError> {
    // SAFETY: plain Winsock creation call
    let s = unsafe { ws::socket(i32::from(ws::AF_INET6), ws::SOCK_DGRAM, ws::IPPROTO_UDP) };
    if s == ws::INVALID_SOCKET {
        return Err(OpenError::Socket(last_error()));
    }
    let one: u32 = 1;
    // SAFETY: s is open; optval points at a live u32 of the stated length
    unsafe {
        ws::setsockopt(
            s,
            ws::IPPROTO_IPV6,
            ws::IPV6_V6ONLY,
            (&raw const one).cast(),
            mem::size_of::<u32>() as i32,
        );
    }
    if let Err(err) = set_nonblocking(s) {
        return Err(OpenError::Ioctl(err, s));
    }
    let mut address = QSockAddr::zeroed();
    address.qsa_family = ws::AF_INET6 as i16;
    address.qsa_data[0..2].copy_from_slice(&port.to_be_bytes());
    address.qsa_data[6..22].copy_from_slice(&bind_addr);
    match bind(s, &address, 28) {
        Ok(()) => {
            // we don't know if we're the server or not. oh well.
            let req = ws::IPV6_MREQ {
                ipv6mr_multiaddr: ws::IN6_ADDR {
                    u: ws::IN6_ADDR_0 { Byte: group },
                },
                ipv6mr_interface: 0,
            };
            // SAFETY: s is open; optval points at a live IPV6_MREQ
            unsafe {
                ws::setsockopt(
                    s,
                    ws::IPPROTO_IPV6,
                    ws::IPV6_JOIN_GROUP,
                    (&raw const req).cast(),
                    mem::size_of::<ws::IPV6_MREQ>() as i32,
                );
            }
            Ok(s)
        }
        Err(err) => Err(OpenError::Bind(err, s, address)),
    }
}

/// `closesocket`
pub fn close_socket(s: SysSocket) -> i32 {
    // SAFETY: s is a socket owned by the driver
    unsafe { ws::closesocket(s) }
}

/// `recvfrom (socketid, buf, len, 0, (struct sockaddr *)addr, &addrlen)`
/// with `addrlen = sizeof (struct qsockaddr)`. Returns (ret, addr,
/// WSAGetLastError-when-SOCKET_ERROR). The out address starts zeroed (C
/// left the caller's bytes past what the kernel wrote; every consumer reads
/// only the family-sized prefix -- same COMPAT note as the unix arm).
pub fn recvfrom(s: SysSocket, buf: &mut [u8]) -> (i32, QSockAddr, i32) {
    let mut addr = QSockAddr::zeroed();
    let mut addrlen: i32 = mem::size_of::<QSockAddr>() as i32;
    // SAFETY: buf and addr are live locals of the stated sizes
    unsafe {
        let ret = ws::recvfrom(
            s,
            buf.as_mut_ptr(),
            buf.len() as i32,
            0,
            (&raw mut addr).cast(),
            &mut addrlen,
        );
        let err = if ret == ws::SOCKET_ERROR {
            last_error()
        } else {
            0
        };
        (ret, addr, err)
    }
}

/// `WIN*_CheckNewConnections`' probe: `recvfrom (.., MSG_PEEK, NULL, NULL)`
/// into a 4096-byte scratch buffer; true when it did not return
/// SOCKET_ERROR (a zero-length datagram counts as a connection attempt on
/// this driver, unlike the unix FIONREAD+absorb path)
pub fn peek_has_data(s: SysSocket) -> bool {
    let mut buf = [0u8; 4096];
    // SAFETY: buf is a live local of the stated size; NULL from/fromlen
    // are allowed by recvfrom
    unsafe {
        ws::recvfrom(
            s,
            buf.as_mut_ptr(),
            buf.len() as i32,
            ws::MSG_PEEK,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
        ) != ws::SOCKET_ERROR
    }
}

/// `sendto (socketid, buf, len, 0, (struct sockaddr *)addr, sizeof (struct
/// qsockaddr))` -- the C driver passes the whole 64-byte blob for either
/// family. Returns (ret, WSAGetLastError-when-SOCKET_ERROR).
pub fn sendto(s: SysSocket, buf: &[u8], addr: &QSockAddr) -> (i32, i32) {
    // SAFETY: buf/addr are live; tolen is exactly addr's size
    unsafe {
        let ret = ws::sendto(
            s,
            buf.as_ptr(),
            buf.len() as i32,
            0,
            std::ptr::from_ref::<QSockAddr>(addr).cast(),
            mem::size_of::<QSockAddr>() as i32,
        );
        let err = if ret == ws::SOCKET_ERROR {
            last_error()
        } else {
            0
        };
        (ret, err)
    }
}

/// `setsockopt (SOL_SOCKET, SO_BROADCAST, 1)`
pub fn set_broadcast(s: SysSocket) -> Result<(), i32> {
    let one: i32 = 1;
    // SAFETY: s is open; optval points at a live i32 of the stated length
    unsafe {
        if ws::setsockopt(
            s,
            ws::SOL_SOCKET,
            ws::SO_BROADCAST,
            (&raw const one).cast(),
            mem::size_of::<i32>() as i32,
        ) == ws::SOCKET_ERROR
        {
            return Err(last_error());
        }
    }
    Ok(())
}

/// `memset (addr, 0, ..); getsockname (socketid, addr, &addrlen)` -- the C
/// driver ignores the result, so a failure yields the zeroed struct
pub fn getsockname(s: SysSocket) -> QSockAddr {
    let mut addr = QSockAddr::zeroed();
    let mut addrlen: i32 = mem::size_of::<QSockAddr>() as i32;
    // SAFETY: addr is a live 64-byte local; addrlen says so
    unsafe {
        ws::getsockname(s, (&raw mut addr).cast(), &mut addrlen);
    }
    addr
}

/// `gethostname (buff, maxlen)` followed by `buff[maxlen - 1] = 0`: the
/// name bytes up to the first NUL; Err(WSAGetLastError) on SOCKET_ERROR
pub fn gethostname(maxlen: usize) -> Result<Vec<u8>, i32> {
    let mut buff = vec![0u8; maxlen];
    // SAFETY: buff has maxlen bytes
    unsafe {
        if ws::gethostname(buff.as_mut_ptr(), maxlen as i32) == ws::SOCKET_ERROR {
            return Err(last_error());
        }
    }
    buff[maxlen - 1] = 0;
    let n = buff.iter().position(|&b| b == 0).unwrap_or(maxlen);
    buff.truncate(n);
    Ok(buff)
}

/// A `struct hostent` as the driver reads it: `h_addrtype` and the first
/// four bytes of every `h_addr_list` entry (network-order `in_addr_t`,
/// which is all `*(in_addr_t *)h_addr_list[i]` ever loads)
pub struct HostEnt {
    /// `h_addrtype`
    pub addrtype: i32,
    /// `h_addr_list[..]` as network-order s_addr values
    pub addrs: Vec<u32>,
}

/// # Safety
/// `h` is the live hostent Winsock returned on this thread.
unsafe fn read_hostent(h: *const ws::HOSTENT) -> HostEnt {
    // SAFETY: caller contract; h_addr_list is NULL-terminated and each
    // entry has h_length (>= 4 for the families gethostbyname returns)
    // bytes -- read the same four the C driver loads
    unsafe {
        let mut addrs = Vec::new();
        let mut list = (*h).h_addr_list;
        if !list.is_null() {
            while !(*list).is_null() {
                let p = (*list).cast::<u8>();
                addrs.push(u32::from_ne_bytes([*p, *p.add(1), *p.add(2), *p.add(3)]));
                list = list.add(1);
            }
        }
        HostEnt {
            addrtype: i32::from((*h).h_addrtype),
            addrs,
        }
    }
}

/// `gethostbyname (name)`; Err = `WSAGetLastError ()` after a NULL result
pub fn host_by_name(name: &[u8]) -> Result<HostEnt, i32> {
    let cname = cstring(name);
    // SAFETY: cname is NUL-terminated; the hostent is thread-local and read
    // before any other Winsock call
    unsafe {
        let h = ws::gethostbyname(cname.as_ptr());
        if h.is_null() {
            return Err(last_error());
        }
        Ok(read_hostent(h))
    }
}

/// `gethostbyaddr (&sin_addr, sizeof (struct in_addr), AF_INET)`: the
/// `h_name` bytes, or None
pub fn gethostbyaddr4(s_addr: u32) -> Option<Vec<u8>> {
    let bytes = s_addr.to_ne_bytes();
    // SAFETY: bytes is a live 4-byte local; h_name is NUL-terminated
    unsafe {
        let h = ws::gethostbyaddr(bytes.as_ptr(), 4, i32::from(ws::AF_INET));
        if h.is_null() || (*h).h_name.is_null() {
            return None;
        }
        Some(
            core::ffi::CStr::from_ptr((*h).h_name.cast())
                .to_bytes()
                .to_vec(),
        )
    }
}

fn cstring(bytes: &[u8]) -> Vec<u8> {
    let mut v = bytes.to_vec();
    v.push(0);
    v
}

/// # Safety
/// `ai` is a live addrinfo node with `ai_addrlen` bytes at `ai_addr`.
unsafe fn copy_sockaddr(ai: *const ws::ADDRINFOA) -> QSockAddr {
    let mut addr = QSockAddr::zeroed();
    // SAFETY: caller contract; the copy is clamped to the out struct
    unsafe {
        core::ptr::copy_nonoverlapping(
            (*ai).ai_addr.cast::<u8>(),
            (&raw mut addr).cast::<u8>(),
            (*ai).ai_addrlen.min(mem::size_of::<QSockAddr>()),
        );
    }
    addr
}

/// the `getaddrinfo` call of `WINIPv6_GetAddrFromName`: SOCK_DGRAM/UDP
/// hints, any family requested, first AF_INET6 result copied out raw.
/// Err(code) = getaddrinfo failed; Ok(None) = it succeeded with no
/// AF_INET6 result -- C retries only on the former.
pub fn getaddrinfo_pick6(node: &[u8], service: Option<&[u8]>) -> Result<Option<QSockAddr>, i32> {
    let cnode = cstring(node);
    let cserv = service.map(cstring);
    // SAFETY: hints zeroed then filled; result list freed after the copy
    unsafe {
        let mut hints: ws::ADDRINFOA = mem::zeroed();
        hints.ai_family = 0;
        hints.ai_socktype = ws::SOCK_DGRAM;
        hints.ai_protocol = ws::IPPROTO_UDP;
        let mut res: *mut ws::ADDRINFOA = core::ptr::null_mut();
        let err = ws::getaddrinfo(
            cnode.as_ptr(),
            cserv
                .as_ref()
                .map_or(core::ptr::null(), std::vec::Vec::as_ptr),
            &hints,
            &mut res,
        );
        if err != 0 {
            return Err(err);
        }
        let mut out = None;
        let mut pos = res;
        while !pos.is_null() {
            if (*pos).ai_family == i32::from(ws::AF_INET6) && out.is_none() {
                out = Some(copy_sockaddr(pos));
            }
            pos = (*pos).ai_next;
        }
        ws::freeaddrinfo(res);
        Ok(out)
    }
}

/// `WINIPv6_GetLocalAddress`' lookup: AF_INET6/SOCK_DGRAM/UDP hints on the
/// host name, the FIRST result's `ai_addr` (whatever its family, like C);
/// Err = `WSAGetLastError ()` after the failure
pub fn getaddrinfo_first6(node: &[u8]) -> Result<QSockAddr, i32> {
    let cnode = cstring(node);
    // SAFETY: hints zeroed then filled; result list freed after the copy
    unsafe {
        let mut hints: ws::ADDRINFOA = mem::zeroed();
        hints.ai_family = i32::from(ws::AF_INET6);
        hints.ai_socktype = ws::SOCK_DGRAM;
        hints.ai_protocol = ws::IPPROTO_UDP;
        let mut res: *mut ws::ADDRINFOA = core::ptr::null_mut();
        if ws::getaddrinfo(cnode.as_ptr(), core::ptr::null(), &hints, &mut res) != 0
            || res.is_null()
        {
            return Err(last_error());
        }
        let addr = copy_sockaddr(res);
        ws::freeaddrinfo(res);
        Ok(addr)
    }
}

/// `WINIPv6_GetAddresses`' lookup: AF_INET6/SOCK_DGRAM hints (protocol 0)
/// on the host name, every AF_INET6 result; None when getaddrinfo failed
pub fn getaddrinfo_all6(node: &[u8]) -> Option<Vec<QSockAddr>> {
    let cnode = cstring(node);
    // SAFETY: hints zeroed then filled; result list freed after the copy
    unsafe {
        let mut hints: ws::ADDRINFOA = mem::zeroed();
        hints.ai_family = i32::from(ws::AF_INET6);
        hints.ai_socktype = ws::SOCK_DGRAM;
        let mut res: *mut ws::ADDRINFOA = core::ptr::null_mut();
        if ws::getaddrinfo(cnode.as_ptr(), core::ptr::null(), &hints, &mut res) != 0 {
            return None;
        }
        let mut out = Vec::new();
        let mut pos = res;
        while !pos.is_null() {
            if !(*pos).ai_addr.is_null() && (*(*pos).ai_addr).sa_family == ws::AF_INET6 {
                out.push(copy_sockaddr(pos));
            }
            pos = (*pos).ai_next;
        }
        ws::freeaddrinfo(res);
        Some(out)
    }
}

/// `inet_addr (cp)`: network-order address, `INADDR_NONE` on failure
pub fn inet_addr(cp: &[u8]) -> u32 {
    let c = cstring(cp);
    // SAFETY: c is NUL-terminated
    unsafe { ws::inet_addr(c.as_ptr()) }
}

/// `INADDR_NONE`
pub const INADDR_NONE: u32 = ws::INADDR_NONE;
/// `INADDR_ANY`
pub const INADDR_ANY: u32 = ws::INADDR_ANY;
/// `INADDR_BROADCAST`
pub const INADDR_BROADCAST: u32 = ws::INADDR_BROADCAST;
/// `INADDR_LOOPBACK` (host order, like the header)
pub const INADDR_LOOPBACK: u32 = ws::INADDR_LOOPBACK;
/// `NET_EWOULDBLOCK`
pub const EWOULDBLOCK: i32 = ws::WSAEWOULDBLOCK;
/// `NET_ECONNREFUSED`
pub const ECONNREFUSED: i32 = ws::WSAECONNREFUSED;
/// `WSAECONNRESET`
pub const ECONNRESET: i32 = ws::WSAECONNRESET;
