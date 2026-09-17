//! Phase 5 M7b: the UDP landriver, transliterated from `Quake/net_udp.c`
//! (the unix personality). Phase 9 M2 adds the `Quake/net_wins.c`
//! personality: the same pure helpers where the two C drivers agree, and
//! `*_wins` variants where net_wins.c's semantics differ (its
//! `AddrToString` honours `masked` and treats every non-AF_INET6 family as
//! IPv4; its `AddrCompare`/`GetSocketPort`/`SetSocketPort` have no
//! unknown-family branch). Both sets are pure and built on every target so
//! they can be unit-tested anywhere; only [`sys`] talks to the OS.
//!
//! Layout note: `struct qsockaddr` is a 64-byte blob punned to
//! `sockaddr_in`/`sockaddr_in6`. On every supported unix and on Winsock
//! both layouts put the port at byte offset 2 (big-endian), the IPv4
//! address at 4, the IPv6 address at 8 and the scope id at 24
//! (native-endian) -- i.e. at offsets 0/2/6/22 of the mirror's `qsa_data`.

use core::ffi::c_ulong;

use crate::cnum::c_atoi;
use quake_types::net::QSockAddr;

/// `MAXHOSTNAMELEN` (net_sys.h). Observable, not an implementation detail:
/// `UDP4_GetAddrFromName` rejects any `host:port` whose host part reaches
/// it, so a drift from the C build's `<sys/param.h>` value would change
/// which hostnames are connectable. Pinned against the engine headers by
/// `quake-ctest/tests/net_abi.rs`.
#[cfg(target_os = "linux")]
pub const MAXHOSTNAMELEN: usize = 64;
/// see the linux variant
#[cfg(not(target_os = "linux"))]
pub const MAXHOSTNAMELEN: usize = 256;

/// libc's AF_INET / AF_INET6 for this target (pure module: constants only)
#[cfg(unix)]
pub const AF_INET: i32 = libc::AF_INET;
/// see [`AF_INET`]
#[cfg(unix)]
pub const AF_INET6: i32 = libc::AF_INET6;
/// Winsock's AF_INET / AF_INET6 (pure module: constants only)
#[cfg(windows)]
pub const AF_INET: i32 = windows_sys::Win32::Networking::WinSock::AF_INET as i32;
/// see [`AF_INET`]
#[cfg(windows)]
pub const AF_INET6: i32 = windows_sys::Win32::Networking::WinSock::AF_INET6 as i32;

/// `addr->qsa_family` through the platform ladder
pub fn family(addr: &QSockAddr) -> i32 {
    addr.qsa_family as i32
}

/// sets `addr->qsa_family` (and zeroes qsa_len where it exists, matching
/// the C code's plain `qsa_family = AF_INET` on a zeroed struct)
pub fn set_family(addr: &mut QSockAddr, fam: i32) {
    #[cfg(any(
        target_os = "macos",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "dragonfly"
    ))]
    {
        addr.qsa_family = fam as u8;
    }
    #[cfg(not(any(
        target_os = "macos",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "dragonfly"
    )))]
    {
        addr.qsa_family = fam as i16;
    }
}

fn port_be(addr: &QSockAddr) -> u16 {
    u16::from_be_bytes([addr.qsa_data[0], addr.qsa_data[1]])
}

fn set_port_be(addr: &mut QSockAddr, port: u16) {
    addr.qsa_data[0..2].copy_from_slice(&port.to_be_bytes());
}

fn v4_addr(addr: &QSockAddr) -> [u8; 4] {
    [
        addr.qsa_data[2],
        addr.qsa_data[3],
        addr.qsa_data[4],
        addr.qsa_data[5],
    ]
}

fn v6_addr(addr: &QSockAddr) -> [u8; 16] {
    let mut a = [0u8; 16];
    a.copy_from_slice(&addr.qsa_data[6..22]);
    a
}

fn v6_scope(addr: &QSockAddr) -> u32 {
    u32::from_ne_bytes([
        addr.qsa_data[22],
        addr.qsa_data[23],
        addr.qsa_data[24],
        addr.qsa_data[25],
    ])
}

