//! The ADR-004 unsafe island of the UDP landriver: every OS call of
//! net_udp.c, wrapped over `socket2` (creation/options, ADR-003) and `libc`
//! (the data path, ioctls and resolution -- kept as the *same* libc calls
//! the C driver made, so their observable behavior is inherited rather than
//! reimplemented). No engine state in here; quake-capi owns that.

use core::ffi::CStr;
use std::io;
use std::mem;
use std::net::{Ipv6Addr, SocketAddrV4, SocketAddrV6};
use std::os::fd::{FromRawFd, IntoRawFd};

use quake_types::net::QSockAddr;
use socket2::{Domain, Protocol, Socket, Type};

/// unix INVALID_SOCKET
pub const INVALID: i32 = -1;

fn errno_of(e: &io::Error) -> i32 {
    e.raw_os_error().unwrap_or(0)
}

/// `socketerror (err)`: strerror text
pub fn strerror(errno: i32) -> String {
    // SAFETY: strerror returns a NUL-terminated static/thread-local string
    unsafe {
        let p = libc::strerror(errno);
        if p.is_null() {
            format!("errno {errno}")
        } else {
            CStr::from_ptr(p).to_string_lossy().into_owned()
        }
    }
}

/// `UDP4_OpenSocket` minus the console prints: Err carries (errno,
/// socket-was-created) so the caller can print and mirror the close
pub fn open_socket4(port: u16) -> Result<i32, (i32, Option<i32>)> {
    let sock = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))
        .map_err(|e| (errno_of(&e), None))?;
    let fd = sock.into_raw_fd();
    // SAFETY: fd was just detached; re-wrapped for the option calls and
    // detached again on every exit path
    let sock = unsafe { Socket::from_raw_fd(fd) };
    let r = sock
        .set_nonblocking(true)
        .and_then(|()| sock.bind(&SocketAddrV4::new(std::net::Ipv4Addr::UNSPECIFIED, port).into()));
    match r {
        Ok(()) => Ok(sock.into_raw_fd()),
        Err(e) => {
            let err = errno_of(&e);
            mem::forget(sock); // the caller closes through close_socket like C
            Err((err, Some(fd)))
        }
    }
}

/// `UDP6_OpenSocket` minus the prints: v6only + nonblocking + bind + the
/// ff03::1 multicast join (option errors ignored exactly like C)
pub fn open_socket6(port: u16) -> Result<i32, (i32, Option<i32>)> {
    let sock = Socket::new(Domain::IPV6, Type::DGRAM, Some(Protocol::UDP))
        .map_err(|e| (errno_of(&e), None))?;
    let _ = sock.set_only_v6(true);
    let r = sock.set_nonblocking(true).and_then(|()| {
        sock.bind(&SocketAddrV6::new(std::net::Ipv6Addr::UNSPECIFIED, port, 0, 0).into())
    });
    match r {
        Ok(()) => {
            let group = Ipv6Addr::new(0xff03, 0, 0, 0, 0, 0, 0, 1);
            let _ = sock.join_multicast_v6(&group, 0);
            Ok(sock.into_raw_fd())
        }
        Err(e) => {
            let err = errno_of(&e);
            let fd = sock.into_raw_fd();
            Err((err, Some(fd)))
        }
    }
}

/// `closesocket`
pub fn close_socket(fd: i32) -> i32 {
    // SAFETY: fd is a socket owned by the driver
    unsafe { libc::close(fd) }
}

/// `recvfrom (socketid, buf, len, 0, (struct sockaddr *)addr, &addrlen)`
/// with `addrlen = sizeof (struct qsockaddr)`; the out address is zeroed
/// first (C left stale caller-stack bytes past addrlen -- COMPAT-noted
/// deterministic divergence). Returns (ret, addr, errno-when-negative).
pub fn recvfrom(fd: i32, buf: &mut [u8]) -> (isize, QSockAddr, i32) {
    let mut addr = QSockAddr::zeroed();
    let mut addrlen: libc::socklen_t = mem::size_of::<QSockAddr>() as libc::socklen_t;
    // SAFETY: buf and addr are live locals of the stated sizes
    unsafe {
        let ret = libc::recvfrom(
            fd,
            buf.as_mut_ptr().cast(),
            buf.len(),
            0,
            (&raw mut addr).cast(),
            &mut addrlen,
        );
        let err = if ret < 0 { errno_value() } else { 0 };
        (ret, addr, err)
    }
}

/// `sendto (socketid, buf, len, 0, (struct sockaddr *)addr, addrsize)`
pub fn sendto(fd: i32, buf: &[u8], addr: &QSockAddr, addrsize: u32) -> (isize, i32) {
    // SAFETY: buf/addr are live; addrsize <= sizeof (QSockAddr)
    unsafe {
        let ret = libc::sendto(
            fd,
            buf.as_ptr().cast(),
            buf.len(),
            0,
            (addr as *const QSockAddr).cast(),
            addrsize as libc::socklen_t,
        );
        let err = if ret < 0 { errno_value() } else { 0 };
        (ret, err)
    }
}

