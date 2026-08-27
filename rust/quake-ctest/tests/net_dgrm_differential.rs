//! Differential test: the Rust datagram reliable layer (`quake_net::dgrm`)
//! vs the C original in `Quake/net_dgrm_rel.c` (compiled as `c_ref_*`).
//! Phase 5 M6.
//!
//! Both sides run identical op scripts over a deterministic mock landriver
//! (the C side through a `net_landrivers[0]` vtable of trampolines, the
//! Rust side through the `NetSys` trait, both over the same event queues).
//! After every op the observable world must agree exactly: return codes,
//! every qsocket sequencing field, the send/receive buffers, net_message,
//! the shared packet scratch (its stale bytes are load-bearing), the six
//! stat counters, the emitted wire packets, and the console diagnostics.

use core::ffi::{c_char, c_int, c_uint, c_void, CStr};
use std::collections::VecDeque;
use std::ptr;
use std::sync::{Mutex, MutexGuard};

use quake_ctest as _; // links the c_ref archive + stub globals
use quake_net::dgrm::{self, DgrmCounters, DgrmGlobals, NetSys, PACKET_BUFFER_SIZE};
use quake_types::net::{NetLanDriver, QSockAddr, QSocket, SizeBuf, SysSocket};

#[repr(C)]
struct PacketScratchC([u8; PACKET_BUFFER_SIZE]);

extern "C" {
    fn c_ref_Datagram_SendMessage(sock: *mut QSocket, data: *mut SizeBuf) -> c_int;
    fn c_ref_SendMessageNext(sock: *mut QSocket) -> c_int;
    fn c_ref_ReSendMessage(sock: *mut QSocket) -> c_int;
    fn c_ref_Datagram_CanSendMessage(sock: *mut QSocket) -> bool;
    fn c_ref_Datagram_CanSendUnreliableMessage(sock: *mut QSocket) -> bool;
    fn c_ref_Datagram_SendUnreliableMessage(sock: *mut QSocket, data: *mut SizeBuf) -> c_int;
    fn c_ref_Datagram_ProcessPacket(length: c_uint, sock: *mut QSocket) -> bool;
    fn c_ref_Datagram_GetMessage(sock: *mut QSocket) -> c_int;
    fn ctest_dgrm_reset_c();
    fn ctest_clear_con_log();
    fn ctest_con_log_len() -> c_int;
    fn ctest_con_log_get(i: c_int) -> *const c_char;
    fn ctest_try_host(f: unsafe extern "C" fn(*mut c_void), arg: *mut c_void) -> c_int;
    fn ctest_host_error_message() -> *const c_char;

    static mut c_ref_net_message: SizeBuf;
    static mut c_ref_net_landrivers: [NetLanDriver; 3];
    static mut c_ref_net_time: f64;
    static mut c_ref_messagesReceived: c_int;
    static mut c_ref_unreliableMessagesReceived: c_int;
    static mut c_ref_packetsSent: c_int;
    static mut c_ref_packetsReSent: c_int;
    static mut c_ref_packetsReceived: c_int;
    static mut c_ref_receivedDuplicateCount: c_int;
    static mut c_ref_shortPacketCount: c_int;
    static mut c_ref_droppedDatagrams: c_int;
    static mut c_ref_packetBuffer: PacketScratchC;
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
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

// ---------------------------------------------------------------------------
// mock landriver: one event-queue core per side

enum RxEvent {
    Packet(Vec<u8>, [u8; 64]),
    Error,
}

#[derive(Default)]
struct MockCore {
    rx: VecDeque<RxEvent>,
    writes: Vec<(Vec<u8>, [u8; 64])>,
}

fn addr_bytes(a: &QSockAddr) -> [u8; 64] {
    // SAFETY: QSockAddr is repr(C), 64 bytes, plain data
    unsafe { core::mem::transmute_copy(a) }
}

fn qs_addr(b: [u8; 64]) -> QSockAddr {
    // SAFETY: any 64-byte pattern is a valid QSockAddr
    unsafe { core::mem::transmute(b) }
}

/// shared AddrToString text so both sides print identical diagnostics
fn addr_str(b: &[u8; 64]) -> String {
    format!("mock:{:02x}{:02x}{:02x}{:02x}", b[0], b[1], b[2], b[3])
}

static mut C_CORE: *mut MockCore = ptr::null_mut();

unsafe extern "C" fn tramp_read(
    _sock: SysSocket,
    buf: *mut u8,
    len: c_int,
    addr: *mut QSockAddr,
) -> c_int {
    // SAFETY: serialized by TEST_LOCK; C_CORE set for the duration of a C op
    unsafe {
        let core = &mut *C_CORE;
        match core.rx.pop_front() {
            None => 0,
            Some(RxEvent::Error) => -1,
            Some(RxEvent::Packet(p, a)) => {
                let n = p.len().min(len as usize);
                ptr::copy_nonoverlapping(p.as_ptr(), buf, n);
                ptr::copy_nonoverlapping(a.as_ptr(), addr.cast::<u8>(), 64);
                n as c_int
            }
        }
    }
}

unsafe extern "C" fn tramp_write(
    _sock: SysSocket,
    buf: *mut u8,
    len: c_int,
    addr: *mut QSockAddr,
) -> c_int {
    // SAFETY: see tramp_read
    unsafe {
        let core = &mut *C_CORE;
        let bytes = core::slice::from_raw_parts(buf, len as usize).to_vec();
        core.writes.push((bytes, addr_bytes(&*addr)));
        len
    }
}

unsafe extern "C" fn tramp_addr_compare(a: *mut QSockAddr, b: *mut QSockAddr) -> c_int {
    // SAFETY: see tramp_read
    unsafe {
        if addr_bytes(&*a) == addr_bytes(&*b) {
            0
        } else {
            -1
        }
    }
}

static mut ADDR_STR_BUF: [u8; 96] = [0; 96];

unsafe extern "C" fn tramp_addr_to_string(addr: *mut QSockAddr, _masked: bool) -> *const c_char {
    // SAFETY: see tramp_read; the static return buffer mirrors the C drivers'
    unsafe {
        let s = addr_str(&addr_bytes(&*addr));
        let buf = &mut *core::ptr::addr_of_mut!(ADDR_STR_BUF);
        buf[..s.len()].copy_from_slice(s.as_bytes());
        buf[s.len()] = 0;
        buf.as_ptr().cast()
    }
}

struct RustSys<'a> {
    core: &'a mut MockCore,
    prints: &'a mut Vec<String>,
}

