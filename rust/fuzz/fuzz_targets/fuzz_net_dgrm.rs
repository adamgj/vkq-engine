//! Datagram reliable-layer fuzzer (Phase 5 M6, ADR-019 gate 5): drives the
//! two RX paths (`get_message`, `process_packet`) and the send paths of
//! `quake_net::dgrm` with fuzzer-shaped packet streams over a mock NetSys.
//! Asserts no panics and the state invariants a hostile peer must not be
//! able to break. The C-vs-Rust differential is quake-ctest's
//! net_dgrm_differential (same design decision as the earlier targets).

#![no_main]

use std::collections::VecDeque;

use libfuzzer_sys::fuzz_target;
use quake_net::dgrm::{self, DgrmCounters, DgrmGlobals, NetSys, PACKET_BUFFER_SIZE};
use quake_types::net::{QSockAddr, QSocket, SysSocket};

const NM_MAXSIZE: usize = 64000;

enum RxEvent {
    Packet(Vec<u8>, [u8; 64]),
    Error,
}

#[derive(Default)]
struct FuzzSys {
    rx: VecDeque<RxEvent>,
    writes: usize,
}

fn qs_addr(b: [u8; 64]) -> QSockAddr {
    // SAFETY: any 64-byte pattern is a valid QSockAddr
    unsafe { core::mem::transmute(b) }
}

fn addr_bytes(a: &QSockAddr) -> [u8; 64] {
    // SAFETY: QSockAddr is repr(C), 64 bytes, plain data
    unsafe { core::mem::transmute_copy(a) }
}

impl NetSys for FuzzSys {
    fn read(&mut self, _s: SysSocket, buf: &mut [u8]) -> (i32, QSockAddr) {
        match self.rx.pop_front() {
            None => (0, qs_addr([0; 64])),
            Some(RxEvent::Error) => (-1, qs_addr([0; 64])),
            Some(RxEvent::Packet(p, a)) => {
                let n = p.len().min(buf.len());
                buf[..n].copy_from_slice(&p[..n]);
                (n as i32, qs_addr(a))
            }
        }
    }
    fn write(&mut self, _s: SysSocket, _buf: &[u8], _addr: &QSockAddr) -> i32 {
        self.writes += 1;
        1
    }
    fn addr_compare(&mut self, a: &QSockAddr, b: &QSockAddr) -> i32 {
        if addr_bytes(a) == addr_bytes(b) {
            0
        } else {
            -1
        }
    }
    fn addr_to_string(&mut self, _addr: &QSockAddr) -> String {
        "fuzz".into()
    }
    fn print(&mut self, _m: &str) {}
    fn dprint(&mut self, _m: &str) {}
}

const ADDR_PEER: [u8; 64] = {
    let mut a = [0u8; 64];
    a[0] = 2;
    a[7] = 1;
    a
};

