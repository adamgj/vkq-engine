//! C numeric string conversions the net layer depends on, reproduced
//! exactly (Phase 5 M10 review fix).
//!
//! These live in one place because the semantics are subtle, platform
//! dependent, and observable: `net_main.c`'s `maxplayers`/`port` handlers
//! and `net_udp.c`'s `PartialIPAddress` all reach them, and the port used
//! to saturate where C truncates.

use core::ffi::c_long;

/// C's `atoi`, i.e. `(int) strtol (s, NULL, 10)`.
///
/// The subtlety is the two-stage out-of-range behavior:
/// `strtol` saturates at `LONG_MAX`/`LONG_MIN` and then the `(int)` cast
/// **truncates** to 32 bits. Saturating at `i32::MAX` instead is wrong for
/// every value above `INT_MAX` that is still within `long` -- e.g.
/// `"4294967300"` is 4 in C but would be `INT_MAX` when saturating, which
/// turns `maxplayers 4294967300` from "4 players" into "server maximum".
///
/// `long` is 64-bit on unix (LP64) and 32-bit on Windows (LLP64), so the
/// saturation point is genuinely per-platform; `c_long` carries that.
pub fn c_atoi(s: &[u8]) -> i32 {
    // the accumulator is a C `long`, exactly as in strtol -- keeping it in
    // `c_long` rather than widening to i64 means the saturation point moves
    // with the platform on its own, with no width-dependent conversions
    let mut i = 0;
    // strtol skips isspace()
    while i < s.len() && (s[i] == b' ' || (0x09..=0x0d).contains(&s[i])) {
        i += 1;
    }
    let mut neg = false;
    if i < s.len() && (s[i] == b'+' || s[i] == b'-') {
        neg = s[i] == b'-';
        i += 1;
    }

    let mut v: c_long = 0;
    let mut overflow = false;
    while i < s.len() && s[i].is_ascii_digit() {
        let d = c_long::from(s[i] - b'0');
        if !overflow && v > (c_long::MAX - d) / 10 {
            overflow = true;
        }
        if !overflow {
            v = v * 10 + d;
        }
        i += 1;
    }

    let wide = if overflow {
        if neg {
            c_long::MIN
        } else {
            c_long::MAX
        }
    } else if neg {
        -v
    } else {
        v
    };

    // the (int) cast: truncation, not saturation
    wide as i32
}

#[cfg(test)]
mod tests {
    use super::c_atoi;

    #[test]
    fn truncates_rather_than_saturating() {
        // the cases the pre-review saturating implementation got wrong
        assert_eq!(c_atoi(b"4294967296"), 0); // 2^32
        assert_eq!(c_atoi(b"4294967300"), 4);
        assert_eq!(c_atoi(b"2147483648"), i32::MIN);
    }

    #[test]
    fn prefix_and_sign_handling() {
        assert_eq!(c_atoi(b"  \t-42abc"), -42);
        assert_eq!(c_atoi(b"+7"), 7);
        assert_eq!(c_atoi(b""), 0);
        assert_eq!(c_atoi(b"abc"), 0);
        assert_eq!(c_atoi(b"-0"), 0);
    }
}