impl NetSys for RustSys<'_> {
    fn read(&mut self, _socket: SysSocket, buf: &mut [u8]) -> (i32, QSockAddr) {
        match self.core.rx.pop_front() {
            None => (0, qs_addr([0; 64])),
            Some(RxEvent::Error) => (-1, qs_addr([0; 64])),
            Some(RxEvent::Packet(p, a)) => {
                let n = p.len().min(buf.len());
                buf[..n].copy_from_slice(&p[..n]);
                (n as i32, qs_addr(a))
            }
        }
    }
    fn write(&mut self, _socket: SysSocket, buf: &[u8], addr: &QSockAddr) -> i32 {
        self.core.writes.push((buf.to_vec(), addr_bytes(addr)));
        buf.len() as i32
    }
    fn addr_compare(&mut self, a: &QSockAddr, b: &QSockAddr) -> i32 {
        if addr_bytes(a) == addr_bytes(b) {
            0
        } else {
            -1
        }
    }
    fn addr_to_string(&mut self, addr: &QSockAddr) -> String {
        addr_str(&addr_bytes(addr))
    }
    fn print(&mut self, msg: &str) {
        self.prints.push(format!("[con] {msg}"));
    }
    fn dprint(&mut self, msg: &str) {
        self.prints.push(format!("[dcon] {msg}"));
    }
}

// ---------------------------------------------------------------------------
// worlds

const ADDR_PEER: [u8; 64] = {
    let mut a = [0u8; 64];
    a[0] = 2; // AF-ish tag
    a[2] = 0x1b;
    a[3] = 0x2c;
    a[4] = 10;
    a[5] = 0;
    a[6] = 0;
    a[7] = 1;
    a
};
const ADDR_STRAY: [u8; 64] = {
    let mut a = [0u8; 64];
    a[0] = 2;
    a[2] = 0x1b;
    a[3] = 0x2d;
    a[4] = 10;
    a[5] = 0;
    a[6] = 0;
    a[7] = 9;
    a
};

const NM_MAXSIZE: usize = 64000; // net_main.c: SZ_Alloc (&net_message, NET_MAXMESSAGE)

fn new_sock() -> Box<QSocket> {
    // SAFETY: an all-zero QSocket is valid (pointers null, plain data zero)
    let mut s = unsafe { Box::<QSocket>::new_zeroed().assume_init() };
    s.socket = 7 as SysSocket;
    s.landriver = 0;
    s.addr = qs_addr(ADDR_PEER);
    s.can_send = true;
    s.max_datagram = 1442;
    s.pending_max_datagram = 1442;
    s
}

struct CWorld {
    sock: Box<QSocket>,
    nm: Box<[u8]>,
    core: MockCore,
    prints: Vec<String>,
}

struct RWorld {
    sock: Box<QSocket>,
    nm: Box<[u8]>,
    nm_cursize: i32,
    counters: DgrmCounters,
    messages_received: i32,
    unreliable_messages_received: i32,
    scratch: Vec<u8>,
    core: MockCore,
    prints: Vec<String>,
    net_time: f64,
}

