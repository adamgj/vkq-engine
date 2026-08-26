//! Differential test: the pure address functions of the Rust UDP landriver
//! (`quake_net::udp`) vs the C originals in `Quake/net_udp.c` (compiled as
//! `c_ref_*`, unix only). Phase 5 M7b.
//!
//! Scope, precisely: AddrToString, StringToAddr, AddrCompare, Get/Set
//! SocketPort and the PartialIPAddress parse (via UDP4_GetAddrFromName's
//! leading-digit branch) -- everything that transforms bytes without
//! touching a socket or the resolver. The socket/resolver halves are
//! exercised end-to-end by the engine-level capture/record/save gates
//! (identical live sessions over the Rust and C drivers).

#![cfg(unix)]

use core::ffi::{c_char, c_int, CStr};
use std::sync::{Mutex, MutexGuard};

use quake_ctest as _; // links the c_ref archive + stub globals
use quake_net::udp;
use quake_types::net::QSockAddr;

extern "C" {
    fn c_ref_UDP_AddrToString(addr: *mut QSockAddr, masked: bool) -> *const c_char;
    fn c_ref_UDP4_StringToAddr(string: *const c_char, addr: *mut QSockAddr) -> c_int;
    fn c_ref_UDP_AddrCompare(a: *mut QSockAddr, b: *mut QSockAddr) -> c_int;
    fn c_ref_UDP_GetSocketPort(addr: *mut QSockAddr) -> c_int;
    fn c_ref_UDP_SetSocketPort(addr: *mut QSockAddr, port: c_int) -> c_int;
    fn c_ref_UDP4_GetAddrFromName(name: *const c_char, addr: *mut QSockAddr) -> c_int;
    static mut c_ref_net_hostport: c_int;
}

static TEST_LOCK: Mutex<()> = Mutex::new(());

fn lock() -> MutexGuard<'static, ()> {
    TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner())
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }
}

fn addr_bytes(a: &QSockAddr) -> [u8; 64] {
    // SAFETY: repr(C), 64 bytes, plain data
    unsafe { core::mem::transmute_copy(a) }
}

fn mk_v4(ip: [u8; 4], port: u16) -> QSockAddr {
    let mut a = QSockAddr::zeroed();
    udp::set_family(&mut a, udp::AF_INET);
    a.qsa_data[0..2].copy_from_slice(&port.to_be_bytes());
    a.qsa_data[2..6].copy_from_slice(&ip);
    a
}

fn mk_v6(ip: [u8; 16], port: u16, scope: u32) -> QSockAddr {
    let mut a = QSockAddr::zeroed();
    udp::set_family(&mut a, udp::AF_INET6);
    a.qsa_data[0..2].copy_from_slice(&port.to_be_bytes());
    a.qsa_data[6..22].copy_from_slice(&ip);
    a.qsa_data[22..26].copy_from_slice(&scope.to_ne_bytes());
    a
}

fn c_addr_to_string(a: &QSockAddr) -> String {
    let mut c = *a;
    // SAFETY: serialized by TEST_LOCK; the static return buffer is copied out
    unsafe {
        CStr::from_ptr(c_ref_UDP_AddrToString(&mut c, false))
            .to_string_lossy()
            .into_owned()
    }
}

#[test]
fn addr_to_string_matches() {
    let _l = lock();
    let mut cases = vec![
        mk_v4([127, 0, 0, 1], 26000),
        mk_v4([0, 0, 0, 0], 0),
        mk_v4([255, 255, 255, 255], 65535),
        mk_v4([10, 1, 2, 3], 1),
        mk_v6([0; 16], 26000, 0),
        mk_v6([0xff; 16], 12345, 0),
        mk_v6(
            [0xfe, 0x80, 0, 0, 0, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8],
            700,
            3,
        ),
        mk_v6(
            [0, 1, 0, 2, 0, 3, 0, 4, 0, 5, 0, 6, 0, 7, 0, 8],
            0,
            0xffff_fffe,
        ),
        QSockAddr::zeroed(), // family 0 -> "?"
        {
            let mut a = QSockAddr::zeroed();
            udp::set_family(&mut a, 77);
            a
        },
    ];
    let mut rng = Rng(0xDEAD_BEEF_1234_5678);
    for _ in 0..500 {
        if rng.next().is_multiple_of(2) {
            cases.push(mk_v4((rng.next() as u32).to_be_bytes(), rng.next() as u16));
        } else {
            let mut ip = [0u8; 16];
            ip.iter_mut().for_each(|b| *b = rng.next() as u8);
            let scope = if rng.next().is_multiple_of(3) {
                rng.next() as u32
            } else {
                0
            };
            cases.push(mk_v6(ip, rng.next() as u16, scope));
        }
    }
    for (i, a) in cases.iter().enumerate() {
        assert_eq!(c_addr_to_string(a), udp::addr_to_string(a), "case {i}");
    }
}