/// `UDP_AddrToString` (the `masked` parameter is ignored by the C original)
pub fn addr_to_string(addr: &QSockAddr) -> String {
    if family(addr) == AF_INET {
        let a = v4_addr(addr);
        let haddr = u32::from_be_bytes(a);
        format!(
            "{}.{}.{}.{}:{}",
            (haddr >> 24) & 0xff,
            (haddr >> 16) & 0xff,
            (haddr >> 8) & 0xff,
            haddr & 0xff,
            port_be(addr)
        )
    } else if family(addr) == AF_INET6 {
        // evil type punning: eight ntohs'd u16 groups
        let a = v6_addr(addr);
        let s: Vec<u16> = a
            .chunks_exact(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        let scope = v6_scope(addr);
        if scope != 0 {
            format!(
                "[{:x}:{:x}:{:x}:{:x}:{:x}:{:x}:{:x}:{:x}%{}]:{}",
                s[0],
                s[1],
                s[2],
                s[3],
                s[4],
                s[5],
                s[6],
                s[7],
                scope as i32,
                port_be(addr)
            )
        } else {
            format!(
                "[{:x}:{:x}:{:x}:{:x}:{:x}:{:x}:{:x}:{:x}]:{}",
                s[0],
                s[1],
                s[2],
                s[3],
                s[4],
                s[5],
                s[6],
                s[7],
                port_be(addr)
            )
        }
    } else {
        "?".into()
    }
}

/// `sscanf(.., "%d")`-style single conversion: (value, bytes consumed);
/// None when no digits matched
fn scan_d(s: &[u8]) -> Option<(i32, usize)> {
    let mut i = 0;
    while i < s.len() && (s[i] == b' ' || (0x09..=0x0d).contains(&s[i])) {
        i += 1;
    }
    let mut sign = 1i64;
    if i < s.len() && (s[i] == b'+' || s[i] == b'-') {
        if s[i] == b'-' {
            sign = -1;
        }
        i += 1;
    }
    let start = i;
    let mut v: i64 = 0;
    while i < s.len() && s[i].is_ascii_digit() {
        v = (v * 10 + (s[i] - b'0') as i64).clamp(i64::MIN / 2, i64::MAX / 2);
        i += 1;
    }
    if i == start {
        return None;
    }
    Some(((sign * v).clamp(i32::MIN as i64, i32::MAX as i64) as i32, i))
}

/// `PartialIPAddress`: dotted-partial address completed from `my_addr4`
/// (network byte order, as the C static holds it). Returns the address or
/// None (-1). COMPAT: the out-struct's tail is zero-filled (C left the
/// caller's stack bytes) -- see `string_to_addr4`.
pub fn partial_ip_address(input: &[u8], my_addr4: u32, net_hostport: i32) -> Option<QSockAddr> {
    let mut buff = Vec::with_capacity(input.len() + 2);
    buff.push(b'.');
    buff.extend_from_slice(input);
    let mut b = 0usize;
    if buff.len() > 1 && buff[1] == b'.' {
        b += 1;
    }

    let mut addr: i32 = 0;
    let mut mask: i32 = -1;
    while b < buff.len() && buff[b] == b'.' {
        b += 1;
        let mut num: i32 = 0;
        let mut run = 0;
        while b < buff.len() && buff[b].is_ascii_digit() {
            num = num * 10 + (buff[b] - b'0') as i32;
            b += 1;
            run += 1;
            if run > 3 {
                return None;
            }
        }
        let c = if b < buff.len() { buff[b] } else { 0 };
        if !c.is_ascii_digit() && c != b'.' && c != b':' && c != 0 {
            return None;
        }
        if !(0..=255).contains(&num) {
            return None;
        }
        mask <<= 8;
        addr = (addr << 8) + num;
    }

    let port = if b < buff.len() && buff[b] == b':' {
        c_atoi(&buff[b + 1..])
    } else {
        net_hostport
    };

    let mut out = QSockAddr::zeroed();
    set_family(&mut out, AF_INET);
    set_port_be(&mut out, port as u16);
    // (myAddr4 & htonl(mask)) | htonl(addr) -- all in network byte order
    let net = (my_addr4 & (mask as u32).to_be()) | (addr as u32).to_be();
    out.qsa_data[2..6].copy_from_slice(&net.to_ne_bytes());
    Some(out)
}

/// `UDP4_StringToAddr`. COMPAT: the C sscanf leaves the outputs
/// uninitialized on a partial match (UB); missing conversions read as 0
/// here. Always returns the address like C returns 0. COMPAT: C wrote only
/// family/port/addr into the caller's (uninitialized stack) qsockaddr; the
/// port zero-fills the remainder -- unobservable (no whole-struct consumer
/// sees these addresses) but recorded.
pub fn string_to_addr4(s: &[u8]) -> QSockAddr {
    let mut vals = [0i32; 5];
    let mut pos = 0usize;
    let pattern: [(usize, u8); 4] = [(0, b'.'), (1, b'.'), (2, b'.'), (3, b':')];
    let mut n = 0usize;
    'scan: {
        for (idx, sep) in pattern {
            match scan_d(&s[pos..]) {
                Some((v, used)) => {
                    vals[idx] = v;
                    pos += used;
                    n = idx + 1;
                }
                None => break 'scan,
            }
            if pos < s.len() && s[pos] == sep {
                pos += 1;
            } else {
                break 'scan;
            }
        }
        if let Some((v, _)) = scan_d(&s[pos..]) {
            vals[4] = v;
            n = 5;
        }
    }
    let _ = n;
    let ipaddr = (vals[0] << 24) | (vals[1] << 16) | (vals[2] << 8) | vals[3];
    let mut out = QSockAddr::zeroed();
    set_family(&mut out, AF_INET);
    out.qsa_data[2..6].copy_from_slice(&(ipaddr as u32).to_be().to_ne_bytes());
    set_port_be(&mut out, vals[4] as u16);
    out
}