fn setup() -> (CWorld, RWorld) {
    // SAFETY: serialized by TEST_LOCK; the c_ref statics are test-owned
    unsafe {
        ctest_dgrm_reset_c();
        ctest_clear_con_log();
        let mut cw = CWorld {
            sock: new_sock(),
            nm: vec![0u8; NM_MAXSIZE].into_boxed_slice(),
            core: MockCore::default(),
            prints: Vec::new(),
        };
        let cn = &raw mut c_ref_net_message;
        (*cn).allowoverflow = false;
        (*cn).overflowed = false;
        (*cn).data = cw.nm.as_mut_ptr();
        (*cn).maxsize = NM_MAXSIZE as c_int;
        (*cn).cursize = 0;
        let ld = &raw mut c_ref_net_landrivers;
        (*ld)[0].read = Some(tramp_read);
        (*ld)[0].write = Some(tramp_write);
        (*ld)[0].addr_compare = Some(tramp_addr_compare);
        (*ld)[0].addr_to_string = Some(tramp_addr_to_string);
        let rw = RWorld {
            sock: new_sock(),
            nm: vec![0u8; NM_MAXSIZE].into_boxed_slice(),
            nm_cursize: 0,
            counters: DgrmCounters::default(),
            messages_received: 0,
            unreliable_messages_received: 0,
            scratch: vec![0u8; PACKET_BUFFER_SIZE],
            core: MockCore::default(),
            prints: Vec::new(),
            net_time: 0.0,
        };
        (cw, rw)
    }
}

/// runs a C-side call with the mock core installed and the con log drained
fn c_call<T>(cw: &mut CWorld, f: impl FnOnce(&mut CWorld) -> T) -> T {
    // SAFETY: serialized by TEST_LOCK
    unsafe {
        C_CORE = &mut cw.core;
        ctest_clear_con_log();
        let r = f(cw);
        C_CORE = ptr::null_mut();
        let n = ctest_con_log_len();
        for i in 0..n {
            cw.prints.push(
                CStr::from_ptr(ctest_con_log_get(i))
                    .to_string_lossy()
                    .into_owned(),
            );
        }
        r
    }
}

fn r_globals<'a>(
    rw: &'a mut RWorld,
) -> (
    DgrmGlobals<'a>,
    &'a mut Box<QSocket>,
    &'a mut Vec<u8>,
    RustSys<'a>,
) {
    let g = DgrmGlobals {
        net_time: rw.net_time,
        counters: &mut rw.counters,
        messages_received: &mut rw.messages_received,
        unreliable_messages_received: &mut rw.unreliable_messages_received,
        net_message: &mut rw.nm,
        net_message_cursize: &mut rw.nm_cursize,
        net_message_maxsize: NM_MAXSIZE as i32,
    };
    let sys = RustSys {
        core: &mut rw.core,
        prints: &mut rw.prints,
    };
    (g, &mut rw.sock, &mut rw.scratch, sys)
}

// ---------------------------------------------------------------------------
// ops

#[derive(Clone, Debug)]
enum Op {
    Time(f64),
    Rx(Vec<u8>, [u8; 64]),
    RxErr,
    GetMessage,
    ProcessPacket(Vec<u8>, u32),
    SendReliable(Vec<u8>),
    SendUnreliable(Vec<u8>),
    CanSend,
    CanSendUnreliable,
    Resend,
    SendNext,
}

/// wire packet: raw header word (flags | wire-length), sequence, payload
fn packet(word: u32, seq: u32, payload: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(8 + payload.len());
    v.extend_from_slice(&word.to_be_bytes());
    v.extend_from_slice(&seq.to_be_bytes());
    v.extend_from_slice(payload);
    v
}

/// a well-formed packet whose wire length matches its real size
fn packet_wf(flags: u32, seq: u32, payload: &[u8]) -> Vec<u8> {
    packet(flags | (8 + payload.len() as u32), seq, payload)
}

const RET_HOST_ERROR: i32 = -1000;

struct GetMsgArgs {
    sock: *mut QSocket,
    ret: i32,
}

unsafe extern "C" fn call_c_getmsg(arg: *mut c_void) {
    // SAFETY: arg is the GetMsgArgs the caller passed
    unsafe {
        let a = &mut *arg.cast::<GetMsgArgs>();
        a.ret = c_ref_Datagram_GetMessage(a.sock);
    }
}