fuzz_target!(|data: &[u8]| {
    // SAFETY: an all-zero QSocket is valid (pointers null, plain data zero)
    let mut sock = unsafe { Box::<QSocket>::new_zeroed().assume_init() };
    sock.addr = qs_addr(ADDR_PEER);
    sock.can_send = true;
    // max_datagram is the one input that can drive send_fragment's
    // pkt[NET_HEADERSIZE..packet_len] slice and the ACK paths'
    // copy_within window out of range, and the engine holds it in range
    // from another translation unit (sv_main.c clamps limit_unreliable).
    // Derive it from the fuzz input over the whole legal range so that
    // invariant is pinned here rather than assumed.
    let mds = data.first().copied().unwrap_or(0) as usize;
    let max_datagram = 1 + (mds * 251) % quake_types::net::MAX_DATAGRAM;
    sock.max_datagram = max_datagram as i32;
    sock.pending_max_datagram = max_datagram as i32;

    let mut sys = FuzzSys::default();
    let mut nm = vec![0u8; NM_MAXSIZE];
    let mut cursize = 0i32;
    let mut counters = DgrmCounters::default();
    let mut mr = 0i32;
    let mut umr = 0i32;
    let mut scratch = vec![0u8; PACKET_BUFFER_SIZE];
    let mut net_time = 1.0f64;

    let mut it = data.iter().copied();
    let take = |n: usize, it: &mut dyn Iterator<Item = u8>| -> Vec<u8> { it.take(n).collect() };

    for _ in 0..64 {
        let Some(op) = it.next() else { break };
        macro_rules! globals {
            () => {
                DgrmGlobals {
                    net_time,
                    counters: &mut counters,
                    messages_received: &mut mr,
                    unreliable_messages_received: &mut umr,
                    net_message: &mut nm,
                    net_message_cursize: &mut cursize,
                    net_message_maxsize: NM_MAXSIZE as i32,
                }
            };
        }
        match op % 9 {
            0 => {
                // queue a raw packet: 2 length bytes then payload (header
                // bytes come straight from the fuzzer, so wire length /
                // flags / sequence are arbitrary)
                let l = take(2, &mut it);
                if l.len() < 2 {
                    break;
                }
                let n = (u16::from_le_bytes([l[0], l[1]]) as usize) % 2048;
                let pkt = take(n, &mut it);
                let addr = if n % 7 == 0 { [9u8; 64] } else { ADDR_PEER };
                sys.rx.push_back(RxEvent::Packet(pkt, addr));
            }
            1 => sys.rx.push_back(RxEvent::Error),
            2 => {
                let mut g = globals!();
                let r = dgrm::get_message(&mut sys, &mut sock, &mut g, &mut scratch);
                assert!(
                    r == -1
                        || r == 0
                        || r == 1
                        || r == 2
                        || r == dgrm::GET_MESSAGE_NET_MESSAGE_OVERFLOW
                );
            }
            3 => {
                let l = take(2, &mut it);
                if l.len() < 2 {
                    break;
                }
                let n = (u16::from_le_bytes([l[0], l[1]]) as usize) % 2048;
                let pkt = take(n, &mut it);
                scratch[..pkt.len()].copy_from_slice(&pkt);
                let mut g = globals!();
                dgrm::process_packet(&mut sys, &mut sock, &mut g, &mut scratch, pkt.len() as u32);
            }
            4 => {
                if sock.can_send {
                    let l = take(2, &mut it);
                    if l.len() < 2 {
                        break;
                    }
                    let n = (u16::from_le_bytes([l[0], l[1]]) as usize) % 4000;
                    let payload = take(n, &mut it);
                    let mut g = globals!();
                    let _ = dgrm::send_message(&mut sys, &mut sock, &mut g, &mut scratch, &payload);
                }
            }
            5 => {
                let n = (it.next().unwrap_or(0) as usize) * 4;
                let payload = take(n, &mut it);
                let mut g = globals!();
                let _ = dgrm::send_unreliable_message(
                    &mut sys,
                    &mut sock,
                    &mut g,
                    &mut scratch,
                    &payload,
                );
            }
            6 => {
                let mut g = globals!();
                dgrm::can_send_message(&mut sys, &mut sock, &mut g, &mut scratch);
            }
            7 => {
                if sock.send_message_length > 0 {
                    let mut g = globals!();
                    dgrm::resend_message(&mut sys, &mut sock, &mut g, &mut scratch);
                }
            }
            _ => {
                net_time += (it.next().unwrap_or(1) as f64) / 8.0;
            }
        }

        // hostile input must never corrupt the bookkeeping
        assert!(cursize >= 0 && cursize as usize <= NM_MAXSIZE);
        assert!(sock.send_message_length >= 0);
        assert!(sock.send_message_length as usize <= sock.send_message.len());
        assert!(sock.receive_message_length >= 0);
        assert!(sock.receive_message_length as usize <= sock.receive_message.len());
    }
});
