//! Differential test: the Rust loopback driver (quake-capi net_loop shims +
//! quake_net::loopback frame format) vs the C original in `Quake/net_loop.c`
//! (compiled as `c_ref_*`). Phase 5 M5.
//!
//! Both sides run the same scripted and randomized traffic over their own
//! qsocket pools and their own `net_message`; after every operation the
//! observable state -- return codes, popped message bytes, queue lengths,
//! canSend/sequence fields -- must agree exactly.

use core::ffi::{c_char, c_int};
use std::sync::{Mutex, MutexGuard};

use quake_ctest::net_stubs;
use quake_rs::net_loop as r;
use quake_types::net::{QSocket, SizeBuf};

extern "C" {
    fn c_ref_Loop_Connect(host: *const c_char) -> *mut QSocket;
    fn c_ref_Loop_CheckNewConnections() -> *mut QSocket;
    fn c_ref_Loop_GetMessage(sock: *mut QSocket) -> c_int;
    fn c_ref_Loop_GetAnyMessage() -> *mut QSocket;
    fn c_ref_Loop_SendMessage(sock: *mut QSocket, data: *mut SizeBuf) -> c_int;
    fn c_ref_Loop_SendUnreliableMessage(sock: *mut QSocket, data: *mut SizeBuf) -> c_int;
    fn c_ref_Loop_CanSendMessage(sock: *mut QSocket) -> bool;
    fn c_ref_Loop_CanSendUnreliableMessage(sock: *mut QSocket) -> bool;
    fn c_ref_Loop_Close(sock: *mut QSocket);
    fn ctest_qsocket_reset_c();

    static mut c_ref_net_message: SizeBuf;
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

/// One side's world: a connected loopback client/server pair plus the
/// side-owned net_message backing store.
struct World {
    client: *mut QSocket,
    server: *mut QSocket,
    _nm_store: Box<[u8]>,
}

unsafe fn setup_c() -> World {
    // SAFETY: serialized by TEST_LOCK; statics are test-owned
    unsafe {
        ctest_qsocket_reset_c();
        let mut store = vec![0u8; 65536].into_boxed_slice();
        let cn = &raw mut c_ref_net_message;
        (*cn).data = store.as_mut_ptr();
        (*cn).maxsize = store.len() as c_int;
        (*cn).cursize = 0;
        let client = c_ref_Loop_Connect(c"local".as_ptr());
        assert!(!client.is_null());
        let server = c_ref_Loop_CheckNewConnections();
        assert!(!server.is_null());
        World {
            client,
            server,
            _nm_store: store,
        }
    }
}

unsafe fn setup_r() -> World {
    // SAFETY: serialized by TEST_LOCK; statics are test-owned
    unsafe {
        net_stubs::qsocket_reset_rust();
        let mut store = vec![0u8; 65536].into_boxed_slice();
        let nm = &raw mut net_stubs::net_message;
        (*nm).data = store.as_mut_ptr();
        (*nm).maxsize = store.len() as c_int;
        (*nm).cursize = 0;
        let client = r::rust_loop_Connect(c"local".as_ptr());
        assert!(!client.is_null());
        let server = r::rust_loop_CheckNewConnections();
        assert!(!server.is_null());
        World {
            client,
            server,
            _nm_store: store,
        }
    }
}

fn make_sizebuf(payload: &mut Vec<u8>) -> SizeBuf {
    SizeBuf {
        allowoverflow: false,
        overflowed: false,
        data: payload.as_mut_ptr(),
        maxsize: payload.len() as c_int,
        cursize: payload.len() as c_int,
    }
}

/// compares one qsocket's loopback-relevant state across sides
unsafe fn assert_socket_state(cs: *mut QSocket, rs: *mut QSocket, ctx: &str) {
    // SAFETY: both are live pool slots owned by the running test
    unsafe {
        assert_eq!(
            (*cs).receive_message_length,
            (*rs).receive_message_length,
            "{ctx}: receiveMessageLength"
        );
        assert_eq!((*cs).can_send, (*rs).can_send, "{ctx}: canSend");
        assert_eq!(
            (*cs).unreliable_send_sequence,
            (*rs).unreliable_send_sequence,
            "{ctx}: unreliableSendSequence"
        );
        assert_eq!(
            (*cs).unreliable_receive_sequence,
            (*rs).unreliable_receive_sequence,
            "{ctx}: unreliableReceiveSequence"
        );
        let cq = core::slice::from_raw_parts(
            (&raw const (*cs).receive_message).cast::<u8>(),
            (*cs).receive_message_length as usize,
        );
        let rq = core::slice::from_raw_parts(
            (&raw const (*rs).receive_message).cast::<u8>(),
            (*rs).receive_message_length as usize,
        );
        assert_eq!(cq, rq, "{ctx}: queued bytes");
        assert_eq!(
            (*cs).driverdata.is_null(),
            (*rs).driverdata.is_null(),
            "{ctx}: driverdata"
        );
        assert_eq!(
            (*cs).proquake_angle_hack,
            (*rs).proquake_angle_hack,
            "{ctx}: proquake_angle_hack"
        );
    }
}

unsafe fn assert_net_message_equal(ctx: &str) {
    // SAFETY: both net_messages point at test-owned stores
    unsafe {
        let rn = &raw const net_stubs::net_message;
        let cn = &raw const c_ref_net_message;
        assert_eq!((*cn).cursize, (*rn).cursize, "{ctx}: cursize");
        let n = (*cn).cursize as usize;
        let cb = core::slice::from_raw_parts((*cn).data, n);
        let rb = core::slice::from_raw_parts((*rn).data, n);
        assert_eq!(cb, rb, "{ctx}: net_message bytes");
    }
}

#[test]
fn connect_and_addresses_match_c() {
    let _g = lock();
    // SAFETY: serialized; pools reset by the setups
    unsafe {
        let c = setup_c();
        let r_ = setup_r();
        for (cs, rs, which) in [
            (c.client, r_.client, "client"),
            (c.server, r_.server, "server"),
        ] {
            let ca = core::ffi::CStr::from_ptr((*cs).trueaddress.as_ptr());
            let ra = core::ffi::CStr::from_ptr((*rs).trueaddress.as_ptr());
            assert_eq!(ca, ra, "{which} trueaddress");
            let cm = core::ffi::CStr::from_ptr((*cs).maskedaddress.as_ptr());
            let rm = core::ffi::CStr::from_ptr((*rs).maskedaddress.as_ptr());
            assert_eq!(cm, rm, "{which} maskedaddress");
            assert_socket_state(cs, rs, which);
        }
        // a second connect while already connected reuses the pair
        let c2 = c_ref_Loop_Connect(c"local".as_ptr());
        let r2 = r::rust_loop_Connect(c"local".as_ptr());
        assert_eq!(c2 == c.client, r2 == r_.client, "reconnect identity");
        // non-"local" host is refused on both sides
        assert!(c_ref_Loop_Connect(c"127.0.0.1".as_ptr()).is_null());
        assert!(r::rust_loop_Connect(c"127.0.0.1".as_ptr()).is_null());
        // leave the driver statics clean for the next scenario
        c_ref_Loop_Close(c.client);
        c_ref_Loop_Close(c.server);
        r::rust_loop_Close(r_.client);
        r::rust_loop_Close(r_.server);
    }
}

#[test]
fn traffic_matches_c() {
    let _g = lock();
    // SAFETY: serialized; every pointer handed between sides is pool-owned
    unsafe {
        let c = setup_c();
        let r_ = setup_r();
        let mut rng = Rng(0xF00DFACECAFEBEEF);

        for step in 0..4000 {
            let op = rng.next() % 6;
            let from_client = rng.next().is_multiple_of(2);
            let (c_src, r_src) = if from_client {
                (c.client, r_.client)
            } else {
                (c.server, r_.server)
            };
            let (c_dst, r_dst) = if from_client {
                (c.server, r_.server)
            } else {
                (c.client, r_.client)
            };
            let ctx = format!("step {step} op {op} from_client {from_client}");
            match op {
                0 => {
                    // reliable send (respect canSend like the engine does,
                    // keeping both queues clear of the Sys_Error bound)
                    if c_ref_Loop_CanSendMessage(c_src) {
                        assert!(r::rust_loop_CanSendMessage(r_src), "{ctx}: canSend gate");
                        let n = (rng.next() % 900) as usize;
                        let mut payload: Vec<u8> = (0..n).map(|_| rng.next() as u8).collect();
                        let mut csb = make_sizebuf(&mut payload);
                        let mut rsb = make_sizebuf(&mut payload);
                        let cr = c_ref_Loop_SendMessage(c_src, &mut csb);
                        let rr = r::rust_loop_SendMessage(r_src, &mut rsb);
                        assert_eq!(cr, rr, "{ctx}: SendMessage ret");
                    }
                }
                1 => {
                    let n = (rng.next() % 1200) as usize;
                    let mut payload: Vec<u8> = (0..n).map(|_| rng.next() as u8).collect();
                    let mut csb = make_sizebuf(&mut payload);
                    let mut rsb = make_sizebuf(&mut payload);
                    let cr = c_ref_Loop_SendUnreliableMessage(c_src, &mut csb);
                    let rr = r::rust_loop_SendUnreliableMessage(r_src, &mut rsb);
                    assert_eq!(cr, rr, "{ctx}: SendUnreliable ret");
                }
                2 | 3 => {
                    let cr = c_ref_Loop_GetMessage(c_dst);
                    let rr = r::rust_loop_GetMessage(r_dst);
                    assert_eq!(cr, rr, "{ctx}: GetMessage ret");
                    if cr > 0 {
                        assert_net_message_equal(&ctx);
                    }
                }
                4 => {
                    let cr = c_ref_Loop_GetAnyMessage();
                    let rr = r::rust_loop_GetAnyMessage();
                    assert_eq!(cr.is_null(), rr.is_null(), "{ctx}: GetAnyMessage");
                    if !cr.is_null() {
                        assert_net_message_equal(&ctx);
                    }
                }
                _ => {
                    assert_eq!(
                        c_ref_Loop_CanSendMessage(c_src),
                        r::rust_loop_CanSendMessage(r_src),
                        "{ctx}: CanSendMessage"
                    );
                    assert_eq!(
                        c_ref_Loop_CanSendUnreliableMessage(c_src),
                        r::rust_loop_CanSendUnreliableMessage(r_src),
                        "{ctx}: CanSendUnreliableMessage"
                    );
                }
            }
            assert_socket_state(c.client, r_.client, &format!("{ctx}: client"));
            assert_socket_state(c.server, r_.server, &format!("{ctx}: server"));
        }
        // leave the driver statics clean for the next scenario
        c_ref_Loop_Close(c.client);
        c_ref_Loop_Close(c.server);
        r::rust_loop_Close(r_.client);
        r::rust_loop_Close(r_.server);
    }
}

#[test]
fn close_semantics_match_c() {
    let _g = lock();
    // SAFETY: serialized; pool slots stay valid after Close (pool-owned)
    unsafe {
        let c = setup_c();
        let r_ = setup_r();
        // close the client: the server's driverdata must drop to NULL and
        // the peer keeps functioning like the C original
        c_ref_Loop_Close(c.client);
        r::rust_loop_Close(r_.client);
        assert_socket_state(c.server, r_.server, "after client close");
        let mut payload = b"post-close".to_vec();
        let mut csb = make_sizebuf(&mut payload);
        let mut rsb = make_sizebuf(&mut payload);
        assert_eq!(
            c_ref_Loop_SendMessage(c.server, &mut csb),
            r::rust_loop_SendMessage(r_.server, &mut rsb),
            "send after peer close"
        );
        assert_eq!(
            c_ref_Loop_CanSendMessage(c.server),
            r::rust_loop_CanSendMessage(r_.server),
            "canSend after peer close"
        );
        c_ref_Loop_Close(c.server);
        r::rust_loop_Close(r_.server);
    }
}
