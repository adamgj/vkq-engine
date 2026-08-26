//! CCREQ/CCREP control-packet fuzzer (Phase 5 M6, ADR-019 gate 5): replays
//! the exact read sequences the engine performs on hostile connectionless
//! packets -- `_Datagram_ServerControlPacket`'s request dispatch, the
//! `_Datagram_Connect` response parse, and `_Datagram_SearchForHosts`'s
//! CCREP_SERVER_INFO / getserversResponse parses -- through the Rust MSG
//! reader those paths run on under `-Duse_rust_net`. Asserts no panics,
//! cursor monotonicity within bounds, and the badread latch.

#![no_main]

use libfuzzer_sys::fuzz_target;
use quake_net::msg::MsgReader;

const CCREQ_CONNECT: i32 = 0x01;
const CCREQ_SERVER_INFO: i32 = 0x02;
const CCREQ_PLAYER_INFO: i32 = 0x03;
const CCREQ_RULE_INFO: i32 = 0x04;
const CCREQ_RCON: i32 = 0x05;
const CCREP_ACCEPT: i32 = 0x81;
const CCREP_REJECT: i32 = 0x82;
const CCREP_SERVER_INFO: i32 = 0x83;
const CCREP_PLAYER_INFO: i32 = 0x84;
const CCREP_RULE_INFO: i32 = 0x85;

fn check(r: &MsgReader, cursize: i32) {
    assert!(r.readcount >= 0);
    assert!(r.readcount <= cursize.max(0) + 16); // reads step past end only by one op's width
}

/// server side: _Datagram_ServerControlPacket after the header validation
fn server_dispatch(payload: &[u8]) {
    let cursize = payload.len() as i32;
    let mut r = MsgReader::begin(payload, cursize);
    r.read_long();
    let command = r.read_byte();
    match command {
        c if c == CCREQ_SERVER_INFO => {
            let s = r.read_string();
            let _ = s == b"QUAKE";
        }
        c if c == CCREQ_PLAYER_INFO => {
            let _player = r.read_byte();
        }
        c if c == CCREQ_RULE_INFO => {
            let _prev = r.read_string();
        }
        c if c == CCREQ_RCON => {
            let _password = r.read_string();
            let _cmd = r.read_string();
        }
        c if c == CCREQ_CONNECT => {
            let _game = r.read_string();
            let _ver = r.read_byte();
            // proquake extension probe: badread downgrades mod to 0
            let mut modb = r.read_byte();
            if r.badread {
                modb = 0;
            }
            let _ = modb;
        }
        _ => {}
    }
    check(&r, cursize);
}

/// client side: the _Datagram_Connect response parse
fn connect_response(payload: &[u8]) {
    let cursize = payload.len() as i32;
    let mut r = MsgReader::begin(payload, cursize);
    r.read_long();
    let ret = r.read_byte();
    if ret == CCREP_REJECT {
        let _reason = r.read_string();
    } else if ret == CCREP_ACCEPT {
        let _port = r.read_long();
        // proquake trailer: each byte gated on readcount < cursize
        let _mod = if r.readcount < cursize {
            r.read_byte()
        } else {
            0
        };
        let _ver = if r.readcount < cursize {
            r.read_byte()
        } else {
            0
        };
        let _flags = if r.readcount < cursize {
            r.read_byte()
        } else {
            0
        };
    }
    check(&r, cursize);
}

/// slist side: CCREP_SERVER_INFO / CCREP_PLAYER_INFO / CCREP_RULE_INFO and
/// the master getserversResponse address list
fn slist_parse(payload: &[u8]) {
    let cursize = payload.len() as i32;
    let mut r = MsgReader::begin(payload, cursize);
    r.read_long();
    match r.read_byte() {
        c if c == CCREP_SERVER_INFO => {
            let _addr = r.read_string();
            let _name = r.read_string();
            let _map = r.read_string();
            let _users = r.read_byte();
            let _max = r.read_byte();
            let _proto = r.read_byte();
        }
        c if c == CCREP_PLAYER_INFO => {
            let _num = r.read_byte();
            let _name = r.read_string();
            let _colors = r.read_long();
            let _frags = r.read_long();
            let _time = r.read_long();
            let _addr = r.read_string();
        }
        c if c == CCREP_RULE_INFO => {
            let _name = r.read_string();
            let _value = r.read_string();
        }
        _ => {}
    }
    check(&r, cursize);

    // getserversResponse: '\\' 6-byte ipv4 / '/' 18-byte ipv6 entries until
    // a bad tag or byte shortage latches badread
    let mut r = MsgReader::begin(payload, cursize);
    r.read_long();
    let mut guard = 0;
    loop {
        let mut bad = false;
        match r.read_byte() {
            0x5c => {
                for _ in 0..6 {
                    r.read_byte();
                }
            }
            0x2f => {
                for _ in 0..18 {
                    r.read_byte();
                }
            }
            _ => bad = true,
        }
        if bad || r.badread {
            break;
        }
        guard += 1;
        assert!(guard <= payload.len()); // each entry consumes >= 1 byte
    }
    check(&r, cursize);
}

fuzz_target!(|data: &[u8]| {
    server_dispatch(data);
    connect_response(data);
    slist_parse(data);
});