fn run_op(step: usize, op: &Op, cw: &mut CWorld, rw: &mut RWorld) {
    match op {
        Op::Time(t) => {
            // SAFETY: serialized by TEST_LOCK
            unsafe {
                c_ref_net_time = *t;
            }
            rw.net_time = *t;
        }
        Op::Rx(bytes, addr) => {
            cw.core.rx.push_back(RxEvent::Packet(bytes.clone(), *addr));
            rw.core.rx.push_back(RxEvent::Packet(bytes.clone(), *addr));
        }
        Op::RxErr => {
            cw.core.rx.push_back(RxEvent::Error);
            rw.core.rx.push_back(RxEvent::Error);
        }
        Op::GetMessage => {
            let cret = c_call(cw, |cw| {
                let mut args = GetMsgArgs {
                    sock: &mut *cw.sock,
                    ret: RET_HOST_ERROR,
                };
                // SAFETY: the Host_Error trap catches the hostile-length
                // SZ_GetSpace path (the longjmp crosses only trivial frames)
                unsafe {
                    if ctest_try_host(call_c_getmsg, (&mut args as *mut GetMsgArgs).cast()) != 0 {
                        let msg = CStr::from_ptr(ctest_host_error_message());
                        assert_eq!(
                            msg.to_str().unwrap(),
                            "SZ_GetSpace: overflow without allowoverflow set",
                            "step {step}: unexpected C Host_Error"
                        );
                        RET_HOST_ERROR
                    } else {
                        args.ret
                    }
                }
            });
            let rret = {
                let (mut g, sock, scratch, mut sys) = r_globals(rw);
                dgrm::get_message(&mut sys, sock, &mut g, scratch)
            };
            let rret = if rret == dgrm::GET_MESSAGE_NET_MESSAGE_OVERFLOW {
                RET_HOST_ERROR
            } else {
                rret
            };
            assert_eq!(cret, rret, "step {step}: GetMessage return");
        }
        Op::ProcessPacket(bytes, os_len) => {
            let cret = c_call(cw, |cw| {
                // SAFETY: serialized by TEST_LOCK; GetAnyMessage fills the C
                // packetBuffer scratch before calling ProcessPacket -- the
                // test does the same on both sides
                unsafe {
                    let pb = &mut (*core::ptr::addr_of_mut!(c_ref_packetBuffer)).0;
                    pb[..bytes.len()].copy_from_slice(bytes);
                    c_ref_Datagram_ProcessPacket(*os_len, &mut *cw.sock)
                }
            });
            let rret = {
                let (mut g, sock, scratch, mut sys) = r_globals(rw);
                scratch[..bytes.len()].copy_from_slice(bytes);
                dgrm::process_packet(&mut sys, sock, &mut g, scratch, *os_len)
            };
            assert_eq!(cret, rret, "step {step}: ProcessPacket return");
        }
        Op::SendReliable(payload) => {
            let mut cp = payload.clone();
            let cret = c_call(cw, |cw| {
                let mut sb = SizeBuf {
                    allowoverflow: false,
                    overflowed: false,
                    data: cp.as_mut_ptr(),
                    maxsize: cp.len() as c_int,
                    cursize: cp.len() as c_int,
                };
                // SAFETY: serialized by TEST_LOCK
                unsafe { c_ref_Datagram_SendMessage(&mut *cw.sock, &mut sb) }
            });
            let rret = {
                let (mut g, sock, scratch, mut sys) = r_globals(rw);
                dgrm::send_message(&mut sys, sock, &mut g, scratch, payload)
                    .expect("in-domain send_message")
            };
            assert_eq!(cret, rret, "step {step}: SendMessage return");
        }
        Op::SendUnreliable(payload) => {
            let mut cp = payload.clone();
            let cret = c_call(cw, |cw| {
                let mut sb = SizeBuf {
                    allowoverflow: false,
                    overflowed: false,
                    data: cp.as_mut_ptr(),
                    maxsize: cp.len() as c_int,
                    cursize: cp.len() as c_int,
                };
                // SAFETY: serialized by TEST_LOCK
                unsafe { c_ref_Datagram_SendUnreliableMessage(&mut *cw.sock, &mut sb) }
            });
            let rret = {
                let (mut g, sock, scratch, mut sys) = r_globals(rw);
                dgrm::send_unreliable_message(&mut sys, sock, &mut g, scratch, payload)
                    .expect("in-domain send_unreliable")
            };
            assert_eq!(cret, rret, "step {step}: SendUnreliable return");
        }
        Op::CanSend => {
            // SAFETY: serialized by TEST_LOCK
            let cret = c_call(cw, |cw| unsafe {
                c_ref_Datagram_CanSendMessage(&mut *cw.sock)
            });
            let rret = {
                let (mut g, sock, scratch, mut sys) = r_globals(rw);
                dgrm::can_send_message(&mut sys, sock, &mut g, scratch)
            };
            assert_eq!(cret, rret, "step {step}: CanSend return");
        }
        Op::CanSendUnreliable => {
            // SAFETY: serialized by TEST_LOCK
            let cret = c_call(cw, |cw| unsafe {
                c_ref_Datagram_CanSendUnreliableMessage(&mut *cw.sock)
            });
            assert_eq!(cret, dgrm::can_send_unreliable_message(), "step {step}");
        }
        Op::Resend => {
            // SAFETY: serialized by TEST_LOCK
            let cret = c_call(cw, |cw| unsafe { c_ref_ReSendMessage(&mut *cw.sock) });
            let rret = {
                let (mut g, sock, scratch, mut sys) = r_globals(rw);
                dgrm::resend_message(&mut sys, sock, &mut g, scratch)
            };
            assert_eq!(cret, rret, "step {step}: ReSend return");
        }
        Op::SendNext => {
            // SAFETY: serialized by TEST_LOCK
            let cret = c_call(cw, |cw| unsafe { c_ref_SendMessageNext(&mut *cw.sock) });
            let rret = {
                let (mut g, sock, scratch, mut sys) = r_globals(rw);
                dgrm::send_message_next(&mut sys, sock, &mut g, scratch)
            };
            assert_eq!(cret, rret, "step {step}: SendNext return");
        }
    }
    compare(step, cw, rw);
}

