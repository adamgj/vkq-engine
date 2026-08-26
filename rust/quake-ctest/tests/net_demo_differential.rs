//! Differential test: `quake_net::demo::parse_forcetrack` vs the exact C
//! idiom CL_PlayDemo_f used (`fscanf ("%i")` + explicit `fgetc == '\n'`),
//! run through the platform libc by the `ctest_demo_forcetrack_oracle` stub.
//! Phase 5 M4 review follow-up: the parser's fidelity is proven against
//! libc, not against the implementer's reading of it.
//!
//! Scope notes: this compares the PARSER over a given byte buffer; the
//! engine-level difference that the flipped cl_demo.c reads a bounded
//! 64-byte chunk (COMPAT-noted there) is outside the parser core. Inputs
//! whose leading integer literal overflows `long` are excluded: that is UB
//! in C (fscanf %i out of range) and the port only approximates it (see
//! demo.rs). NUL bytes are excluded too -- the oracle feeds libc through a
//! FILE while the engine hands the parser a raw chunk, and no demo header
//! contains NULs before the newline.

use core::ffi::{c_char, c_int};

use quake_ctest as _;
use quake_net::demo;

extern "C" {
    fn ctest_demo_forcetrack_oracle(
        bytes: *const c_char,
        len: c_int,
        track: *mut c_int,
        consumed: *mut c_int,
    ) -> c_int;
}

fn oracle(bytes: &[u8]) -> Option<(i32, usize)> {
    let mut track: c_int = 0;
    let mut consumed: c_int = 0;
    // SAFETY: the oracle only reads `bytes[..len]` and writes the two outs
    let ok = unsafe {
        ctest_demo_forcetrack_oracle(
            bytes.as_ptr().cast::<c_char>(),
            bytes.len() as c_int,
            &mut track,
            &mut consumed,
        )
    };
    assert!(ok >= 0, "oracle tmpfile failure");
    if ok == 1 {
        Some((track, consumed as usize))
    } else {
        None
    }
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

fn check(bytes: &[u8]) {
    assert_eq!(
        demo::parse_forcetrack(bytes),
        oracle(bytes),
        "input {:?}",
        String::from_utf8_lossy(bytes)
    );
}

#[test]
fn forcetrack_parse_matches_libc() {
    // every engine-written form plus the corner grammar
    for t in [
        -1i64,
        0,
        1,
        2,
        11,
        255,
        65535,
        i32::MAX as i64,
        i32::MIN as i64,
    ] {
        check(format!("{t}\n").as_bytes());
        check(format!("  \t{t}\n").as_bytes());
        check(format!("+{t}\n").as_bytes());
    }
    for s in [
        "0x10\n",
        "0X1f\n",
        "0x\n",
        "010\n",
        "08\n",
        "0\n",
        "-0\n",
        "+0\n",
        "0xg\n",
        "-\n",
        "+\n",
        " \n",
        "\n",
        "",
        "5",
        "5 \n",
        "5x\n",
        "5\nrest",
        "\t\r\x0b\x0c 7\n",
        "--5\n",
        "+-5\n",
        "0x0\n",
        "2147483647\n",
        "-2147483648\n",
    ] {
        check(s.as_bytes());
    }

    // randomized sweep over the grammar's alphabet (no NULs, and reject
    // inputs whose literal would overflow `long` -- C UB, approximated only)
    let mut rng = Rng(0xDEC0DE5EED);
    let alphabet: &[u8] = b" \t\n\r+-0123456789abcdefxXgh.";
    for _ in 0..20000 {
        let n = (rng.next() % 14) as usize;
        let bytes: Vec<u8> = (0..n)
            .map(|_| alphabet[(rng.next() % alphabet.len() as u64) as usize])
            .collect();
        // at most 13 chars: cannot overflow a 64-bit long's decimal/hex range
        check(&bytes);
    }
}
