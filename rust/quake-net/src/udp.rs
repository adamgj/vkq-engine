//! Phase 5 M7b: the UDP landriver, transliterated from `Quake/net_udp.c`
//! (the unix personality; the net_wins.c flip is deferred until a Windows
//! UDP runtime CI leg exists, ADR-017 precedent).
//!
//! Layout note: `struct qsockaddr` is a 64-byte blob punned to
//! `sockaddr_in`/`sockaddr_in6`. On every supported unix both layouts put
//! the port at byte offset 2 (big-endian), the IPv4 address at 4, the IPv6
//! address at 8 and the scope id at 24 (native-endian) -- i.e. at offsets
//! 0/2/6/22 of the mirror's `qsa_data`. The pure functions below work on
//! those bytes; only [`sys`] talks to the OS.

use quake_types::net::QSockAddr;

/// libc's AF_INET / AF_INET6 for this target (pure module: constants only)
pub const AF_INET: i32 = libc::AF_INET;
/// see [`AF_INET`]
pub const AF_INET6: i32 = libc::AF_INET6;

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

/// C `atoi`: leading whitespace, optional sign, digits (i64 accumulation --
/// C overflow is UB, clamped here; COMPAT: out-of-int domain accepted
/// divergence)
fn c_atoi(s: &[u8]) -> i32 {
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
    let mut v: i64 = 0;
    while i < s.len() && s[i].is_ascii_digit() {
        v = (v * 10 + (s[i] - b'0') as i64).clamp(i64::MIN / 2, i64::MAX / 2);
        i += 1;
    }
    (sign * v).clamp(i32::MIN as i64, i32::MAX as i64) as i32
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
            // leading whitespace, optional sign ('-' wraps modulo
            // ULONG_MAX+1), digits, clamp at ULONG_MAX, then the u16 cut
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
            let mut v: u64 = 0;
            while i < s.len() && s[i].is_ascii_digit() {
                v = v.saturating_mul(10).saturating_add((s[i] - b'0') as u64);
                i += 1;
            }
            if neg {
                v = v.wrapping_neg();
            }
            Some((host, Some(v as u16)))
        }
        None => Some((name.to_vec(), None)),
    }
}

// ADR-004: the one unsafe island of this crate (see lib.rs)
#[allow(unsafe_code)]
pub mod sys;
