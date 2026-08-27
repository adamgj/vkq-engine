//! Differential test: `quake_net::cnum::c_atoi` vs the platform libc's
//! `atoi`. Phase 5 M10 (review fix).
//!
//! `atoi` is `(int) strtol (s, NULL, 10)`: it saturates at `LONG_MAX` and
//! then *truncates* to `int`. Both stages matter and the first is
//! platform-dependent (`long` is 64-bit on unix, 32-bit on Windows), so
//! this is pinned against the real libc on every CI OS rather than against
//! a reading of the standard. Reached in-engine by `maxplayers`, `port`
//! and `PartialIPAddress`'s `:port`.

use core::ffi::{c_char, c_int};

use quake_ctest as _; // links the c_ref archive + stub globals
use quake_net::cnum::c_atoi;

extern "C" {
    fn ctest_atoi(s: *const c_char) -> c_int;
}

fn libc_atoi(s: &str) -> c_int {
    let mut b = s.as_bytes().to_vec();
    b.push(0);
    // SAFETY: b is NUL-terminated; atoi has no state
    unsafe { ctest_atoi(b.as_ptr().cast()) }
}

fn check(s: &str) {
    assert_eq!(libc_atoi(s), c_atoi(s.as_bytes()), "input {s:?}");
}

#[test]
fn matches_libc_on_the_engine_reachable_grammar() {
    for s in [
        "",
        "0",
        "1",
        "16",
        "-1",
        "+7",
        "  42",
        "\t\n 42",
        "42abc",
        "abc",
        "-0",
        "007",
        "2147483647",
        "2147483648",
        "-2147483648",
        "-2147483649",
        // above INT_MAX but inside long on LP64: truncation, not saturation
        "4294967295",
        "4294967296",
        "4294967300",
        "8589934592",
        // at and beyond LONG_MAX (LP64) / far beyond it on LLP64
        "9223372036854775807",
        "9223372036854775808",
        "99999999999999999999",
        "-9223372036854775808",
        "-9223372036854775809",
        "-99999999999999999999",
        // the value the review cited for `maxplayers`
        "99999999999",
    ] {
        check(s);
    }
}

#[test]
fn matches_libc_over_a_randomized_sweep() {
    let mut x: u64 = 0x2545_F491_4F6C_DD1D;
    for _ in 0..20_000 {
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        let v = x.wrapping_mul(0x2545_F491_4F6C_DD1D);
        // spread across the interesting magnitudes and both signs
        let digits = (v % 21) as u32 + 1;
        let mag = v % 10u64.saturating_pow(digits.min(19));
        let s = if v & 1 == 0 {
            format!("{mag}")
        } else {
            format!("-{mag}")
        };
        check(&s);
    }
}