fn compare(step: usize, cw: &mut CWorld, rw: &mut RWorld) {
    let cs = &cw.sock;
    let rs = &rw.sock;
    assert_eq!(cs.can_send, rs.can_send, "step {step}: canSend");
    assert_eq!(cs.send_next, rs.send_next, "step {step}: sendNext");
    assert_eq!(cs.ack_sequence, rs.ack_sequence, "step {step}: ackSequence");
    assert_eq!(
        cs.send_sequence, rs.send_sequence,
        "step {step}: sendSequence"
    );
    assert_eq!(
        cs.unreliable_send_sequence, rs.unreliable_send_sequence,
        "step {step}: unreliableSendSequence"
    );
    assert_eq!(
        cs.receive_sequence, rs.receive_sequence,
        "step {step}: receiveSequence"
    );
    assert_eq!(
        cs.unreliable_receive_sequence, rs.unreliable_receive_sequence,
        "step {step}: unreliableReceiveSequence"
    );
    assert_eq!(
        cs.send_message_length, rs.send_message_length,
        "step {step}: sendMessageLength"
    );
    assert_eq!(
        cs.receive_message_length, rs.receive_message_length,
        "step {step}: receiveMessageLength"
    );
    assert_eq!(
        cs.last_send_time, rs.last_send_time,
        "step {step}: lastSendTime"
    );
    let sml = cs.send_message_length.max(0) as usize;
    assert_eq!(
        &cs.send_message[..sml],
        &rs.send_message[..sml],
        "step {step}: sendMessage bytes"
    );
    let rml = cs.receive_message_length.max(0) as usize;
    assert_eq!(
        &cs.receive_message[..rml],
        &rs.receive_message[..rml],
        "step {step}: receiveMessage bytes"
    );

    // SAFETY: serialized by TEST_LOCK
    unsafe {
        let cn = &raw const c_ref_net_message;
        assert_eq!(
            (*cn).cursize,
            rw.nm_cursize,
            "step {step}: net_message.cursize"
        );
        let n = (*cn).cursize.max(0) as usize;
        assert_eq!(&cw.nm[..n], &rw.nm[..n], "step {step}: net_message bytes");

        let pb = &(*core::ptr::addr_of!(c_ref_packetBuffer)).0;
        assert_eq!(
            pb.as_slice(),
            &rw.scratch[..PACKET_BUFFER_SIZE],
            "step {step}: packet scratch"
        );

        let cc = DgrmCounters {
            packets_sent: core::ptr::addr_of!(c_ref_packetsSent).read(),
            packets_resent: core::ptr::addr_of!(c_ref_packetsReSent).read(),
            packets_received: core::ptr::addr_of!(c_ref_packetsReceived).read(),
            received_duplicate_count: core::ptr::addr_of!(c_ref_receivedDuplicateCount).read(),
            short_packet_count: core::ptr::addr_of!(c_ref_shortPacketCount).read(),
            dropped_datagrams: core::ptr::addr_of!(c_ref_droppedDatagrams).read(),
        };
        assert_eq!(cc, rw.counters, "step {step}: counters");
        assert_eq!(
            core::ptr::addr_of!(c_ref_messagesReceived).read(),
            rw.messages_received,
            "step {step}: messagesReceived"
        );
        assert_eq!(
            core::ptr::addr_of!(c_ref_unreliableMessagesReceived).read(),
            rw.unreliable_messages_received,
            "step {step}: unreliableMessagesReceived"
        );
    }

    let cwrites: Vec<_> = cw.core.writes.drain(..).collect();
    let rwrites: Vec<_> = rw.core.writes.drain(..).collect();
    assert_eq!(cwrites, rwrites, "step {step}: emitted wire packets");

    let cprints: Vec<_> = cw.prints.drain(..).collect();
    let rprints: Vec<_> = rw.prints.drain(..).collect();
    // Scope: this pins the rel layer's own diagnostics -- text, count and
    // relative order. It canNOT see the interleaving against landriver
    // prints, because the mock NetSys never prints: in the engine the C
    // path interleaves them and the Rust path defers its own to the end of
    // the call (accepted divergence, documented in quake-capi::net_dgrm).
    assert_eq!(cprints, rprints, "step {step}: console diagnostics");
}