#[test]
fn string_to_addr_matches() {
    let _l = lock();
    // full-match inputs only: the C sscanf leaves outputs uninitialized on
    // partial matches (UB, COMPAT-documented in quake_net::udp)
    let mut cases: Vec<String> = vec![
        "127.0.0.1:26000".into(),
        "0.0.0.0:0".into(),
        "255.255.255.255:65535".into(),
        "10.0.0.1:700".into(),
        "300.400.500.600:70000".into(), // out-of-range octets still combine
        "-1.-2.-3.-4:-5".into(),
        "1.2.3.4:26000 trailing junk".into(),
    ];
    let mut rng = Rng(0x1234_5678_9ABC_DEF0);
    for _ in 0..500 {
        cases.push(format!(
            "{}.{}.{}.{}:{}",
            rng.next() % 700,
            rng.next() % 700,
            rng.next() % 700,
            rng.next() % 700,
            rng.next() % 100000
        ));
    }
    for s in &cases {
        let mut cs = s.clone().into_bytes();
        cs.push(0);
        let mut ca = QSockAddr::zeroed();
        // SAFETY: serialized by TEST_LOCK; cs NUL-terminated
        let cret = unsafe { c_ref_UDP4_StringToAddr(cs.as_ptr().cast(), &mut ca) };
        let ra = udp::string_to_addr4(s.as_bytes());
        assert_eq!(cret, 0);
        assert_eq!(addr_bytes(&ca), addr_bytes(&ra), "input {s:?}");
    }
}

#[test]
fn addr_compare_and_ports_match() {
    let _l = lock();
    let mut rng = Rng(0xABCD_EF01_2345_6789);
    let mut pool: Vec<QSockAddr> = Vec::new();
    for _ in 0..60 {
        let a = match rng.next() % 4 {
            0 => mk_v4(
                [10, 0, 0, (rng.next() % 3) as u8],
                (rng.next() % 3) as u16 * 7,
            ),
            1 => mk_v4([10, 0, 0, 1], 26000),
            2 => {
                let mut ip = [0u8; 16];
                ip[15] = (rng.next() % 2) as u8;
                mk_v6(ip, (rng.next() % 2) as u16 * 9, (rng.next() % 3) as u32)
            }
            _ => {
                let mut a = QSockAddr::zeroed();
                udp::set_family(&mut a, (rng.next() % 5) as i32);
                a
            }
        };
        pool.push(a);
    }
    for x in &pool {
        for y in &pool {
            let mut cx = *x;
            let mut cy = *y;
            // SAFETY: serialized by TEST_LOCK
            let cret = unsafe { c_ref_UDP_AddrCompare(&mut cx, &mut cy) };
            assert_eq!(cret, udp::addr_compare(x, y));
        }
        let mut cx = *x;
        // SAFETY: serialized by TEST_LOCK
        let cport = unsafe { c_ref_UDP_GetSocketPort(&mut cx) };
        assert_eq!(cport, udp::get_socket_port(x));

        let mut cs = *x;
        let mut rs = *x;
        let p = (rng.next() % 70000) as c_int;
        // SAFETY: serialized by TEST_LOCK
        let cret = unsafe { c_ref_UDP_SetSocketPort(&mut cs, p) };
        let rret = udp::set_socket_port(&mut rs, p);
        assert_eq!(cret, rret);
        assert_eq!(addr_bytes(&cs), addr_bytes(&rs));
    }
}

#[test]
fn partial_ip_address_matches() {
    let _l = lock();
    // via UDP4_GetAddrFromName's leading-digit branch. The c_ref myAddr4
    // static is 0 (UDP4_Init never runs in tests); the Rust side gets the
    // same value explicitly.
    let cases: &[&str] = &[
        "1.2.3.4",
        "1.2.3.4:26001",
        "12.13",
        "12.13:700",
        "1",
        "1:080",
        "5.6.7.8.9", // 5 components: mask keeps shifting
        "999.1",     // >255 component -> -1
        "1234.1",    // >3 digit run -> -1
        "1.2.x",     // bad terminator -> -1
        "3...4",
        "9:",
        "9:junk",
        "0.0.0.0",
        "255.255.255.255:65535",
        "1.2:70000",
    ];
    for s in cases {
        let mut cs = s.as_bytes().to_vec();
        cs.push(0);
        let mut ca = QSockAddr::zeroed();
        // SAFETY: serialized by TEST_LOCK
        let cret = unsafe { c_ref_UDP4_GetAddrFromName(cs.as_ptr().cast(), &mut ca) };
        // SAFETY: serialized by TEST_LOCK
        let hostport = unsafe { c_ref_net_hostport };
        let r = udp::partial_ip_address(s.as_bytes(), 0, hostport);
        match r {
            None => assert_eq!(cret, -1, "input {s:?}"),
            Some(ra) => {
                assert_eq!(cret, 0, "input {s:?}");
                assert_eq!(addr_bytes(&ca), addr_bytes(&ra), "input {s:?}");
            }
        }
    }
}

#[test]
fn split_host_port_bound_matches_c_guard() {
    let _l = lock();
    // the C guard: colon - name + 1 > MAXHOSTNAMELEN -> -1 (resolver never
    // invoked). Reproduce with a long non-numeric name; both sides must
    // refuse without resolving.
    let long = format!("{}:26000", "h".repeat(400));
    let mut cs = long.clone().into_bytes();
    cs.push(0);
    let mut ca = QSockAddr::zeroed();
    // SAFETY: serialized by TEST_LOCK
    let cret = unsafe { c_ref_UDP4_GetAddrFromName(cs.as_ptr().cast(), &mut ca) };
    assert_eq!(cret, -1);
    #[cfg(target_os = "linux")]
    const MAXHOSTNAMELEN: usize = 64;
    #[cfg(not(target_os = "linux"))]
    const MAXHOSTNAMELEN: usize = 256;
    assert!(udp::split_host_port(long.as_bytes(), MAXHOSTNAMELEN).is_none());
}
