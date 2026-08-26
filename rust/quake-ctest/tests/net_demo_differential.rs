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
//! contains NULs before the newline. Finally, a `0x` prefix with no hex
//! digit after it has no single libc answer (see `glibc_hex_prefix_quirk`),
//! so those inputs are pinned against the port's own contract instead of
//! against the host's libc.

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

/// COMPAT: `%i` on a `0x`/`0X` prefix that no hex digit follows is a place
/// where the C libcs genuinely disagree, so the C engine itself behaves
/// differently per platform and no port can match them all.
///
/// C99 7.21.6.2p9 lets scanf push back at most one character, which macOS
/// libc and MSVC honour: they consume the `0`, push back the `x`, convert
/// 0, and cl_demo.c's explicit `fgetc () == '\n'` then sees the `x` and
/// rejects the demo. glibc instead swallows the whole `0x` and converts 0,
/// so on Linux a header line of exactly `"0x\n"` is ACCEPTED as track 0.
/// `demo::parse_forcetrack` implements the one-character-pushback reading;
/// on Linux that turns a `"0x\n"` header from accepted into
/// "not a demo file". Only hand-authored malformed headers can reach it --
/// the engine's own writer emits plain decimal -- so the divergence is
/// accepted rather than made platform-conditional.
fn glibc_hex_prefix_quirk(bytes: &[u8]) -> bool {
    let mut i = 0;
    while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r') {
        i += 1;
    }
    if matches!(bytes.get(i), Some(b'-') | Some(b'+')) {
        i += 1;
    }
    bytes.get(i) == Some(&b'0')
        && matches!(bytes.get(i + 1), Some(b'x') | Some(b'X'))
        && !bytes.get(i + 2).is_some_and(u8::is_ascii_hexdigit)
}

fn check(bytes: &[u8]) {
    if glibc_hex_prefix_quirk(bytes) {
        return;
    }
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
    // the libc-divergent class, pinned against the port's contract (the
    // one-character-pushback reading) rather than the host libc
    for s in ["0x\n", "0X\n", "  +0x\n", "-0X\n", "0xg\n", "0x"] {
        assert!(glibc_hex_prefix_quirk(s.as_bytes()));
        assert_eq!(demo::parse_forcetrack(s.as_bytes()), None, "input {s:?}");
    }

    for s in [
        "0x10\n",
        "0X1f\n",
        "010\n",
        "08\n",
        "0\n",
        "-0\n",
        "+0\n",
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