/// `UDP_AddrCompare`
pub fn addr_compare(a: &QSockAddr, b: &QSockAddr) -> i32 {
    if family(a) != family(b) {
        return -1;
    }
    if family(a) == AF_INET {
        if v4_addr(a) != v4_addr(b) {
            return -1;
        }
        if a.qsa_data[0..2] != b.qsa_data[0..2] {
            return 1;
        }
        0
    } else if family(a) == AF_INET6 {
        if v6_addr(a) != v6_addr(b) {
            return -1;
        }
        if a.qsa_data[0..2] != b.qsa_data[0..2] {
            return 1;
        }
        if v6_scope(a) != 0 && v6_scope(b) != 0 && v6_scope(a) != v6_scope(b) {
            return 1;
        }
        0
    } else {
        -1
    }
}

/// `UDP_GetSocketPort`
pub fn get_socket_port(addr: &QSockAddr) -> i32 {
    if family(addr) == AF_INET || family(addr) == AF_INET6 {
        port_be(addr) as i32
    } else {
        -1
    }
}

/// `UDP_SetSocketPort`
pub fn set_socket_port(addr: &mut QSockAddr, port: i32) -> i32 {
    if family(addr) == AF_INET || family(addr) == AF_INET6 {
        set_port_be(addr, port as u16);
        0
    } else {
        -1
    }
}

/// The colon split of `UDP4_GetAddrFromName`'s hostname branch:
/// (host-without-port, Some(port)) via `strrchr` + `strtoul(base 10)`.
/// None when the name is too long for the C `MAXHOSTNAMELEN` dupe buffer.
pub fn split_host_port(name: &[u8], maxhostnamelen: usize) -> Option<(Vec<u8>, Option<u16>)> {
    match name.iter().rposition(|&c| c == b':') {
        Some(colon) => {
            if colon + 1 > maxhostnamelen {
                return None;
            }
            let host = name[..colon].to_vec();
            // strtoul(colon+1, NULL, 10) truncated to unsigned short:
            // leading whitespace, optional sign, digits; overflow returns
            // ULONG_MAX unsigned (glibc and the UCRT agree) and only an
            // in-range '-' value wraps modulo ULONG_MAX+1, then the u16 cut.
            // The accumulator is the target's `unsigned long` because the
            // clamp is width-dependent: 64-bit on LP64 unix, 32-bit on
            // Windows (LLP64) and 32-bit targets, so `host:4294967296` is
            // port 0xFFFF on Windows and 0 on 64-bit Linux
            let s = &name[colon + 1..];
            let mut i = 0;
            while i < s.len() && (s[i] == b' ' || (0x09..=0x0d).contains(&s[i])) {
                i += 1;
            }
            let mut neg = false;
            if i < s.len() && (s[i] == b'+' || s[i] == b'-') {
                neg = s[i] == b'-';
                i += 1;
            }
            let mut v: c_ulong = 0;
            let mut overflow = false;
            while i < s.len() && s[i].is_ascii_digit() {
                match v
                    .checked_mul(10)
                    .and_then(|v| v.checked_add(c_ulong::from(s[i] - b'0')))
                {
                    Some(n) => v = n,
                    None => overflow = true,
                }
                i += 1;
            }
            if overflow {
                v = c_ulong::MAX;
            } else if neg {
                v = v.wrapping_neg();
            }
            Some((host, Some(v as u16)))
        }
        None => Some((name.to_vec(), None)),
    }
}

