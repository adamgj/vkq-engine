//! Net message reader/writer fuzzer (Phase 5, ADR-019 gate 5 "per-protocol
//! net message reader"): the first bytes select the protocol flag set
//! (15/666/999 encodings x PRFL variants x PEXT2) and an op schedule; the
//! rest is the message payload. Asserts no panics, cursor monotonicity, the
//! badread invariant, and write->read roundtrips where the encoding is
//! canonical. The true C-vs-Rust differential is quake-ctest's
//! net_msg_differential (same design decision as the Phase 3/4 targets).

#![no_main]

use libfuzzer_sys::fuzz_target;
use quake_net::msg::{self, MsgReader};
use quake_net::protocol::*;
use quake_net::sizebuf::SizeBuf;

const FLAG_SETS: [u32; 8] = [
    0, // protocol 15/666 byte angles + 13.3 coords
    PRFL_SHORTANGLE,
    PRFL_FLOATANGLE,
    PRFL_24BITCOORD,
    PRFL_FLOATCOORD,
    PRFL_INT32COORD,
    PRFL_SHORTANGLE | PRFL_24BITCOORD,
    PRFL_FLOATANGLE | PRFL_FLOATCOORD,
];

fuzz_target!(|data: &[u8]| {
    if data.len() < 4 {
        return;
    }
    let flags = FLAG_SETS[(data[0] & 7) as usize];
    let pext2 = if data[1] & 1 != 0 {
        PEXT2_REPLACEMENTDELTAS
    } else {
        0
    };
    let schedule = data[2];
    let payload = &data[4..];

    // reader over arbitrary bytes: must never panic, cursor must only grow,
    // badread must latch
    let cursize = payload.len() as i32;
    let mut r = MsgReader::begin(payload, cursize);
    let mut prev_count = 0i32;
    let mut prev_bad = false;
    for i in 0..96u32 {
        match (schedule as u32).wrapping_add(i) % 13 {
            0 => {
                r.read_char();
            }
            1 => {
                r.read_byte();
            }
            2 => {
                r.read_short();
            }
            3 => {
                r.read_long();
            }
            4 => {
                r.read_uint64();
            }
            5 => {
                r.read_int64();
            }
            6 => {
                r.read_float();
            }
            7 => {
                r.read_double();
            }
            8 => {
                r.read_string();
            }
            9 => {
                r.read_coord(flags);
            }
            10 => {
                r.read_angle(flags);
            }
            11 => {
                r.read_angle16(flags);
            }
            _ => {
                r.read_entity(pext2);
            }
        }
        assert!(r.readcount >= prev_count, "cursor went backwards");
        assert!(!prev_bad || r.badread, "badread unlatched");
        prev_count = r.readcount;
        prev_bad = r.badread;
    }

    // canonical roundtrips: derive values from the payload and require
    // write->read identity for the encodings that are exact
    let mut sb = SizeBuf::alloc(4096);
    let mut vals_i32 = Vec::new();
    let mut vals_u64 = Vec::new();
    for chunk in payload.chunks_exact(8).take(32) {
        let v = u64::from_le_bytes(chunk.try_into().unwrap());
        // < 2^28: outside the ReadUInt64 masked-shift bug domain (COMPAT --
        // values needing >= 4 continuation bytes do not round-trip in C)
        vals_u64.push(v & 0x0fff_ffff);
        vals_i32.push(v as i32);
    }
    for &v in &vals_i32 {
        msg::write_long(&mut sb, v).unwrap();
    }
    for &v in &vals_u64 {
        msg::write_uint64(&mut sb, v).unwrap();
    }
    let n = vals_i32.len() as u32;
    for i in 0..n {
        let e = (i * 0x777) & 0xffffff;
        msg::write_entity(&mut sb, e, PEXT2_REPLACEMENTDELTAS).unwrap();
    }
    let cursize = sb.cursize;
    let mut r = MsgReader::begin(&sb.data, cursize);
    for &v in &vals_i32 {
        assert_eq!(r.read_long(), v, "long roundtrip");
    }
    for &v in &vals_u64 {
        assert_eq!(r.read_uint64(), v, "u64 roundtrip");
    }
    for i in 0..n {
        // the pext2 escape is exact up to 0x7fffff; these stay far below
        let e = (i * 0x777) & 0xffffff;
        assert_eq!(
            r.read_entity(PEXT2_REPLACEMENTDELTAS),
            e,
            "entity roundtrip"
        );
    }
    assert!(!r.badread, "roundtrip underran");
});