fn run_script(ops: &[Op]) {
    let (mut cw, mut rw) = setup();
    for (i, op) in ops.iter().enumerate() {
        run_op(i, op, &mut cw, &mut rw);
    }
}

// ---------------------------------------------------------------------------
// fixed scenarios

use quake_types::net::{NETFLAG_ACK, NETFLAG_CTL, NETFLAG_DATA, NETFLAG_EOM, NETFLAG_UNRELIABLE};

#[test]
fn reliable_send_ack_cycle() {
    let _l = lock();
    let payload = vec![0xAAu8; 600];
    run_script(&[
        Op::Time(1.0),
        Op::SendReliable(payload),
        Op::CanSend, // false, nothing pending
        // ACK seq 0 arrives
        Op::Rx(packet(NETFLAG_ACK | 8, 0, &[]), ADDR_PEER),
        Op::GetMessage,
        Op::CanSend, // true again
    ]);
}

#[test]
fn reliable_multi_fragment_send() {
    let _l = lock();
    let payload: Vec<u8> = (0..4000u32).map(|i| (i % 251) as u8).collect();
    run_script(&[
        Op::Time(1.0),
        Op::SendReliable(payload), // 1442 max_datagram -> 3 fragments
        Op::Rx(packet(NETFLAG_ACK | 8, 0, &[]), ADDR_PEER),
        Op::GetMessage, // consumes ack, sets sendNext, sends fragment 2 at loop exit
        Op::Rx(packet(NETFLAG_ACK | 8, 1, &[]), ADDR_PEER),
        Op::GetMessage,
        Op::Rx(packet(NETFLAG_ACK | 8, 2, &[]), ADDR_PEER),
        Op::GetMessage,
        Op::CanSend,
    ]);
}

#[test]
fn resend_after_timeout() {
    let _l = lock();
    run_script(&[
        Op::Time(5.0),
        Op::SendReliable(vec![1, 2, 3]),
        Op::GetMessage, // no timeout yet (lastSendTime == net_time)
        Op::Time(6.5),
        Op::GetMessage, // > 1.0 since send -> resend fires
        Op::Resend,     // direct resend too
    ]);
}

#[test]
fn rx_reliable_single_and_fragmented() {
    let _l = lock();
    let a: Vec<u8> = (0..100u8).collect();
    let b = vec![7u8; 50];
    run_script(&[
        // single EOM reliable
        Op::Rx(packet_wf(NETFLAG_DATA | NETFLAG_EOM, 0, &a), ADDR_PEER),
        Op::GetMessage,
        // fragmented: two frags then EOM
        Op::Rx(packet_wf(NETFLAG_DATA, 1, &b), ADDR_PEER),
        Op::Rx(packet_wf(NETFLAG_DATA, 2, &b), ADDR_PEER),
        Op::Rx(packet_wf(NETFLAG_DATA | NETFLAG_EOM, 3, &a), ADDR_PEER),
        Op::GetMessage,
        // duplicate DATA (stale sequence): acked but counted duplicate
        Op::Rx(packet_wf(NETFLAG_DATA | NETFLAG_EOM, 1, &b), ADDR_PEER),
        Op::GetMessage,
    ]);
}

#[test]
fn rx_unreliable_order_and_drops() {
    let _l = lock();
    let p = vec![9u8; 32];
    run_script(&[
        Op::Rx(packet_wf(NETFLAG_UNRELIABLE, 0, &p), ADDR_PEER),
        Op::GetMessage,
        // gap: seq 4 -> "Dropped 3 datagram(s)"
        Op::Rx(packet_wf(NETFLAG_UNRELIABLE, 4, &p), ADDR_PEER),
        Op::GetMessage,
        // stale
        Op::Rx(packet_wf(NETFLAG_UNRELIABLE, 2, &p), ADDR_PEER),
        Op::GetMessage,
    ]);
}