/// `WINS_AddrToString` (net_wins.c): unlike the unix driver it honours
/// `masked` and falls through to the IPv4 formatting for every family
/// other than AF_INET6 (no "?" branch)
pub fn addr_to_string_wins(addr: &QSockAddr, masked: bool) -> String {
    if family(addr) == AF_INET6 {
        let a = v6_addr(addr);
        let s: Vec<u16> = a
            .chunks_exact(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        if masked {
            return format!("[{:x}:{:x}:{:x}:{:x}::]/64", s[0], s[1], s[2], s[3]);
        }
        let scope = v6_scope(addr);
        if scope != 0 {
            format!(
                "[{:x}:{:x}:{:x}:{:x}:{:x}:{:x}:{:x}:{:x}%{}]:{}",
                s[0],
                s[1],
                s[2],
                s[3],
                s[4],
                s[5],
                s[6],
                s[7],
                scope as i32,
                port_be(addr)
            )
        } else {
            format!(
                "[{:x}:{:x}:{:x}:{:x}:{:x}:{:x}:{:x}:{:x}]:{}",
                s[0],
                s[1],
                s[2],
                s[3],
                s[4],
                s[5],
                s[6],
                s[7],
                port_be(addr)
            )
        }
    } else {
        let haddr = u32::from_be_bytes(v4_addr(addr));
        if masked {
            format!(
                "{}.{}.{}.0/24",
                (haddr >> 24) & 0xff,
                (haddr >> 16) & 0xff,
                (haddr >> 8) & 0xff
            )
        } else {
            format!(
                "{}.{}.{}.{}:{}",
                (haddr >> 24) & 0xff,
                (haddr >> 16) & 0xff,
                (haddr >> 8) & 0xff,
                haddr & 0xff,
                port_be(addr)
            )
        }
    }
}

/// `WINS_AddrCompare` (net_wins.c): family mismatch -1, AF_INET6 like the
/// unix driver, every other family compared as IPv4
pub fn addr_compare_wins(a: &QSockAddr, b: &QSockAddr) -> i32 {
    if family(a) != family(b) {
        return -1;
    }
    if family(a) == AF_INET6 {
        if v6_addr(a) != v6_addr(b) {
            return -1;
        }
        if a.qsa_data[0..2] != b.qsa_data[0..2] {
            return 1;
        }
        if v6_scope(a) != 0 && v6_scope(b) != 0 && v6_scope(a) != v6_scope(b) {
            return 1;
        }
        0
    } else {
        if v4_addr(a) != v4_addr(b) {
            return -1;
        }
        if a.qsa_data[0..2] != b.qsa_data[0..2] {
            return 1;
        }
        0
    }
}

/// `WINS_GetSocketPort` (net_wins.c): the sockaddr_in port for every family
/// but AF_INET6 -- same byte offset, so no branch is observable, but no
/// unknown-family -1 either
pub fn get_socket_port_wins(addr: &QSockAddr) -> i32 {
    port_be(addr) as i32
}

/// `WINS_SetSocketPort` (net_wins.c): always 0
pub fn set_socket_port_wins(addr: &mut QSockAddr, port: i32) -> i32 {
    set_port_be(addr, port as u16);
    0
}

// ADR-004: the one unsafe island of this crate (see lib.rs)
#[allow(unsafe_code)]
pub mod sys;

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_v4(ip: [u8; 4], port: u16) -> QSockAddr {
        let mut a = QSockAddr::zeroed();
        set_family(&mut a, AF_INET);
        set_port_be(&mut a, port);
        a.qsa_data[2..6].copy_from_slice(&ip);
        a
    }

    fn mk_v6(ip: [u8; 16], port: u16, scope: u32) -> QSockAddr {
        let mut a = QSockAddr::zeroed();
        set_family(&mut a, AF_INET6);
        set_port_be(&mut a, port);
        a.qsa_data[6..22].copy_from_slice(&ip);
        a.qsa_data[22..26].copy_from_slice(&scope.to_ne_bytes());
        a
    }

    #[test]
    fn wins_addr_to_string_v4_and_masks() {
        let a = mk_v4([192, 168, 1, 77], 26000);
        assert_eq!(addr_to_string_wins(&a, false), "192.168.1.77:26000");
        assert_eq!(addr_to_string_wins(&a, true), "192.168.1.0/24");
        // net_wins.c: any non-AF_INET6 family takes the IPv4 path
        let mut odd = a;
        set_family(&mut odd, 77);
        assert_eq!(addr_to_string_wins(&odd, false), "192.168.1.77:26000");
        assert_eq!(addr_to_string(&odd), "?");
    }

    #[test]
    fn wins_addr_to_string_v6() {
        let ip = [
            0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0x2f,
        ];
        let a = mk_v6(ip, 26000, 0);
        assert_eq!(
            addr_to_string_wins(&a, false),
            "[2001:db8:0:1:0:0:0:2f]:26000"
        );
        assert_eq!(addr_to_string_wins(&a, true), "[2001:db8:0:1::]/64");
        let b = mk_v6(ip, 26000, 3);
        assert_eq!(
            addr_to_string_wins(&b, false),
            "[2001:db8:0:1:0:0:0:2f%3]:26000"
        );
        assert_eq!(addr_to_string_wins(&b, true), "[2001:db8:0:1::]/64");
        assert_eq!(addr_to_string_wins(&a, false), addr_to_string(&a));
    }

    #[test]
    fn wins_addr_compare_and_ports() {
        let a = mk_v4([10, 0, 0, 1], 26000);
        let b = mk_v4([10, 0, 0, 1], 26001);
        let c = mk_v4([10, 0, 0, 2], 26000);
        assert_eq!(addr_compare_wins(&a, &a), 0);
        assert_eq!(addr_compare_wins(&a, &b), 1);
        assert_eq!(addr_compare_wins(&a, &c), -1);
        let v6 = mk_v6([0; 16], 26000, 0);
        assert_eq!(addr_compare_wins(&a, &v6), -1);
        assert_eq!(addr_compare_wins(&v6, &mk_v6([0; 16], 26000, 4)), 0);
        assert_eq!(
            addr_compare_wins(&mk_v6([0; 16], 1, 2), &mk_v6([0; 16], 1, 4)),
            1
        );
        // unknown family: unix says -1 everywhere, net_wins.c compares as v4
        let mut x = a;
        set_family(&mut x, 0);
        let mut y = b;
        set_family(&mut y, 0);
        assert_eq!(addr_compare(&x, &y), -1);
        assert_eq!(addr_compare_wins(&x, &y), 1);
        assert_eq!(get_socket_port(&x), -1);
        assert_eq!(get_socket_port_wins(&x), 26000);
        assert_eq!(set_socket_port(&mut x, 5), -1);
        assert_eq!(set_socket_port_wins(&mut x, 5), 0);
        assert_eq!(get_socket_port_wins(&x), 5);
    }

    #[test]
    fn split_host_port_strtoul_width() {
        let port = |s: &str| split_host_port(s.as_bytes(), 256).unwrap().1.unwrap();
        assert_eq!(port("h:26000"), 26000);
        assert_eq!(port("h: +70000"), (70000u32 & 0xFFFF) as u16);
        assert_eq!(port("h:-1"), 0xFFFF);
        assert_eq!(port("h:junk"), 0);
        // one past ULONG_MAX: strtoul clamps, so the u16 cut differs by
        // target width -- 0xFFFF where unsigned long is 32-bit, 0 at 64
        let past32 = port("h:4294967296");
        let past64 = port("h:18446744073709551616");
        if c_ulong::BITS == 32 {
            assert_eq!(past32, 0xFFFF);
        } else {
            assert_eq!(past32, 0);
        }
        assert_eq!(past64, 0xFFFF);
        // overflow with a sign is still ULONG_MAX, not its negation
        assert_eq!(port("h:-18446744073709551616"), 0xFFFF);
        assert_eq!(
            port("h:-4294967296"),
            if c_ulong::BITS == 32 { 0xFFFF } else { 0 }
        );
    }
}