fn errno_value() -> i32 {
    io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

/// `ioctl (sock, FIONREAD, &available)`
pub fn available(fd: i32) -> Result<i32, i32> {
    let mut avail: libc::c_int = 0;
    // SAFETY: FIONREAD writes an int
    let r = unsafe { libc::ioctl(fd, libc::FIONREAD, &mut avail) };
    if r == -1 {
        Err(errno_value())
    } else {
        Ok(avail)
    }
}

/// the "quietly absorb empty packets" zero-length recvfrom.
/// COMPAT: the C v4 variant passed an UNINITIALIZED `fromlen` (UB; the v6
/// variant initializes it) -- the port always passes sizeof(sockaddr_in).
pub fn absorb_empty(fd: i32) {
    let mut from: libc::sockaddr_in = // SAFETY: plain data out-param
        unsafe { mem::zeroed() };
    let mut fromlen: libc::socklen_t = mem::size_of::<libc::sockaddr_in>() as libc::socklen_t;
    let mut buf = [0u8; 1];
    // SAFETY: mirrors the C call (len 0)
    unsafe {
        libc::recvfrom(
            fd,
            buf.as_mut_ptr().cast(),
            0,
            0,
            (&raw mut from).cast(),
            &mut fromlen,
        );
    }
}

/// `setsockopt (socketid, SOL_SOCKET, SO_BROADCAST, ...)`
pub fn set_broadcast(fd: i32) -> Result<(), i32> {
    // SAFETY: fd is a socket; re-wrapped without taking ownership
    let sock = unsafe { Socket::from_raw_fd(fd) };
    let r = sock.set_broadcast(true);
    mem::forget(sock);
    r.map_err(|e| errno_of(&e))
}

/// `getsockname` into a zeroed qsockaddr (the C caller memsets first)
pub fn getsockname(fd: i32) -> Option<QSockAddr> {
    let mut addr = QSockAddr::zeroed();
    let mut addrlen: libc::socklen_t = mem::size_of::<QSockAddr>() as libc::socklen_t;
    // SAFETY: addr is a live 64-byte out-param
    let r = unsafe { libc::getsockname(fd, (&raw mut addr).cast(), &mut addrlen) };
    if r != 0 {
        None
    } else {
        Some(addr)
    }
}

/// `gethostname (buff, MAXHOSTNAMELEN)` with the C truncation
pub fn gethostname(maxlen: usize) -> Result<Vec<u8>, i32> {
    let mut buf = vec![0u8; maxlen];
    // SAFETY: buf is maxlen bytes
    let r = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), maxlen) };
    if r != 0 {
        return Err(errno_value());
    }
    buf[maxlen - 1] = 0;
    let n = buf.iter().position(|&c| c == 0).unwrap_or(maxlen);
    buf.truncate(n);
    Ok(buf)
}

// the legacy resolver API is not in the libc crate; declared here against
// the platform libc (identical prototypes on glibc/musl/libSystem)
#[repr(C)]
struct HostEnt {
    h_name: *mut libc::c_char,
    h_aliases: *mut *mut libc::c_char,
    h_addrtype: libc::c_int,
    h_length: libc::c_int,
    h_addr_list: *mut *mut libc::c_char,
}
extern "C" {
    fn gethostbyname(name: *const libc::c_char) -> *mut HostEnt;
    fn gethostbyaddr(
        addr: *const libc::c_void,
        len: libc::socklen_t,
        ty: libc::c_int,
    ) -> *mut HostEnt;
    fn hstrerror(err: libc::c_int) -> *const libc::c_char;
}

/// `gethostbyname` restricted to the driver's use: Ok(first IPv4 address in
/// network byte order), Err(hstrerror text) on lookup failure,
/// Err(special) when the family is not AF_INET.
pub enum HostByName {
    V4(u32),
    NotInet,
    Failed(String),
}

/// see [`HostByName`]
pub fn host_by_name(name: &[u8]) -> HostByName {
    let mut cname = name.to_vec();
    cname.push(0);
    // SAFETY: cname is NUL-terminated; hostent is thread-local libc storage
    // read immediately
    unsafe {
        let he = gethostbyname(cname.as_ptr().cast());
        if he.is_null() {
            let hp = hstrerror(*h_errno_location());
            let msg = if hp.is_null() {
                "unknown".into()
            } else {
                CStr::from_ptr(hp).to_string_lossy().into_owned()
            };
            return HostByName::Failed(msg);
        }
        if (*he).h_addrtype != libc::AF_INET {
            return HostByName::NotInet;
        }
        let list = (*he).h_addr_list;
        let first = *list;
        let mut a = [0u8; 4];
        core::ptr::copy_nonoverlapping(first.cast::<u8>(), a.as_mut_ptr(), 4);
        HostByName::V4(u32::from_ne_bytes(a))
    }
}