#[test]
fn rx_ack_edge_cases() {
    let _l = lock();
    run_script(&[
        Op::SendReliable(vec![1; 10]),
        // stale ack (wrong sequence)
        Op::Rx(packet(NETFLAG_ACK | 8, 5, &[]), ADDR_PEER),
        Op::GetMessage,
        // correct ack
        Op::Rx(packet(NETFLAG_ACK | 8, 0, &[]), ADDR_PEER),
        Op::GetMessage,
        // duplicate of it: "Stale ACK received" (sendSequence moved on? no --
        // sequence != sendSequence-1 is false, ackSequence mismatch path)
        Op::Rx(packet(NETFLAG_ACK | 8, 0, &[]), ADDR_PEER),
        Op::GetMessage,
    ]);
}

#[test]
fn rx_junk_ctl_short_stray_error() {
    let _l = lock();
    run_script(&[
        // CTL-flagged: skipped
        Op::Rx(packet(NETFLAG_CTL | 12, 0, &[1, 2, 3, 4]), ADDR_PEER),
        // unknown flags word
        Op::Rx(packet(0x0100_0000 | 12, 0, &[1, 2, 3, 4]), ADDR_PEER),
        // short packet (os length < 8)
        Op::Rx(vec![1, 2, 3], ADDR_PEER),
        // stray address
        Op::Rx(
            packet_wf(NETFLAG_DATA | NETFLAG_EOM, 0, &[5; 8]),
            ADDR_STRAY,
        ),
        Op::GetMessage,
        // read error
        Op::RxErr,
        Op::GetMessage,
    ]);
}

#[test]
fn rx_oversize_paths() {
    let _l = lock();
    run_script(&[
        // oversize unreliable via get_message: C Host_Errors in SZ_GetSpace
        Op::Rx(packet(NETFLAG_UNRELIABLE | 0xFFFF, 0, &[0; 64]), ADDR_PEER),
        Op::GetMessage,
    ]);
    run_script(&[
        // oversize unreliable via process_packet: pre-checked, prints
        Op::ProcessPacket(packet(NETFLAG_UNRELIABLE | 0xFFFF, 0, &[0; 64]), 72),
        // oversize reliable EOM via get_message: pre-checked, returns -1
        Op::Rx(
            packet(NETFLAG_DATA | NETFLAG_EOM | 0xFFFF, 0, &[0; 64]),
            ADDR_PEER,
        ),
        Op::GetMessage,
    ]);
    run_script(&[
        // wire length below the header (wraps huge): oversize reliable
        Op::Rx(
            packet(NETFLAG_DATA | NETFLAG_EOM | 3, 0, &[0; 8]),
            ADDR_PEER,
        ),
        Op::GetMessage,
    ]);
}

#[test]
fn rx_stale_scratch_bytes_are_reproduced() {
    let _l = lock();
    // a send fills the scratch with payload bytes; a later short packet
    // claiming a longer wire length makes C copy those stale bytes into
    // net_message -- the port must produce the identical stale content
    let marker: Vec<u8> = (0..1200u32).map(|i| (i * 7 % 253) as u8).collect();
    run_script(&[
        Op::SendUnreliable(marker),
        // 16 os bytes, wire claims 8+200
        Op::Rx(packet(NETFLAG_UNRELIABLE | 208, 0, &[0xEE; 8]), ADDR_PEER),
        Op::GetMessage,
        // same shape through the server path
        Op::ProcessPacket(packet(NETFLAG_UNRELIABLE | 208, 1, &[0xEE; 8]), 16),
    ]);
}

#[test]
fn process_packet_paths() {
    let _l = lock();
    let a: Vec<u8> = (0..300u16).map(|i| (i % 256) as u8).collect();
    run_script(&[
        // short
        Op::ProcessPacket(vec![1, 2, 3], 3),
        // ctl
        Op::ProcessPacket(packet(NETFLAG_CTL | 12, 0, &[1, 2, 3, 4]), 12),
        // unreliable in order
        Op::ProcessPacket(packet_wf(NETFLAG_UNRELIABLE, 0, &a), 308),
        // unreliable gap
        Op::ProcessPacket(packet_wf(NETFLAG_UNRELIABLE, 3, &a), 308),
        // unreliable stale
        Op::ProcessPacket(packet_wf(NETFLAG_UNRELIABLE, 1, &a), 308),
        // reliable fragment + EOM (ACKs go to sock->addr here)
        Op::ProcessPacket(packet_wf(NETFLAG_DATA, 0, &a), 308),
        Op::ProcessPacket(packet_wf(NETFLAG_DATA | NETFLAG_EOM, 1, &a), 308),
        // duplicate data
        Op::ProcessPacket(packet_wf(NETFLAG_DATA | NETFLAG_EOM, 0, &a), 308),
        // unknown flags
        Op::ProcessPacket(packet(0x0100_0000 | 12, 9, &[0; 4]), 12),
    ]);
}

#[test]
fn ack_reshuffles_send_window() {
    let _l = lock();
    // ack of a fragment shifts sendMessage down by max_datagram via the
    // server path too
    let payload: Vec<u8> = (0..3000u32).map(|i| (i % 250) as u8).collect();
    run_script(&[
        Op::SendReliable(payload),
        Op::ProcessPacket(packet(NETFLAG_ACK | 8, 0, &[]), 8),
        Op::SendNext,
        Op::ProcessPacket(packet(NETFLAG_ACK | 8, 1, &[]), 8),
        Op::ProcessPacket(packet(NETFLAG_ACK | 8, 1, &[]), 8), // duplicate
        Op::CanSend,
    ]);
}

// ---------------------------------------------------------------------------
// randomized sweep

#[test]
fn randomized_op_sweep() {
    let _l = lock();
    let mut rng = Rng(0x9E3779B97F4A7C15);
    for round in 0..60 {
        let mut ops = Vec::new();
        let mut t = 1.0f64;
        // keep the peer mostly well-formed so deep states are reached, with
        // junk sprinkled in
        let mut next_unrel = 0u32;
        let mut next_data = 0u32;
        for _ in 0..40 {
            match rng.below(12) {
                0 => {
                    t += (rng.below(30) as f64) / 10.0;
                    ops.push(Op::Time(t));
                }
                1 | 2 => {
                    let n = 1 + rng.below(3000) as usize;
                    let p: Vec<u8> = (0..n).map(|i| (i as u64 ^ rng.0) as u8).collect();
                    ops.push(Op::SendReliable(p));
                    ops.push(Op::CanSend);
                }
                3 => {
                    let n = 1 + rng.below(1300) as usize;
                    ops.push(Op::SendUnreliable(vec![rng.next() as u8; n]));
                }
                4 | 5 => {
                    let n = rng.below(900) as usize;
                    let p: Vec<u8> = (0..n).map(|_| rng.next() as u8).collect();
                    let seq = next_unrel + (rng.below(3) as u32).saturating_sub(1);
                    next_unrel = next_unrel.max(seq + 1);
                    ops.push(Op::Rx(packet_wf(NETFLAG_UNRELIABLE, seq, &p), ADDR_PEER));
                    ops.push(Op::GetMessage);
                }
                6 | 7 => {
                    let n = rng.below(900) as usize;
                    let p: Vec<u8> = (0..n).map(|_| rng.next() as u8).collect();
                    let eom = if rng.below(2) == 0 { NETFLAG_EOM } else { 0 };
                    let seq = next_data + (rng.below(2) as u32).saturating_sub(1);
                    if eom != 0 || seq == next_data {
                        next_data = next_data.max(seq + 1);
                    }
                    ops.push(Op::Rx(packet_wf(NETFLAG_DATA | eom, seq, &p), ADDR_PEER));
                    ops.push(Op::GetMessage);
                }
                8 => {
                    // acks (often stale)
                    let seq = rng.below(6) as u32;
                    ops.push(Op::Rx(packet(NETFLAG_ACK | 8, seq, &[]), ADDR_PEER));
                    ops.push(Op::GetMessage);
                }
                9 => {
                    // junk: ctl / unknown / short / stray
                    let junk = match rng.below(4) {
                        0 => packet(NETFLAG_CTL | 12, 0, &[1; 4]),
                        1 => packet(0x0100_0000 | 12, 0, &[1; 4]),
                        2 => vec![1, 2, 3],
                        _ => packet_wf(NETFLAG_DATA | NETFLAG_EOM, 0, &[1; 4]),
                    };
                    let addr = if rng.below(4) == 0 {
                        ADDR_STRAY
                    } else {
                        ADDR_PEER
                    };
                    ops.push(Op::Rx(junk, addr));
                    ops.push(Op::GetMessage);
                }
                10 => {
                    // server path traffic
                    let n = rng.below(600) as usize;
                    let p: Vec<u8> = (0..n).map(|_| rng.next() as u8).collect();
                    let flags = match rng.below(3) {
                        0 => NETFLAG_UNRELIABLE,
                        1 => NETFLAG_DATA,
                        _ => NETFLAG_DATA | NETFLAG_EOM,
                    };
                    let seq = rng.below(4) as u32;
                    let pk = packet_wf(flags, seq, &p);
                    let os = pk.len() as u32;
                    ops.push(Op::ProcessPacket(pk, os));
                }
                _ => {
                    ops.push(Op::CanSend);
                    ops.push(Op::CanSendUnreliable);
                }
            }
        }
        let _ = round;
        run_script(&ops);
    }
}