// h_errno is not exposed uniformly by the libc crate
#[cfg(target_os = "macos")]
unsafe fn h_errno_location() -> *mut libc::c_int {
    extern "C" {
        // netdb.h: a plain global on libSystem
        static mut h_errno: libc::c_int;
    }
    &raw mut h_errno
}
#[cfg(not(target_os = "macos"))]
unsafe fn h_errno_location() -> *mut libc::c_int {
    extern "C" {
        fn __h_errno_location() -> *mut libc::c_int;
    }
    // SAFETY: glibc/musl h_errno accessor
    unsafe { __h_errno_location() }
}

/// `gethostbyaddr` reverse lookup of an IPv4 address (network byte order):
/// Some(h_name) on success
pub fn gethostbyaddr4(s_addr: u32) -> Option<Vec<u8>> {
    let addr = s_addr.to_ne_bytes();
    // SAFETY: 4-byte in_addr; hostent storage read immediately
    unsafe {
        let he = gethostbyaddr(addr.as_ptr().cast(), 4, libc::AF_INET);
        if he.is_null() {
            return None;
        }
        Some(CStr::from_ptr((*he).h_name).to_bytes().to_vec())
    }
}

/// the `getaddrinfo` call of `UDP6_GetAddrFromName`: SOCK_DGRAM/UDP hints,
/// any family requested, first AF_INET6 result copied out raw.
/// Err(code) = getaddrinfo failed; Ok(None) = it succeeded with no
/// AF_INET6 result -- C retries only on the former.
pub fn getaddrinfo_pick6(node: &[u8], service: Option<&[u8]>) -> Result<Option<QSockAddr>, i32> {
    let mut cnode = node.to_vec();
    cnode.push(0);
    let cserv = service.map(|s| {
        let mut v = s.to_vec();
        v.push(0);
        v
    });
    // SAFETY: hints zeroed then filled; result list freed after the copy
    unsafe {
        let mut hints: libc::addrinfo = mem::zeroed();
        hints.ai_family = 0;
        hints.ai_socktype = libc::SOCK_DGRAM;
        hints.ai_protocol = libc::IPPROTO_UDP;
        let mut res: *mut libc::addrinfo = core::ptr::null_mut();
        let err = libc::getaddrinfo(
            cnode.as_ptr().cast(),
            cserv
                .as_ref()
                .map_or(core::ptr::null(), |v| v.as_ptr().cast()),
            &hints,
            &mut res,
        );
        if err != 0 {
            return Err(err);
        }
        let mut out = None;
        let mut pos = res;
        while !pos.is_null() {
            if (*pos).ai_family == libc::AF_INET6 && out.is_none() {
                let mut addr = QSockAddr::zeroed();
                core::ptr::copy_nonoverlapping(
                    (*pos).ai_addr.cast::<u8>(),
                    (&raw mut addr).cast::<u8>(),
                    ((*pos).ai_addrlen as usize).min(mem::size_of::<QSockAddr>()),
                );
                out = Some(addr);
            }
            pos = (*pos).ai_next;
        }
        libc::freeaddrinfo(res);
        Ok(out)
    }
}

/// linux `getifaddrs` enumeration (`UDP_GetAddresses`); raw sockaddr bytes
/// per interface of the requested family. Non-linux unix returns none,
/// exactly like the C `#else` stub.
#[cfg(target_os = "linux")]
pub fn interface_addrs(fam: i32) -> Vec<QSockAddr> {
    let mut out = Vec::new();
    // SAFETY: getifaddrs list freed after the copy loop
    unsafe {
        let mut list: *mut libc::ifaddrs = core::ptr::null_mut();
        if libc::getifaddrs(&mut list) != 0 {
            return out;
        }
        let mut ifa = list;
        while !ifa.is_null() {
            let sa = (*ifa).ifa_addr;
            if !sa.is_null() && i32::from((*sa).sa_family) == fam {
                let len = match fam {
                    x if x == libc::AF_INET => mem::size_of::<libc::sockaddr_in>(),
                    _ => mem::size_of::<libc::sockaddr_in6>(),
                };
                let mut addr = QSockAddr::zeroed();
                core::ptr::copy_nonoverlapping(
                    sa.cast::<u8>(),
                    (&raw mut addr).cast::<u8>(),
                    len.min(mem::size_of::<QSockAddr>()),
                );
                out.push(addr);
            }
            ifa = (*ifa).ifa_next;
        }
        libc::freeifaddrs(list);
    }
    out
}

/// see the linux variant
#[cfg(not(target_os = "linux"))]
pub fn interface_addrs(_fam: i32) -> Vec<QSockAddr> {
    Vec::new()
}
